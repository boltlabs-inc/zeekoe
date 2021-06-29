use {async_trait::async_trait, futures::StreamExt, rand::rngs::StdRng, thiserror::Error};

use crate::database::SqlitePool;
use crate::protocol::{ChannelStatus, ContractId};
use std::str::FromStr;
use zkabacus_crypto::{
    revlock::{RevocationLock, RevocationSecret},
    ChannelId, CommitmentParameters, CustomerBalance, KeyPair, MerchantBalance, Nonce,
    RangeProofParameters,
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

    /// Get information about every channel in the database.
    async fn get_channels(&self) -> Result<Vec<(ChannelId, ChannelStatus)>>;

    /// Get details about a particular channel based on a unique prefix of its [`ChannelId`].
    // TODO: This currently does not implement prefix matching
    async fn get_channel_details(&self, prefix: &str) -> Result<ChannelDetails>;
}

/// An error when accessing the merchant database.
#[derive(Debug, Error)]
pub enum Error {
    /// A channel with the given ID could not be found.
    #[error("Could not find channel with id {0}")]
    ChannelNotFound(ChannelId),
    /// A channel with the given ID prefix could not be found.
    #[error("Could not find channel with an id that starts with {0}")]
    ChannelNotFoundWithPrefix(String),
    /// Tried to search by a malformed channel id.
    #[error("`{0}` was not a valid channel id")]
    MalformedChannelId(String),
    /// The channel status was expected to be one thing, but it was another.
    #[error("Unexpected status for channel {channel_id} (expected {expected}, found {found})")]
    UnexpectedChannelStatus {
        channel_id: ChannelId,
        expected: ChannelStatus,
        found: ChannelStatus,
    },
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
                signing_keypair
                    AS "signing_keypair: KeyPair",
                revocation_commitment_parameters
                    AS "revocation_commitment_parameters: CommitmentParameters",
                range_proof_parameters
                    AS "range_proof_parameters: RangeProofParameters"
            FROM
                merchant_config
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
                    existing.range_proof_parameters,
                ));
            }
            Some(Err(err)) => return Err(err.into()),
            None => {}
        }

        let new_config = zkabacus_crypto::merchant::Config::new(rng);

        let signing_keypair = new_config.signing_keypair();
        let revocation_commitment_parameters = new_config.revocation_commitment_parameters();
        let range_proof_parameters = new_config.range_proof_parameters();

        sqlx::query!(
            r#"
            INSERT INTO
                merchant_config
            (signing_keypair, revocation_commitment_parameters, range_proof_parameters)
                VALUES
            (?, ?, ?)
            "#,
            signing_keypair,
            revocation_commitment_parameters,
            range_proof_parameters,
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
        sqlx::query!(
            "INSERT INTO merchant_channels (
                channel_id,
                contract_id,
                merchant_deposit,
                customer_deposit,
                status
            ) VALUES (?, ?, ?, ?, ?)",
            channel_id,
            contract_id,
            merchant_deposit,
            customer_deposit,
            ChannelStatus::Originated
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
            r#"SELECT status AS "status: Option<ChannelStatus>"
            FROM merchant_channels
            WHERE
                channel_id = ?"#,
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
                expected: *expected,
                found: unexpected_status,
            }),
        }
    }

    async fn get_channels(&self) -> Result<Vec<(ChannelId, ChannelStatus)>> {
        let channels = sqlx::query!(
            r#"SELECT
                channel_id AS "channel_id: ChannelId",
                status as "status: ChannelStatus"
            FROM merchant_channels"#
        )
        .fetch_all(self)
        .await?
        .into_iter()
        .map(|r| (r.channel_id, r.status))
        .collect();

        Ok(channels)
    }

    async fn get_channel_details(&self, prefix: &str) -> Result<ChannelDetails> {
        let channel_id = ChannelId::from_str(prefix)
            .map_err(|_| Error::MalformedChannelId(prefix.to_string()))?;
        let result = sqlx::query!(
            r#"
            SELECT
                channel_id AS "channel_id: ChannelId",
                status as "status: ChannelStatus",
                contract_id AS "contract_id: ContractId",
                merchant_deposit AS "merchant_deposit: MerchantBalance",
                customer_deposit AS "customer_deposit: CustomerBalance"
            FROM merchant_channels
            WHERE channel_id = ?
        "#,
            channel_id
        )
        .fetch_optional(self)
        .await?;

        match result {
            None => Err(Error::ChannelNotFoundWithPrefix(prefix.to_string())),
            Some(channel) => Ok(ChannelDetails {
                channel_id: channel.channel_id,
                status: channel.status,
                contract_id: channel.contract_id,
                merchant_deposit: channel.merchant_deposit,
                customer_deposit: channel.customer_deposit,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::SqlitePoolOptions;
    use rand::SeedableRng;

    use zkabacus_crypto::internal::{
        test_new_nonce, test_new_revocation_lock, test_new_revocation_secret, test_verify_pair,
    };
    use zkabacus_crypto::{CustomerRandomness, MerchantRandomness, Verification};

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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_migrate() -> Result<()> {
        create_migrated_db().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
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

    #[tokio::test(flavor = "multi_thread")]
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
            config1.range_proof_parameters(),
            config1.range_proof_parameters()
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_merchant_channels() -> Result<()> {
        let conn = create_migrated_db().await?;
        let mut rng = StdRng::from_entropy();

        let cid_m = MerchantRandomness::new(&mut rng);
        let cid_c = CustomerRandomness::new(&mut rng);
        let pk = KeyPair::new(&mut rng).public_key().clone();
        let channel_id = ChannelId::new(cid_m, cid_c, &pk, &[], &[]);
        let contract_id = ContractId {};

        let merchant_deposit = MerchantBalance::try_new(5).unwrap();
        let customer_deposit = CustomerBalance::try_new(5).unwrap();
        conn.new_channel(
            &channel_id,
            &contract_id,
            &merchant_deposit,
            &customer_deposit,
        )
        .await?;
        conn.compare_and_swap_channel_status(
            &channel_id,
            &ChannelStatus::Originated,
            &ChannelStatus::CustomerFunded,
        )
        .await?;

        Ok(())
    }
}
