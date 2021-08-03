use {
    async_trait::async_trait, futures::stream::StreamExt, sqlx::SqlitePool, std::any::Any,
    thiserror::Error,
};

use zkabacus_crypto::{
    customer::{ClosingMessage, Inactive},
    CustomerBalance, MerchantBalance,
};

use crate::customer::{client::ZkChannelAddress, ChannelName};

mod state;
use self::state::{zkchannels_state::ZkChannelState, IsZkAbacusState, StateError};

pub use super::connect_sqlite;
pub use state::{zkchannels_state, ImpossibleState, State, StateName, UnexpectedState};

type Result<T> = std::result::Result<T, Error>;

/// An error when accessing the customer database.
#[derive(Debug, Error)]
pub enum Error {
    /// The state of the channel was not what was expected.
    #[error(transparent)]
    UnexpectedState(#[from] UnexpectedState),
    /// The state of the channel does not contain the requested data.
    #[error(transparent)]
    ImpossibleState(#[from] ImpossibleState),
    /// Channel could not be transitioned to pending close.
    #[error("Channel closure could not be initiated - it is likely not in a closeable state.")]
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
}

/// The contents of a row of the database for a particular channel.
#[non_exhaustive]
pub struct ChannelDetails {
    pub label: ChannelName,
    pub state: State,
    pub merchant_deposit: MerchantBalance,
    pub customer_deposit: CustomerBalance,
    pub address: ZkChannelAddress,
}

/// Extension trait augmenting the customer database [`QueryCustomer`] with extra methods.
///
/// These are implemented automatically for any database handle which implements
/// [`QueryCustomer`]; when passing a trait object, use the [`QueryCustomer`] trait instead of
/// this one.
#[async_trait]
pub trait QueryCustomerExt {
    /// Given a channel's unique name, mutate its state in the database using a provided closure,
    /// that is given the current state.
    ///
    /// The return type for this function can be interpreted as follows:
    /// - A successful run returns `Ok(Ok(T))`, where `T` is returned by the closure. It holds any
    ///   values that need to be used in the course of the protocol, like a message derived from
    ///   the state change.
    /// - A run where the closure fails to execute returns `Ok(Err(E))`, where `E` is the error
    ///   type returned by the closure.
    /// - A run where something other than the closure fails to execute returns `Err(Error)`. This
    ///   includes [`StateError`]s (e.g. if the current state does not match `expected_state_name`)
    ///   as well as database failures.
    ///
    /// **Important:** The given closure should be idempotent on the state of the world.
    /// In particular, the closure **should not result in communication with the merchant**.
    async fn with_channel_state<
        'a,
        S: ZkChannelState + Send,
        F: FnOnce(S::ZkAbacusState) -> std::result::Result<(State, T), E> + Send + 'a,
        T: Send + 'static,
        E: Send + 'static,
    >(
        &'a self,
        channel_name: &ChannelName,
        with_zkabacus_state: F,
    ) -> Result<std::result::Result<T, E>>;

    /*
       async fn with_channel_state<
           'a,
           S: ZkChannelState + Send,
           T: Send + 'static,
           E: Send + 'static,
       >(
           &'a self,
           channel_name: &ChannelName,
           with_zkabacus_state: impl for<'s> FnOnce(S::ZkAbacusState) -> std::result::Result<(State, T), E>
               + Send
               + 'a,
       ) -> Result<std::result::Result<T, E>>;
    */
    /// Given a channel's unique name, mutate its state in the database using a provided closure,
    /// that is given the current state and must convert it to [`State::PendingClose`].
    ///
    /// The return type can be interpreted as follows:
    /// - A successful run returns `Ok(Ok([`ClosingMessage`]))`. This indicates that the database
    ///   is correctly updated.
    /// - An `Ok(Err(e))` indicates an error raised by the closure
    /// - An `Err(e)` indicates an error raised outside the closure. This could be a database
    ///   failure or an incorrect state error (e.g. the closure returns a [`State`] variant other
    ///   than [`State::PendingClose`]).
    ///
    /// **Important:** The given closure should be idempotent on the state of the world.
    /// In particular, the closure **should not result in communication with the merchant**.
    async fn with_closeable_channel<'a, E: Send + 'static>(
        &self,
        channel_name: &ChannelName,
        close_zkabacus_state: impl for<'s> FnOnce(State) -> std::result::Result<(State, ClosingMessage), E>
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

    /// Insert a newly initialized [`Requested`] channel into the customer database, associated with
    /// a unique name and [`ZkChannelAddress`].
    ///
    /// If the [`Requested`] could not be inserted, it is returned along with the error that
    /// prevented its insertion.
    async fn new_channel(
        &self,
        channel_name: &ChannelName,
        address: &ZkChannelAddress,
        inactive: Inactive,
    ) -> std::result::Result<(), (Inactive, Error)>;

    /// Get the address of a given channel.
    async fn channel_address(&self, channel_name: &ChannelName) -> Result<ZkChannelAddress>;

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

    /// Get all the information about all the channels.
    async fn get_channels(&self) -> Result<Vec<ChannelDetails>>;

    /// **Don't call this function directly:** instead call [`QueryCustomer::with_channel_state`]
    /// or [`QueryCustomer::mark_closing_channel`].
    /// This method retrieves the current state from the database, retrieves an updated state by
    /// executing `with_state` on the current state, and updates the database.
    /// This method uses `Box<dyn Any + Send>` to avoid the use of generic parameters,
    /// which is what allows the trait to be object safe.
    ///
    /// # Panics
    ///
    /// The corresponding method [`QueryCustomer::with_channel_state`] and
    /// [`QueryCustomer::mark_closing_channel`] will panic if the boxed
    /// [`Any`] types returned by this method do not match that function's type parameters.
    /// It is expected that any implementation of this function merely forwards these values to
    /// the returned `Result<Box<dyn Any>, Box<dyn Any>>`.
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
    ) -> std::result::Result<(), (Inactive, Error)> {
        let merchant_deposit = *inactive.merchant_balance();
        let customer_deposit = *inactive.customer_balance();
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

            let result = sqlx::query!(
                "INSERT INTO customer_channels (
                    label,
                    address,
                    merchant_deposit,
                    customer_deposit,
                    state
                ) VALUES (?, ?, ?, ?, ?)",
                channel_name,
                address,
                merchant_deposit,
                customer_deposit,
                state,
            )
            .execute(&mut transaction)
            .await
            .map(|_| ());

            transaction.commit().await?;

            Ok(result?)
        })()
        .await
        .map_err(|e| (Inactive::from_state(state, StateName::Inactive).unwrap(), e))
    }

    async fn channel_address(&self, channel_name: &ChannelName) -> Result<ZkChannelAddress> {
        Ok(sqlx::query!(
            r#"
            SELECT address AS "address: ZkChannelAddress"
            FROM customer_channels
            WHERE label = ?"#,
            channel_name,
        )
        .fetch(self)
        .next()
        .await
        .ok_or_else(|| Error::NoSuchChannel(channel_name.clone()))?
        .map(|record| record.address)?)
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
        let channels = sqlx::query!(
            r#"SELECT
                label AS "label: ChannelName",
                state AS "state: State",
                address AS "address: ZkChannelAddress",
                customer_deposit AS "customer_deposit: CustomerBalance",
                merchant_deposit AS "merchant_deposit: MerchantBalance"
            FROM customer_channels"#
        )
        .fetch_all(self)
        .await?
        .into_iter()
        .map(|r| ChannelDetails {
            label: r.label,
            state: r.state,
            address: r.address,
            customer_deposit: r.customer_deposit,
            merchant_deposit: r.merchant_deposit,
        })
        .collect();

        Ok(channels)
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
        S: ZkChannelState + Send,
        F: FnOnce(S::ZkAbacusState) -> std::result::Result<(State, T), E> + Send + 'a,
        T: Send + 'static,
        E: Send + 'static,
    >(
        &'a self,
        channel_name: &ChannelName,
        with_zkabacus_state: F,
    ) -> Result<std::result::Result<T, E>> {
        let result = <Self as QueryCustomer>::with_channel_state_erased(
            self,
            channel_name,
            Box::new(
                // Extract the inner zkAbacus type from the state enum and make sure it matches
                |state| match S::to_zkabacus_state(state) {
                    Ok(zkabacus_state) => match with_zkabacus_state(zkabacus_state) {
                        Ok((state, t)) => Ok((state, Box::new(t))),
                        Err(e) => Err(Box::new(Ok::<E, StateError>(e))),
                    },
                    Err(unexpected_state) => {
                        Err(Box::new(Err::<E, StateError>(unexpected_state.into())))
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
                let error_result: std::result::Result<E, StateError> =
                    *error_result.downcast().unwrap();
                match error_result {
                    // Error returned by the closure
                    Ok(e) => Ok(Err(e)),
                    // Error returned because the state didn't match the one in the database.
                    Err(StateError::UnexpectedState(e)) => return Err(e.into()),
                    // Error returned because the caller requested an impossible state.
                    Err(StateError::ImpossibleState(e)) => return Err(e.into()),
                }
            }
        }
    }

    async fn with_closeable_channel<'a, E: Send + 'static>(
        &self,
        channel_name: &ChannelName,
        with_closeable_state: impl for<'s> FnOnce(State) -> std::result::Result<(State, ClosingMessage), E>
            + Send
            + 'a,
    ) -> Result<std::result::Result<ClosingMessage, E>> {
        let result = <Self as QueryCustomer>::with_channel_state_erased(
            self,
            channel_name,
            Box::new(|state| match with_closeable_state(state) {
                Ok((state, t)) => {
                    // Only allow updates that result in the PendingClose status.
                    if let State::PendingClose(_) = state {
                        Ok((state, Box::new(t)))
                    } else {
                        Err(Box::new(Err::<E, Error>(Error::CloseFailure)))
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
