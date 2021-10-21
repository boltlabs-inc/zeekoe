use {async_trait::async_trait, futures::StreamExt, rand::rngs::StdRng, thiserror::Error};

pub use super::connect_sqlite;
use crate::database::SqlitePool;
use crate::{escrow::types::ContractId, protocol::ChannelStatus};
use serde::{Deserialize, Serialize};
use zkabacus_crypto::{
    revlock::{RevocationLock, RevocationSecret},
    ChannelId, CommitmentParameters, CustomerBalance, KeyPair, MerchantBalance, Nonce,
    RangeConstraintParameters,
};

type Result<T> = std::result::Result<T, Error>;

#[async_trait]
pub trait QueryMerchant: Send + Sync {
    /// Perform all the DB migrations defined in src/database/migrations/merchant/*.sql
    async fn migrate(&self) -> Result<()>;

    /// Atomically insert a nonce, returning `true` if it was added successfully
    /// and `false` if it already exists.
    async fn insert_nonce(&self, nonce: &Nonce) -> Result<bool>;

    /// Insert a revocation lock and optional secret, returning all revocations
    /// that existed prior.
    async fn insert_revocation(
        &self,
        revocation: &RevocationLock,
        secret: Option<&RevocationSecret>,
    ) -> Result<Vec<Option<RevocationSecret>>>;

    /// Fetch a singleton merchant config, creating it if it doesn't already exist.
    async fn fetch_or_create_config(
        &self,
        rng: &mut StdRng,
    ) -> Result<zkabacus_crypto::merchant::Config>;

    /// Create a new merchant channel.
    async fn new_channel(
        &self,
        channel_id: &ChannelId,
        contract_id: &ContractId,
        merchant_deposit: &MerchantBalance,
        customer_deposit: &CustomerBalance,
    ) -> Result<()>;

    /// Update an existing merchant channel's status to a new state, only if it is currently in the
    /// expected state.
    async fn compare_and_swap_channel_status(
        &self,
        channel_id: &ChannelId,
        expected: &ChannelStatus,
        new: &ChannelStatus,
    ) -> Result<()>;

    /// Update an existing merchant channel's status to PendingClose, if it is in a state that can
    /// do so allowably (e.g. not already in a close flow).
    async fn update_status_to_pending_close(&self, channel_id: &ChannelId) -> Result<()>;

    /// Update the closing balances of the channel, only if it is currently in the expected state.
    ///
    /// This should only be called once the balances are finalized on chain and maintains the
    /// following invariants:
    /// - The customer balance can be set at most once.
    /// - The merchant balance can only be increased.
    /// If either of these invariants are violated, will raise [`Error::InvalidBalanceUpdate`].
    async fn update_closing_balances(
        &self,
        channel_id: &ChannelId,
        expected_status: &ChannelStatus,
        merchant_balance: MerchantBalance,
        customer_balance: Option<CustomerBalance>,
    ) -> Result<()>;

    /// Get information about every channel in the database.
    async fn get_channels(&self) -> Result<Vec<ChannelDetails>>;

    /// Get channel status for a particular channel based on its [`ChannelId`].
    async fn channel_status(&self, channel_id: &ChannelId) -> Result<ChannelStatus>;

    /// Get closing balances for a particular channel based on its [`ChannelId`]. These  may not
    /// sum to total channel balance if the status is not [`Closed`](ChannelStatus::Closed).
    async fn closing_balances(&self, channel_id: &ChannelId) -> Result<ClosingBalances>;

    /// Get initial channel balances for a particular channel based on its [`ChannelId`].
    async fn initial_balances(
        &self,
        channel_id: &ChannelId,
    ) -> Result<(MerchantBalance, CustomerBalance)>;

    /// Get contract information for a particular channel based on its [`ChannelId`].
    async fn contract_details(&self, channel_id: &ChannelId) -> Result<ContractId>;

    /// Get details about a particular channel based on a unique prefix of its [`ChannelId`].
    async fn get_channel_details_by_prefix(&self, prefix: &str) -> Result<ChannelDetails>;
}

/// An error when accessing the merchant database.
#[derive(Debug, Error)]
pub enum Error {
    /// A channel with the given ID could not be found.
    #[error("Could not find channel with id: {0}")]
    ChannelNotFound(ChannelId),
    /// A channel with the given ID prefix could not be found.
    #[error("No channel with id that starts with: {0}")]
    ChannelNotFoundWithPrefix(String),
    /// Multiple channels were found with a given prefix.
    #[error("Multiple channels with prefix: {0}")]
    ChannelIdCollision(String),
    /// Tried to search by a malformed channel id.
    #[error("Invalid channel id: {0}")]
    MalformedChannelId(String),
    /// The channel status was expected to be one thing, but it was another.
    #[error("Unexpected status for channel {channel_id} (expected {expected:?}, found {found})")]
    UnexpectedChannelStatus {
        channel_id: ChannelId,
        expected: Vec<ChannelStatus>,
        found: ChannelStatus,
    },
    /// A channel balance update was invalid.
    #[error("Failed to update channel balance to invalid set (merchant: {0:?}, customer: {1:?})")]
    InvalidBalanceUpdate(MerchantBalance, Option<CustomerBalance>),
    /// An underlying database error occurred.
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    /// An underlying database migration error occurred.
    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),
}

/// The contents of a row of the database for a particular channel.
pub struct ChannelDetails {
    pub channel_id: ChannelId,
    pub status: ChannelStatus,
    pub contract_id: ContractId,
    pub merchant_deposit: MerchantBalance,
    pub customer_deposit: CustomerBalance,
    pub closing_balances: ClosingBalances,
}

/// The balances of a channel at closing. These may change during a close flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosingBalances {
    pub merchant_balance: Option<MerchantBalance>,
    pub customer_balance: Option<CustomerBalance>,
}

zkabacus_crypto::impl_sqlx_for_bincode_ty!(ClosingBalances);

impl Default for ClosingBalances {
    fn default() -> Self {
        Self {
            merchant_balance: None,
            customer_balance: None,
        }
    }
}

#[async_trait]
impl QueryMerchant for SqlitePool {
    async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("src/database/migrations/merchant")
            .run(self)
            .await?;
        Ok(())
    }

    async fn insert_nonce(&self, nonce: &Nonce) -> Result<bool> {
        let res = sqlx::query!(
            "INSERT INTO nonces (data) VALUES (?) ON CONFLICT (data) DO NOTHING",
            nonce
        )
        .execute(self)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    async fn insert_revocation(
        &self,
        lock: &RevocationLock,
        secret: Option<&RevocationSecret>,
    ) -> Result<Vec<Option<RevocationSecret>>> {
        let mut transaction = self.begin().await?;
        let existing_pairs = sqlx::query!(
            r#"
            SELECT secret AS "secret: RevocationSecret"
            FROM revocations
            WHERE lock = ?
            "#,
            lock,
        )
        .fetch_all(&mut transaction)
        .await?
        .into_iter()
        .map(|r| r.secret)
        .collect();

        sqlx::query!(
            "INSERT INTO revocations (lock, secret) VALUES (?, ?)",
            lock,
            secret,
        )
        .execute(&mut transaction)
        .await?;

        transaction.commit().await?;
        Ok(existing_pairs)
    }

    async fn fetch_or_create_config(
        &self,
        rng: &mut StdRng,
    ) -> Result<zkabacus_crypto::merchant::Config> {
        let mut transaction = self.begin().await?;

        let existing = sqlx::query!(
            r#"
            SELECT
                signing_keypair AS "signing_keypair: KeyPair",
                revocation_commitment_parameters
                    AS "revocation_commitment_parameters: CommitmentParameters",
                range_constraint_parameters
                    AS "range_constraint_parameters: RangeConstraintParameters"
            FROM merchant_config
            "#,
        )
        .fetch(&mut transaction)
        .next()
        .await;

        match existing {
            Some(Ok(existing)) => {
                transaction.commit().await?;
                return Ok(zkabacus_crypto::merchant::Config::from_parts(
                    existing.signing_keypair,
                    existing.revocation_commitment_parameters,
                    existing.range_constraint_parameters,
                ));
            }
            Some(Err(err)) => return Err(err.into()),
            None => {}
        }

        let new_config = zkabacus_crypto::merchant::Config::new(rng);

        let signing_keypair = new_config.signing_keypair();
        let revocation_commitment_parameters = new_config.revocation_commitment_parameters();
        let range_constraint_parameters = new_config.range_constraint_parameters();

        sqlx::query!(
            r#"
            INSERT INTO merchant_config (
                signing_keypair,
                revocation_commitment_parameters,
                range_constraint_parameters
            )
            VALUES (?, ?, ?)
            "#,
            signing_keypair,
            revocation_commitment_parameters,
            range_constraint_parameters,
        )
        .execute(&mut transaction)
        .await?;

        transaction.commit().await?;
        Ok(new_config)
    }

    async fn new_channel(
        &self,
        channel_id: &ChannelId,
        contract_id: &ContractId,
        merchant_deposit: &MerchantBalance,
        customer_deposit: &CustomerBalance,
    ) -> Result<()> {
        let default_balances = ClosingBalances::default();
        sqlx::query!(
            "INSERT INTO merchant_channels (
                channel_id,
                contract_id,
                merchant_deposit,
                customer_deposit,
                status,
                closing_balances
            )
            VALUES (?, ?, ?, ?, ?, ?)",
            channel_id,
            contract_id,
            merchant_deposit,
            customer_deposit,
            ChannelStatus::Originated,
            default_balances,
        )
        .execute(self)
        .await?;

        Ok(())
    }

    async fn compare_and_swap_channel_status(
        &self,
        channel_id: &ChannelId,
        expected: &ChannelStatus,
        new: &ChannelStatus,
    ) -> Result<()> {
        // TODO: This should return a different error when the CAS fails
        let mut transaction = self.begin().await?;

        // Find out the current status
        let result: Option<ChannelStatus> = sqlx::query!(
            r#"
            SELECT status AS "status: Option<ChannelStatus>"
            FROM merchant_channels
            WHERE channel_id = ?
            "#,
            channel_id,
        )
        .fetch_one(&mut transaction)
        .await?
        .status;

        // Only if the current status is what was expected, update the status to the new status
        match result {
            None => Err(Error::ChannelNotFound(*channel_id)),
            Some(ref current) if current == expected => {
                sqlx::query!(
                    "UPDATE merchant_channels
                    SET status = ?
                    WHERE channel_id = ?",
                    new,
                    channel_id
                )
                .execute(&mut transaction)
                .await?;

                transaction.commit().await?;
                Ok(())
            }
            Some(unexpected_status) => Err(Error::UnexpectedChannelStatus {
                channel_id: *channel_id,
                expected: vec![*expected],
                found: unexpected_status,
            }),
        }
    }

    async fn update_status_to_pending_close(&self, channel_id: &ChannelId) -> Result<()> {
        let mut transaction = self.begin().await?;

        // Find out current status
        let result: Option<ChannelStatus> = sqlx::query!(
            r#"
            SELECT status AS "status: Option<ChannelStatus>"
            FROM merchant_channels
            WHERE channel_id = ?
            "#,
            channel_id,
        )
        .fetch_one(&mut transaction)
        .await?
        .status;

        // Only update status if it is an allowable value.
        match result {
            None => Err(Error::ChannelNotFound(*channel_id)),
            Some(ChannelStatus::MerchantFunded)
            | Some(ChannelStatus::Active)
            | Some(ChannelStatus::PendingExpiry)
            | Some(ChannelStatus::PendingMutualClose) => {
                sqlx::query!(
                    "UPDATE merchant_channels
                    SET status = ?
                    WHERE channel_id = ?",
                    ChannelStatus::PendingClose,
                    channel_id
                )
                .execute(&mut transaction)
                .await?;

                transaction.commit().await?;
                Ok(())
            }
            Some(unexpected_status) => Err(Error::UnexpectedChannelStatus {
                channel_id: *channel_id,
                expected: vec![
                    ChannelStatus::Originated,
                    ChannelStatus::CustomerFunded,
                    ChannelStatus::MerchantFunded,
                    ChannelStatus::Active,
                    ChannelStatus::PendingExpiry,
                    ChannelStatus::PendingMutualClose,
                ],
                found: unexpected_status,
            }),
        }
    }

    async fn update_closing_balances(
        &self,
        channel_id: &ChannelId,
        expected_status: &ChannelStatus,
        merchant_balance: MerchantBalance,
        customer_balance: Option<CustomerBalance>,
    ) -> Result<()> {
        let mut transaction = self.begin().await?;

        // Find out the current status
        let result = sqlx::query!(
            r#"
            SELECT
                status AS "status: Option<ChannelStatus>",
                closing_balances AS "closing_balances: ClosingBalances"
            FROM merchant_channels
            WHERE channel_id = ?
            "#,
            channel_id,
        )
        .fetch_one(&mut transaction)
        .await?;

        // Only if the current status is what was expected, update the channel balances.
        match result.status {
            None => Err(Error::ChannelNotFound(*channel_id)),
            Some(ref current) if current == expected_status => {
                let closing_balances = result.closing_balances;

                // Make sure we're not decreasing merchant balance.
                if let Some(original) = closing_balances.merchant_balance {
                    if original.into_inner() > merchant_balance.into_inner() {
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
                    "UPDATE merchant_channels
                    SET closing_balances = ?
                    WHERE channel_id = ?",
                    updated_closing_balances,
                    channel_id,
                )
                .execute(&mut transaction)
                .await?;

                transaction.commit().await?;
                Ok(())
            }
            Some(unexpected_status) => Err(Error::UnexpectedChannelStatus {
                channel_id: *channel_id,
                expected: vec![*expected_status],
                found: unexpected_status,
            }),
        }
    }

    async fn get_channels(&self) -> Result<Vec<ChannelDetails>> {
        let channels = sqlx::query!(
            r#"
            SELECT
                channel_id AS "channel_id: ChannelId",
                status as "status: ChannelStatus",
                contract_id AS "contract_id: ContractId",
                merchant_deposit AS "merchant_deposit: MerchantBalance",
                customer_deposit AS "customer_deposit: CustomerBalance",
                closing_balances AS "closing_balances: ClosingBalances"
            FROM merchant_channels
            "#
        )
        .fetch_all(self)
        .await?
        .into_iter()
        .map(|r| ChannelDetails {
            channel_id: r.channel_id,
            status: r.status,
            contract_id: r.contract_id,
            merchant_deposit: r.merchant_deposit,
            customer_deposit: r.customer_deposit,
            closing_balances: r.closing_balances,
        })
        .collect();

        Ok(channels)
    }

    async fn channel_status(&self, channel_id: &ChannelId) -> Result<ChannelStatus> {
        let mut results = sqlx::query!(
            r#"
            SELECT status as "status: ChannelStatus"
            FROM merchant_channels
            WHERE channel_id = ?
            LIMIT 2
            "#,
            channel_id
        )
        .fetch_all(self)
        .await?
        .into_iter();

        let status = match results.next() {
            None => return Err(Error::ChannelNotFound(*channel_id)),
            Some(record) => record.status,
        };

        if results.next().is_some() {
            return Err(Error::ChannelIdCollision(channel_id.to_string()));
        }

        Ok(status)
    }

    async fn closing_balances(&self, channel_id: &ChannelId) -> Result<ClosingBalances> {
        let mut results = sqlx::query!(
            r#"
            SELECT closing_balances as "closing_balances: ClosingBalances"
            FROM merchant_channels
            WHERE channel_id = ?
            LIMIT 2
            "#,
            channel_id
        )
        .fetch_all(self)
        .await?
        .into_iter();

        let closing_balances = match results.next() {
            None => return Err(Error::ChannelNotFound(*channel_id)),
            Some(record) => record.closing_balances,
        };

        if results.next().is_some() {
            return Err(Error::ChannelIdCollision(channel_id.to_string()));
        }

        Ok(closing_balances)
    }

    async fn initial_balances(
        &self,
        channel_id: &ChannelId,
    ) -> Result<(MerchantBalance, CustomerBalance)> {
        let mut results = sqlx::query!(
            r#"
            SELECT 
                merchant_deposit as "merchant_balance: MerchantBalance",
                customer_deposit as "customer_balance: CustomerBalance"
            FROM merchant_channels
            WHERE channel_id = ?
            LIMIT 2
            "#,
            channel_id
        )
        .fetch_all(self)
        .await?
        .into_iter();

        let initial_balances = match results.next() {
            None => return Err(Error::ChannelNotFound(*channel_id)),
            Some(record) => (record.merchant_balance, record.customer_balance),
        };

        if results.next().is_some() {
            return Err(Error::ChannelIdCollision(channel_id.to_string()));
        }

        Ok(initial_balances)
    }

    async fn contract_details(&self, channel_id: &ChannelId) -> Result<ContractId> {
        let mut result = sqlx::query!(
            r#"
            SELECT contract_id as "contract_id: ContractId"
            FROM merchant_channels
            WHERE channel_id = ?
            LIMIT 2
            "#,
            channel_id
        )
        .fetch_all(self)
        .await?
        .into_iter();

        let contract_details = match result.next() {
            None => return Err(Error::ChannelNotFound(*channel_id)),
            Some(record) => record.contract_id,
        };

        if result.next().is_some() {
            return Err(Error::ChannelIdCollision(channel_id.to_string()));
        }

        Ok(contract_details)
    }

    async fn get_channel_details_by_prefix(&self, prefix: &str) -> Result<ChannelDetails> {
        let query = format!("{}%", &prefix);
        let mut results = sqlx::query!(
            r#"
            SELECT
                channel_id AS "channel_id: ChannelId",
                status as "status: ChannelStatus",
                contract_id AS "contract_id: ContractId",
                merchant_deposit AS "merchant_deposit: MerchantBalance",
                customer_deposit AS "customer_deposit: CustomerBalance",
                closing_balances AS "closing_balances: ClosingBalances"
            FROM merchant_channels
            WHERE channel_id LIKE ?
            LIMIT 2
            "#,
            query
        )
        .fetch_all(self)
        .await?
        .into_iter();

        let details = match results.next() {
            None => return Err(Error::ChannelNotFoundWithPrefix(prefix.to_string())),
            Some(channel) => ChannelDetails {
                channel_id: channel.channel_id,
                status: channel.status,
                contract_id: channel.contract_id,
                merchant_deposit: channel.merchant_deposit,
                customer_deposit: channel.customer_deposit,
                closing_balances: channel.closing_balances,
            },
        };

        if results.next().is_some() {
            return Err(Error::ChannelIdCollision(prefix.to_string()));
        }

        Ok(details)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::SqlitePoolOptions;
    use {rand::SeedableRng, strum::IntoEnumIterator, tezedge::OriginatedAddress};

    use zkabacus_crypto::internal::{
        test_new_nonce, test_new_revocation_lock, test_new_revocation_secret, test_verify_pair,
    };
    use zkabacus_crypto::{CustomerRandomness, MerchantRandomness, Verification};

    // The default dummy originated contract address, per https://tezos.stackexchange.com/a/2270
    const DEFAULT_ADDR: &str = "KT1Mjjcb6tmSsLm7Cb3DSQszePjfchPM4Uxm";

    fn assert_valid_pair(lock: &RevocationLock, secret: &RevocationSecret) {
        assert!(
            matches!(test_verify_pair(lock, secret), Verification::Verified),
            "revocation lock {:?} unlocks with {:?}",
            lock,
            secret
        );
    }

    async fn create_migrated_db() -> Result<SqlitePool> {
        let conn = SqlitePoolOptions::new().connect("sqlite::memory:").await?;
        conn.migrate().await?;
        Ok(conn)
    }

    #[tokio::test]
    async fn test_migrate() -> Result<()> {
        create_migrated_db().await?;
        Ok(())
    }

    #[tokio::test]
    async fn test_insert_nonce() -> Result<()> {
        let conn = create_migrated_db().await?;
        let mut rng = rand::thread_rng();

        let nonce = test_new_nonce(&mut rng);
        assert!(conn.insert_nonce(&nonce).await?);
        assert!(!conn.insert_nonce(&nonce).await?);

        let nonce2 = test_new_nonce(&mut rng);
        assert!(conn.insert_nonce(&nonce2).await?);
        Ok(())
    }

    #[tokio::test]
    async fn test_insert_revocation() -> Result<()> {
        let conn = create_migrated_db().await?;
        let mut rng = rand::thread_rng();

        let secret1 = test_new_revocation_secret(&mut rng);
        let lock1 = test_new_revocation_lock(&secret1);

        // Each time we insert a lock (& optional secret), it returns all previously
        // stored pairs for that lock.
        let result = conn.insert_revocation(&lock1, None).await?;
        assert_eq!(result.len(), 0);

        conn.insert_revocation(&lock1, Some(&secret1)).await?;

        let result = conn.insert_revocation(&lock1, None).await?;
        assert!(result[0].is_none());
        assert!(result[1].is_some());
        assert_valid_pair(&lock1, result[1].as_ref().unwrap());
        assert_eq!(result.len(), 2);

        // Inserting a previously-unseen lock should not return any old pairs.
        let secret2 = test_new_revocation_secret(&mut rng);
        let lock2 = test_new_revocation_lock(&secret2);
        let result = conn.insert_revocation(&lock2, Some(&secret2)).await?;
        assert_eq!(result.len(), 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_merchant_statuses() -> Result<()> {
        let conn = create_migrated_db().await?;

        // Create channel and set its initial status.
        let channel_id = insert_new_channel(&conn).await?;

        // Get a list of every possible status, assuming that the first one is what channels
        // are inserted with
        let mut statuses = ChannelStatus::iter();
        let mut current_status = statuses.next().unwrap();

        // Make sure that every legal channel status can be inserted into the db.
        for next_status in statuses {
            conn.compare_and_swap_channel_status(&channel_id, &current_status, &next_status)
                .await?;

            current_status = next_status;
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_merchant_config() -> Result<()> {
        let conn = create_migrated_db().await?;
        let mut rng = StdRng::from_entropy();

        let config1 = conn.fetch_or_create_config(&mut rng).await?;
        let config2 = conn.fetch_or_create_config(&mut rng).await?;

        // The two configs should be equal, because the first is now permanently the config
        assert_eq!(
            config1.signing_keypair().public_key(),
            config2.signing_keypair().public_key()
        );
        assert_eq!(
            config1.revocation_commitment_parameters(),
            config2.revocation_commitment_parameters()
        );
        assert_eq!(
            config1.range_constraint_parameters(),
            config1.range_constraint_parameters()
        );

        Ok(())
    }

    async fn insert_new_channel(conn: &SqlitePool) -> Result<ChannelId> {
        let mut rng = StdRng::from_entropy();

        let cid_m = MerchantRandomness::new(&mut rng);
        let cid_c = CustomerRandomness::new(&mut rng);
        let pk = KeyPair::new(&mut rng).public_key().clone();
        let channel_id = ChannelId::new(cid_m, cid_c, &pk, &[], &[]);
        let contract_id =
            ContractId::new(OriginatedAddress::from_base58check(DEFAULT_ADDR).unwrap());

        let merchant_deposit = MerchantBalance::try_new(5).unwrap();
        let customer_deposit = CustomerBalance::try_new(5).unwrap();
        conn.new_channel(
            &channel_id,
            &contract_id,
            &merchant_deposit,
            &customer_deposit,
        )
        .await?;

        Ok(channel_id)
    }

    #[tokio::test]
    async fn test_merchant_channels() -> Result<()> {
        let conn = create_migrated_db().await?;
        let channel_id = insert_new_channel(&conn).await?;
        conn.compare_and_swap_channel_status(
            &channel_id,
            &ChannelStatus::Originated,
            &ChannelStatus::CustomerFunded,
        )
        .await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_closing_balance_update() -> Result<()> {
        // set up new db
        let conn = create_migrated_db().await?;

        // Make a new random channel.
        let channel_id = insert_new_channel(&conn).await?;

        // make sure the initial closing balances are not set
        let mut closing_balances = conn.closing_balances(&channel_id).await?;
        assert!(matches!(closing_balances.merchant_balance, None));
        assert!(matches!(closing_balances.customer_balance, None));

        // update closing balances
        let new_merchant_balance = MerchantBalance::try_new(10).unwrap();
        let new_customer_balance = Some(CustomerBalance::try_new(0).unwrap());
        conn.update_closing_balances(
            &channel_id,
            &ChannelStatus::Originated,
            new_merchant_balance,
            new_customer_balance,
        )
        .await?;

        // make sure the updated closing balances are set correctly
        closing_balances = conn.closing_balances(&channel_id).await?;
        assert!(
            matches!(closing_balances.merchant_balance, Some(_))
                && closing_balances.merchant_balance.unwrap().into_inner()
                    == new_merchant_balance.into_inner()
        );
        assert!(
            matches!(closing_balances.customer_balance, Some(_))
                && closing_balances.customer_balance.unwrap().into_inner() == 0
        );

        Ok(())
    }
}
