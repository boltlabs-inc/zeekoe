use {async_trait::async_trait, futures::StreamExt, rand::rngs::StdRng};

use crate::database::SqlitePool;
use crate::protocol::ChannelStatus;
use zkabacus_crypto::{
    revlock::{RevocationLock, RevocationSecret},
    ChannelId, CommitmentParameters, KeyPair, Nonce, RangeProofParameters,
};

#[async_trait]
pub trait QueryMerchant {
    /// Perform all the DB migrations defined in src/database/migrations/merchant/*.sql
    async fn migrate(&self) -> sqlx::Result<()>;

    /// Atomically insert a nonce, returning `true` if it was added successfully
    /// and `false` if it already exists.
    async fn insert_nonce(&self, nonce: &Nonce) -> sqlx::Result<bool>;

    /// Insert a revocation lock and optional secret, returning all revocations
    /// that existed prior.
    async fn insert_revocation(
        &self,
        revocation: &RevocationLock,
        secret: Option<&RevocationSecret>,
    ) -> sqlx::Result<Vec<Option<RevocationSecret>>>;

    /// Fetch a singleton merchant config, creating it if it doesn't already exist.
    async fn fetch_or_create_config(
        &self,
        rng: &mut StdRng,
    ) -> sqlx::Result<zkabacus_crypto::merchant::Config>;

    /// Create a new merchant channel.
    async fn create_merchant_channel(&self, channel_id: &ChannelId) -> sqlx::Result<()>;

    /// Update an existing merchant channel's status.
    async fn update_merchant_channel_status(
        &self,
        channel_id: &ChannelId,
        status: &ChannelStatus,
    ) -> sqlx::Result<()>;
}

#[async_trait]
impl QueryMerchant for SqlitePool {
    async fn migrate(&self) -> sqlx::Result<()> {
        sqlx::migrate!("src/database/migrations/merchant")
            .run(self)
            .await?;
        Ok(())
    }

    async fn insert_nonce(&self, nonce: &Nonce) -> sqlx::Result<bool> {
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
    ) -> sqlx::Result<Vec<Option<RevocationSecret>>> {
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
    ) -> sqlx::Result<zkabacus_crypto::merchant::Config> {
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
            Some(Err(err)) => Err(err)?,
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

    async fn create_merchant_channel(&self, channel_id: &ChannelId) -> sqlx::Result<()> {
        sqlx::query!(
            "INSERT INTO merchant_channels (channel_id, status) VALUES (?, ?)",
            channel_id,
            ChannelStatus::Originated
        )
        .execute(self)
        .await?;

        Ok(())
    }

    async fn update_merchant_channel_status(
        &self,
        channel_id: &ChannelId,
        status: &ChannelStatus,
    ) -> sqlx::Result<()> {
        sqlx::query!(
            "UPDATE merchant_channels
            SET status = ?
            WHERE channel_id = ?",
            status,
            channel_id
        )
        .execute(self)
        .await?;
        Ok(())
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

    async fn create_migrated_db() -> Result<SqlitePool, anyhow::Error> {
        let conn = SqlitePoolOptions::new().connect("sqlite::memory:").await?;
        conn.migrate().await?;
        Ok(conn)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_migrate() -> Result<(), anyhow::Error> {
        create_migrated_db().await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_insert_nonce() -> Result<(), anyhow::Error> {
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
    async fn test_insert_revocation() -> Result<(), anyhow::Error> {
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
    async fn test_merchant_config() -> Result<(), anyhow::Error> {
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
    async fn test_merchant_channels() -> Result<(), anyhow::Error> {
        let conn = create_migrated_db().await?;
        let mut rng = StdRng::from_entropy();

        let cid_m = MerchantRandomness::new(&mut rng);
        let cid_c = CustomerRandomness::new(&mut rng);
        let pk = KeyPair::new(&mut rng).public_key().clone();
        let channel_id = ChannelId::new(cid_m, cid_c, &pk, &[], &[]);

        conn.create_merchant_channel(&channel_id).await?;
        conn.update_merchant_channel_status(&channel_id, &ChannelStatus::CustomerFunded)
            .await?;

        Ok(())
    }
}
