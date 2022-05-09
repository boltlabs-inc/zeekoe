use async_trait::async_trait;
use futures::stream::StreamExt;
use sqlx::SqlitePool;
use std::any::Any;
use thiserror::Error;

use zkabacus_crypto::{
    customer::{ClosingMessage, Inactive},
    CustomerBalance, MerchantBalance,
};

use tezedge::crypto::ToBase58Check;

use crate::{
    customer::ChannelName,
    escrow::types::{ContractDetails, ContractId, TezosPublicKey},
};

mod state;
use self::state::zkchannels_state::ZkChannelState;

pub use super::connect_sqlite;
use crate::{database::ClosingBalances, transport::ZkChannelAddress};
pub use state::{zkchannels_state, State, StateName, UnexpectedState};

type Result<T> = std::result::Result<T, Error>;

/// An error when accessing the customer database.
#[derive(Debug, Error)]
pub enum Error {
    /// The state of the channel was not what was expected.
    #[error(transparent)]
    UnexpectedState(#[from] UnexpectedState),
    /// Channel could not be transitioned to pending close.
    #[error("Channel closure could not be initiated - it is likely not in a closeable state")]
    CloseFailure,
    /// An underlying error occurred in the database.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// An underlying error occurred while migrating the database.
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
    /// A channel which was expected to exist in the database did not exist.
    #[error("There is no channel by the name of \"{0}\"")]
    NoSuchChannel(ChannelName),
    /// A channel which was expected *not* to exist in the database *did* exist.
    #[error("There is already a channel by the name of \"{0}\"")]
    ChannelExists(ChannelName),
    /// A channel balance update was invalid.
    #[error("Failed to update channel balance to invalid set (merchant: {0:?}, customer: {1:?})")]
    InvalidBalanceUpdate(MerchantBalance, Option<CustomerBalance>),
    /// A channel contained incomplete contract details.
    #[error("Error retrieving contract details for \"{0}\": incomplete details")]
    InvalidContractDetails(ChannelName),
    /// A channel already holds contract details.
    #[error("The channel \"{0}\" already has contract details set")]
    ContractDetailsExist(ChannelName),
}

/// The contents of a row of the database for a particular channel.
#[non_exhaustive]
pub struct ChannelDetails {
    pub label: ChannelName,
    pub state: State,
    pub merchant_deposit: MerchantBalance,
    pub customer_deposit: CustomerBalance,
    pub address: ZkChannelAddress,
    pub closing_balances: ClosingBalances,
    pub contract_details: ContractDetails,
}

/// Extension trait augmenting the customer database [`QueryCustomer`] with extra methods.
///
/// These are implemented automatically for any database handle which implements
/// [`QueryCustomer`]; when passing a trait object, use the [`QueryCustomer`] trait instead of
/// this one.
#[async_trait]
pub trait QueryCustomerExt {
    /// Given a channel's unique name, mutate its state in the database using a provided closure
    /// if the stored state matches type `S`.
    ///
    /// The return type for this function can be interpreted as follows:
    /// - A successful run returns `Ok(Ok(T))`, where `T` is returned by the closure. It holds any
    ///   values that need to be used in the course of the protocol, like a message derived from
    ///   the state change.
    /// - A run where the closure fails to execute returns `Ok(Err(E))`, where `E` is the error
    ///   type returned by the closure.
    /// - A run where something other than the closure fails to execute returns `Err(Error)`. This
    ///   is either an [`UnexpectedState`] error, where the stored state does not match the
    ///   expected value, or a database failure.
    ///
    /// **Important:** The given closure should be idempotent on the state of the world.
    /// In particular, the closure **should not result in communication with the merchant**.
    async fn with_channel_state<
        'a,
        S: ZkChannelState + Send + 'static,
        F: FnOnce(S::ZkAbacusState) -> std::result::Result<(State, T), E> + Send + 'a,
        T: Send + 'static,
        E: Send + 'static,
    >(
        &'a self,
        channel_name: &ChannelName,
        expected_state: S,
        with_zkabacus_state: F,
    ) -> Result<std::result::Result<T, E>>;

    /// Given a channel's unique name, mutate its state in the database using a provided closure,
    /// that is given the current state and must convert it to [`State::PendingClose`] or
    /// [`State::PendingExpiry`].
    ///
    /// The return type can be interpreted as follows:
    /// - A successful run returns `Ok(Ok([`ClosingMessage`]))`. This indicates that the database
    ///   is correctly updated.
    /// - An `Ok(Err(e))` indicates an error raised by the closure
    /// - An `Err(e)` indicates an error raised outside the closure. This could be a database
    ///   failure or an incorrect state error (e.g. the closure returns a [`State`] variant other
    ///   than [`State::PendingClose`] or [`State::PendingExpiry`]).
    ///
    /// **Important:** The given closure should be idempotent on the state of the world.
    /// In particular, the closure **should not result in communication with the merchant**.
    async fn with_closeable_channel<'a, E: Send + 'static>(
        &self,
        channel_name: &ChannelName,
        close_zkabacus_state: impl FnOnce(State) -> std::result::Result<(State, ClosingMessage), E>
            + Send
            + 'a,
    ) -> Result<std::result::Result<ClosingMessage, E>>;
}

/// Trait-object safe version of [`QueryCustomer`]: use this type in trait objects and implement it
/// for database backends.
#[async_trait]
pub trait QueryCustomer: Send + Sync {
    /// Perform all the DB migrations defined in src/database/migrations/customer/*.sql
    async fn migrate(&self) -> Result<()>;

    /// Insert a newly initialized [`zkabacus_crypto::customer::Requested`] channel into the
    /// customer database, associated with a unique name and [`ZkChannelAddress`].
    ///
    /// If the [`zkabacus_crypto::customer::Requested`] could not be inserted, it is returned along
    /// with the error that prevented its insertion.
    async fn new_channel(
        &self,
        channel_name: &ChannelName,
        address: &ZkChannelAddress,
        inactive: Inactive,
        contract_details: &ContractDetails,
        zkabacus_config: &zkabacus_crypto::customer::Config,
    ) -> std::result::Result<(), (Inactive, Error)>;

    /// Get a channel's [`zkabacus_crypto::customer::Config`].
    async fn channel_zkabacus_config(
        &self,
        channel_name: &ChannelName,
    ) -> Result<zkabacus_crypto::customer::Config>;

    /// Get the address of a given channel.
    async fn channel_address(&self, channel_name: &ChannelName) -> Result<ZkChannelAddress>;

    /// Get the closing balances of a given channel.
    async fn closing_balances(&self, channel_name: &ChannelName) -> Result<ClosingBalances>;

    /// Update the closing balances for a given channel.
    ///
    /// This should only be called once the balances are finalized on chain and maintains the
    /// following invariants:
    /// - The customer balance can be set at most once.
    /// - The merchant balance can only be increased.
    /// If either of these invariants are violated, will raise [`Error::InvalidBalanceUpdate`].
    async fn update_closing_balances(
        &self,
        channel_name: &ChannelName,
        merchant_balance: MerchantBalance,
        customer_balance: Option<CustomerBalance>,
    ) -> Result<()>;

    /// Get the merchant's Tezos key and details about the originated Tezos contract if it exists.
    async fn contract_details(&self, channel_name: &ChannelName) -> Result<ContractDetails>;

    /// Set contract information for a given channel. Will fail if the contract information has
    /// previously been set.
    async fn initialize_contract_details(
        &self,
        channel_name: &ChannelName,
        contract_id: &ContractId,
    ) -> Result<()>;

    /// Rename an existing channel from a given name to a new one.
    async fn rename_channel(
        &self,
        channel_name: &ChannelName,
        new_label: &ChannelName,
    ) -> Result<()>;

    /// Assign a new [`ZkChannelAddress`] to an existing channel.
    async fn readdress_channel(
        &self,
        label: &ChannelName,
        new_address: &ZkChannelAddress,
    ) -> Result<()>;

    /// Get complete [`ChannelDetails`] for _every_ channel, including the current status and
    /// balances, the zkAbacus state, the merchant's address for initiating sub-protocols,
    /// details about the originated contract, and any money that has been paid out.
    async fn get_channels(&self) -> Result<Vec<ChannelDetails>>;

    /// Get complete [`ChannelDetails`] for the given channel, including the current status and
    /// balances, the zkAbacus state, the merchant's address for initiating sub-protocols,
    /// details about the originated contract, and any money that has been paid out.
    async fn get_channel(&self, channel_name: &ChannelName) -> Result<ChannelDetails>;

    /// **Don't call this function directly:** instead call
    /// [`QueryCustomerExt::with_channel_state`] or [`QueryCustomerExt::with_closeable_channel`].  This
    /// method retrieves the current state from the database, retrieves an updated state by executing
    /// `with_state` on the current state, and updates the database.  This method uses `Box<dyn Any +
    /// Send>` to avoid the use of generic parameters,
    /// which is what allows the trait to be object safe.
    ///
    /// # Panics
    ///
    /// The corresponding method [`QueryCustomerExt::with_channel_state`] and
    /// [`QueryCustomerExt::with_closeable_channel`] will panic if the boxed [`Any`] types returned by this
    /// method do not match that function's type parameters.  It is expected that any implementation of
    /// this function merely forwards these values to the returned `Result<Box<dyn Any>, Box<dyn
    /// Any>>`.
    async fn with_channel_state_erased<'a>(
        &'a self,
        channel_name: &ChannelName,
        with_state: Box<
            dyn for<'s> FnOnce(
                    State,
                ) -> std::result::Result<
                    (State, Box<dyn Any + Send>),
                    Box<dyn Any + Send>,
                > + Send
                + 'a,
        >,
    ) -> Result<std::result::Result<Box<dyn Any>, Box<dyn Any>>>;
}

#[async_trait]
impl QueryCustomer for SqlitePool {
    async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("src/database/migrations/customer")
            .run(self)
            .await?;
        Ok(())
    }

    async fn new_channel(
        &self,
        channel_name: &ChannelName,
        address: &ZkChannelAddress,
        inactive: Inactive,
        contract_details: &ContractDetails,
        zkabacus_config: &zkabacus_crypto::customer::Config,
    ) -> std::result::Result<(), (Inactive, Error)> {
        let merchant_deposit = inactive.merchant_balance();
        let customer_deposit = inactive.customer_balance();
        let state = State::Inactive(inactive);
        (|| async {
            let mut transaction = self.begin().await?;

            // Determine if the channel already exists
            let already_exists = sqlx::query!(
                "SELECT label FROM customer_channels WHERE label = ?",
                channel_name
            )
            .fetch(&mut transaction)
            .next()
            .await
            .transpose()?
            .is_some();

            // Return an error if it does exist
            if already_exists {
                return Err(Error::ChannelExists(channel_name.clone()));
            }

            // Return an error if contract details are already originated
            if contract_details.contract_id.is_some() {
                return Err(Error::InvalidContractDetails(channel_name.clone()));
            }

            let default_balances = ClosingBalances::default();
            let merchant_tezos_public_key_string =
                contract_details.merchant_tezos_public_key.to_base58check();
            let inserted_config = sqlx::query!(
                r#"
                INSERT INTO configs (data)
                VALUES (?)
                RETURNING id AS "id: i32"
                "#,
                zkabacus_config
            )
            .fetch_one(&mut transaction)
            .await?;

            let result = sqlx::query!(
                "INSERT INTO customer_channels (
                    label,
                    address,
                    merchant_deposit,
                    customer_deposit,
                    state,
                    closing_balances,
                    merchant_tezos_public_key,
                    contract_id,
                    config_id
                )
                VALUES (?, ?, ?, ?, ?, ?, ?, NULL, ?)
            ",
                channel_name,
                address,
                merchant_deposit,
                customer_deposit,
                state,
                default_balances,
                merchant_tezos_public_key_string,
                inserted_config.id
            )
            .execute(&mut transaction)
            .await
            .map(|_| ());

            transaction.commit().await?;

            Ok(result?)
        })()
        .await
        .map_err(|e| {
            (
                zkchannels_state::Inactive::zkabacus_state(state).unwrap(),
                e,
            )
        })
    }

    async fn channel_zkabacus_config(
        &self,
        channel_name: &ChannelName,
    ) -> Result<zkabacus_crypto::customer::Config> {
        Ok(sqlx::query!(
            r#"
            SELECT data AS "data: zkabacus_crypto::customer::Config"
            FROM configs
            INNER JOIN customer_channels ON configs.id = customer_channels.config_id
            WHERE customer_channels.label = ?
            LIMIT 1
            "#,
            channel_name
        )
        .fetch_one(self)
        .await?
        .data)
    }

    async fn channel_address(&self, channel_name: &ChannelName) -> Result<ZkChannelAddress> {
        Ok(sqlx::query!(
            r#"
            SELECT address AS "address: ZkChannelAddress"
            FROM customer_channels
            WHERE label = ?
            "#,
            channel_name,
        )
        .fetch(self)
        .next()
        .await
        .ok_or_else(|| Error::NoSuchChannel(channel_name.clone()))?
        .map(|record| record.address)?)
    }

    async fn closing_balances(&self, channel_name: &ChannelName) -> Result<ClosingBalances> {
        Ok(sqlx::query!(
            r#"
            SELECT closing_balances AS "closing_balances: ClosingBalances"
            FROM customer_channels
            WHERE label = ?
            "#,
            channel_name,
        )
        .fetch(self)
        .next()
        .await
        .ok_or_else(|| Error::NoSuchChannel(channel_name.clone()))?
        .map(|record| record.closing_balances)?)
    }

    async fn update_closing_balances(
        &self,
        channel_name: &ChannelName,
        merchant_balance: MerchantBalance,
        customer_balance: Option<CustomerBalance>,
    ) -> Result<()> {
        let mut transaction = self.begin().await?;

        // Ensure that the channel name exists
        // TODO: find a way to do this modularly with `closing_balances()`?
        let closing_balances = sqlx::query!(
            r#"
            SELECT closing_balances AS "closing_balances: ClosingBalances"
            FROM customer_channels
            WHERE label = ?"#,
            channel_name,
        )
        .fetch(&mut transaction)
        .next()
        .await
        .ok_or_else(|| Error::NoSuchChannel(channel_name.clone()))?
        .map(|record| record.closing_balances)?;

        // Make sure we're not decreasing merchant balance.
        if let Some(original_balance) = closing_balances.merchant_balance {
            if original_balance > merchant_balance {
                return Err(Error::InvalidBalanceUpdate(
                    merchant_balance,
                    customer_balance,
                ));
            }
        }

        // Make sure we don't update customer balance more than once.
        match (closing_balances.customer_balance, customer_balance) {
            (Some(_), Some(_)) | (Some(_), None) => {
                return Err(Error::InvalidBalanceUpdate(
                    merchant_balance,
                    customer_balance,
                ))
            }
            _ => (),
        }

        // If everything was ok, set the new balances.
        let updated_closing_balances = ClosingBalances {
            merchant_balance: Some(merchant_balance),
            customer_balance,
        };

        // Update the db with the new balances.
        sqlx::query!(
            "UPDATE customer_channels SET closing_balances = ? WHERE label = ?",
            updated_closing_balances,
            channel_name,
        )
        .execute(&mut transaction)
        .await?;

        transaction.commit().await?;

        Ok(())
    }

    async fn contract_details(&self, channel_name: &ChannelName) -> Result<ContractDetails> {
        let record = sqlx::query!(
            r#"
            SELECT 
                contract_id AS "contract_id: ContractId",
                merchant_tezos_public_key AS "merchant_tezos_public_key: String"
            FROM customer_channels
            WHERE label = ?
            "#,
            channel_name,
        )
        .fetch(self)
        .next()
        .await
        .ok_or_else(|| Error::NoSuchChannel(channel_name.clone()))??;

        // Try to parse the Tezos key
        let merchant_tezos_public_key =
            TezosPublicKey::from_base58check(&record.merchant_tezos_public_key)
                .map_err(|_| Error::InvalidContractDetails(channel_name.clone()))?;

        Ok(ContractDetails {
            merchant_tezos_public_key,
            contract_id: record.contract_id,
        })
    }

    async fn initialize_contract_details(
        &self,
        channel_name: &ChannelName,
        contract_id: &ContractId,
    ) -> Result<()> {
        let mut transaction = self.begin().await?;

        // Ensure that channel exists and does not already have contract details.
        // TODO: find a way to do this modularly with `contract_details()`
        let record = sqlx::query!(
            r#"
            SELECT
                contract_id AS "contract_id: Option<ContractId>"
            FROM customer_channels
            WHERE label = ?
            "#,
            channel_name,
        )
        .fetch(&mut transaction)
        .next()
        .await
        .ok_or_else(|| Error::NoSuchChannel(channel_name.clone()))??;

        if record.contract_id.is_some() {
            return Err(Error::ContractDetailsExist(channel_name.clone()));
        }

        // Update channel with new details.
        sqlx::query!(
            "UPDATE customer_channels SET contract_id = ? WHERE label = ?",
            contract_id,
            channel_name,
        )
        .execute(&mut transaction)
        .await?;

        transaction.commit().await?;

        Ok(())
    }

    async fn rename_channel(
        &self,
        channel_name: &ChannelName,
        new_channel_name: &ChannelName,
    ) -> Result<()> {
        let mut transaction = self.begin().await?;

        // Ensure that the old channel name exists
        let old_exists = sqlx::query!(
            "SELECT label FROM customer_channels WHERE label = ?",
            channel_name
        )
        .fetch(&mut transaction)
        .next()
        .await
        .is_some();

        if !old_exists {
            return Err(Error::NoSuchChannel(channel_name.clone()));
        }

        // Ensure that the new channel name *does not* exist
        let new_does_not_exist = sqlx::query!(
            "SELECT label FROM customer_channels WHERE label = ?",
            new_channel_name
        )
        .fetch(&mut transaction)
        .next()
        .await
        .is_none();

        if !new_does_not_exist {
            return Err(Error::ChannelExists(new_channel_name.clone()));
        }

        sqlx::query!(
            "UPDATE customer_channels SET label = ? WHERE label = ?",
            new_channel_name,
            channel_name,
        )
        .execute(self)
        .await?;

        transaction.commit().await?;

        Ok(())
    }

    async fn readdress_channel(
        &self,
        channel_name: &ChannelName,
        new_address: &ZkChannelAddress,
    ) -> Result<()> {
        let rows_affected = sqlx::query!(
            "UPDATE customer_channels SET address = ? WHERE label = ?",
            new_address,
            channel_name,
        )
        .execute(self)
        .await?
        .rows_affected();

        // If the rows affected is 1, that means we found the channel to readdress
        if rows_affected == 1 {
            Ok(())
        } else {
            Err(Error::NoSuchChannel(channel_name.clone()))
        }
    }

    async fn get_channels(&self) -> Result<Vec<ChannelDetails>> {
        sqlx::query!(
            r#"
            SELECT
                label AS "label: ChannelName",
                state AS "state: State",
                address AS "address: ZkChannelAddress",
                customer_deposit AS "customer_deposit: CustomerBalance",
                merchant_deposit AS "merchant_deposit: MerchantBalance",
                closing_balances AS "closing_balances: ClosingBalances",
                merchant_tezos_public_key AS "merchant_tezos_public_key: String",
                contract_id AS "contract_id: ContractId"
            FROM customer_channels
            "#
        )
        .fetch_all(self)
        .await?
        .into_iter()
        .map(|r| -> Result<ChannelDetails> {
            let label_copy = r.label.clone();
            Ok(ChannelDetails {
                label: r.label,
                state: r.state,
                address: r.address,
                customer_deposit: r.customer_deposit,
                merchant_deposit: r.merchant_deposit,
                closing_balances: r.closing_balances,
                contract_details: ContractDetails {
                    merchant_tezos_public_key: TezosPublicKey::from_base58check(
                        &r.merchant_tezos_public_key,
                    )
                    .map_err(|_| Error::InvalidContractDetails(label_copy))?,
                    contract_id: r.contract_id,
                },
            })
        })
        .collect()
    }

    async fn get_channel(&self, channel_name: &ChannelName) -> Result<ChannelDetails> {
        sqlx::query!(
            r#"
            SELECT
                state AS "state: State",
                address AS "address: ZkChannelAddress",
                customer_deposit AS "customer_deposit: CustomerBalance",
                merchant_deposit AS "merchant_deposit: MerchantBalance",
                closing_balances AS "closing_balances: ClosingBalances",
                merchant_tezos_public_key AS "merchant_tezos_public_key: String",
                contract_id AS "contract_id: ContractId"
            FROM customer_channels 
            WHERE label = ?
            "#,
            channel_name,
        )
        .fetch(self)
        .next()
        .await
        .ok_or_else(|| Error::NoSuchChannel(channel_name.clone()))?
        .map(|r| -> Result<ChannelDetails> {
            Ok(ChannelDetails {
                label: channel_name.clone(),
                state: r.state,
                address: r.address,
                customer_deposit: r.customer_deposit,
                merchant_deposit: r.merchant_deposit,
                closing_balances: r.closing_balances,
                contract_details: ContractDetails {
                    merchant_tezos_public_key: TezosPublicKey::from_base58check(
                        &r.merchant_tezos_public_key,
                    )
                    .map_err(|_| Error::InvalidContractDetails(channel_name.clone()))?,
                    contract_id: r.contract_id,
                },
            })
        })?
    }

    async fn with_channel_state_erased<'a>(
        &'a self,
        channel_name: &ChannelName,
        with_state: Box<
            dyn for<'s> FnOnce(
                    State,
                ) -> std::result::Result<
                    (State, Box<dyn Any + Send>),
                    Box<dyn Any + Send>,
                > + Send
                + 'a,
        >,
    ) -> Result<std::result::Result<Box<dyn Any>, Box<dyn Any>>> {
        let mut transaction = self.begin().await?;

        // Retrieve the state so that we can modify it
        let state: State = sqlx::query!(
            r#"SELECT state AS "state: State" FROM customer_channels WHERE label = ?"#,
            channel_name,
        )
        .fetch(&mut transaction)
        .next()
        .await
        .ok_or_else(|| Error::NoSuchChannel(channel_name.clone()))??
        .state;

        // Perform the operation with the state fetched from the database
        match with_state(state) {
            Ok((state, output)) => {
                // Store the new state to the database
                sqlx::query!(
                    "UPDATE customer_channels SET state = ? WHERE label = ?",
                    state,
                    channel_name
                )
                .execute(&mut transaction)
                .await?;

                // Commit the transaction
                transaction.commit().await?;

                Ok(Ok(output))
            }
            Err(error) => Ok(Err(error)),
        }
    }
}

// Blanket implementation of [`QueryCustomerExt`] for all [`QueryCustomer`]
#[async_trait]
impl<Q: QueryCustomer + ?Sized> QueryCustomerExt for Q {
    async fn with_channel_state<
        'a,
        S: ZkChannelState + Send + 'static,
        F: FnOnce(S::ZkAbacusState) -> std::result::Result<(State, T), E> + Send + 'a,
        T: Send + 'static,
        E: Send + 'static,
    >(
        &'a self,
        channel_name: &ChannelName,
        _expected_state: S,
        with_zkabacus_state: F,
    ) -> Result<std::result::Result<T, E>> {
        let result = <Self as QueryCustomer>::with_channel_state_erased(
            self,
            channel_name,
            Box::new(
                // Extract the inner zkAbacus type from the state enum and make sure it matches
                |state| match S::zkabacus_state(state) {
                    Ok(zkabacus_state) => match with_zkabacus_state(zkabacus_state) {
                        Ok((state, t)) => Ok((state, Box::new(t))),
                        Err(e) => Err(Box::new(Ok::<E, UnexpectedState>(e))),
                    },
                    Err(unexpected_state) => {
                        Err(Box::new(Err::<E, UnexpectedState>(unexpected_state)))
                    }
                },
            ),
        )
        .await?;

        // Cast the result back to its true type
        match result {
            // Successful result
            Ok(t) => {
                let t: T = *t.downcast().unwrap();
                Ok(Ok(t))
            }
            // Error, which could be one of...
            Err(error_result) => {
                let error_result: std::result::Result<E, UnexpectedState> =
                    *error_result.downcast().unwrap();
                match error_result {
                    // Error returned by the closure
                    Ok(e) => Ok(Err(e)),
                    // Error returned because the state didn't match the one in the database.
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }

    async fn with_closeable_channel<'a, E: Send + 'static>(
        &self,
        channel_name: &ChannelName,
        with_closeable_state: impl FnOnce(State) -> std::result::Result<(State, ClosingMessage), E>
            + Send
            + 'a,
    ) -> Result<std::result::Result<ClosingMessage, E>> {
        let result = <Self as QueryCustomer>::with_channel_state_erased(
            self,
            channel_name,
            Box::new(|state| match with_closeable_state(state) {
                Ok((state, t)) => {
                    // Only allow updates that result in the PendingClose or PendingExpiry status
                    match state {
                        State::PendingClose(_) | State::PendingExpiry(_) => {
                            Ok((state, Box::new(t)))
                        }
                        _ => Err(Box::new(Err::<E, Error>(Error::CloseFailure))),
                    }
                }
                // Closure function failed somehow
                Err(e) => Err(Box::new(Ok::<E, Error>(e))),
            }),
        )
        .await?;

        // Cast the result back to its true type
        match result {
            // Successful result: get the `ClosingMessage` out of the box.
            Ok(t) => Ok(Ok(*t.downcast().unwrap())),
            // Error, which could be one of...
            Err(error_result) => {
                let err: Result<E> = *error_result.downcast().unwrap();
                match err {
                    // Error returned by the closure
                    Ok(e) => Ok(Err(e)),
                    // Error returned because the closure didn't return a PendingClose status.
                    Err(e) => Err(e),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::SqlitePoolOptions;
    use rand::{rngs::StdRng, SeedableRng};
    use std::str::FromStr;

    use tezedge::OriginatedAddress;
    use zkabacus_crypto::{customer::*, merchant, *};

    async fn create_migrated_db() -> Result<SqlitePool> {
        let conn = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        conn.migrate().await?;
        Ok(conn)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_migrate() -> Result<()> {
        create_migrated_db().await?;
        Ok(())
    }

    async fn insert_channel(channel_name: &ChannelName, conn: &SqlitePool) -> Result<()> {
        // set up zkchannel details
        let mut rng = StdRng::from_entropy();
        let address = ZkChannelAddress::from_str("zkchannel://localhost").unwrap();

        // set up keys
        let merchant_config = merchant::Config::new(&mut rng);
        let (pk, rev_param, range_param) = merchant_config.extract_customer_config_parts();
        let zkabacus_config = Config::from_parts(pk, rev_param, range_param);

        // build a channel id
        let cid_m = MerchantRandomness::new(&mut rng);
        let cid_c = CustomerRandomness::new(&mut rng);
        let channel_id = ChannelId::new(
            cid_m,
            cid_c,
            zkabacus_config.merchant_public_key(),
            &[],
            &[],
        );

        // set up deposit info
        let merchant_balance = MerchantBalance::try_new(5).unwrap();
        let customer_balance = CustomerBalance::try_new(5).unwrap();
        let context = Context::new(b"here is some fake context");

        // simulate establish to get zkabacus objects
        let (requested, proof) = Requested::new(
            &mut rng,
            &zkabacus_config,
            channel_id,
            merchant_balance,
            customer_balance,
            &context,
        );

        let (closing_signature, _blinded_state) = merchant_config
            .initialize(
                &mut rng,
                &channel_id,
                customer_balance,
                merchant_balance,
                proof,
                &context,
            )
            .unwrap();

        let inactive = requested
            .complete(closing_signature, &zkabacus_config)
            .unwrap();
        let contract_details = ContractDetails {
            merchant_tezos_public_key: TezosPublicKey::from_base58check(
                "edpku5Ei6Dni4qwoJGqXJs13xHfyu4fhUg6zqZkFyiEh1mQhFD3iZE",
            )
            .unwrap(),
            contract_id: None,
        };

        conn.new_channel(
            channel_name,
            &address,
            inactive,
            &contract_details,
            &zkabacus_config,
        )
        .await
        .map_err(|(_, e)| e)?;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn insert_customer_channel() -> Result<()> {
        let conn = create_migrated_db().await?;
        let channel_name = ChannelName::new("test channel".to_string());
        insert_channel(&channel_name, &conn).await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn insert_contract_details() -> Result<()> {
        let conn = create_migrated_db().await?;
        let channel_name = ChannelName::new("test contract details channel".to_string());
        insert_channel(&channel_name, &conn).await?;

        // make sure contract details are not set initially
        if conn
            .contract_details(&channel_name)
            .await?
            .contract_id
            .is_some()
        {
            panic!("Contract details should not be set yet.")
        }

        // pick contract details
        let contract_id = ContractId::new(
            OriginatedAddress::from_base58check("KT1Mjjcb6tmSsLm7Cb3DSQszePjfchPM4Uxm").unwrap(),
        );

        // set contract details
        conn.initialize_contract_details(&channel_name, &contract_id)
            .await?;

        // make sure saved details match expected values
        let details = conn.contract_details(&channel_name).await?;
        match details.contract_id {
            Some(saved_id) => {
                assert!(saved_id == contract_id)
            }
            None => panic!("Contract details did not get set when they should"),
        }

        // make sure we cannot overwrite saved contact details
        match conn
            .initialize_contract_details(&channel_name, &contract_id)
            .await
        {
            Ok(()) => panic!("Allowed overwrite of contract details"),
            Err(super::Error::ContractDetailsExist(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }
}
