use crate::database::SqlitePool;
use zkabacus_crypto::{
    revlock::{RevocationLock, RevocationSecret},
    CommitmentParameters, KeyPair, Nonce, RangeProofParameters,
};
use {async_trait::async_trait, futures::StreamExt, rand::rngs::StdRng};

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
    ) -> sqlx::Result<Vec<(RevocationLock, Option<RevocationSecret>)>>;

    async fn fetch_or_initialize_config(
        &self,
        rng: &mut StdRng,
    ) -> sqlx::Result<zkabacus_crypto::merchant::Config>;
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
    ) -> sqlx::Result<Vec<(RevocationLock, Option<RevocationSecret>)>> {
        let mut transaction = self.begin().await?;
        let existing_pairs = sqlx::query!(
            r#"
            SELECT
                lock AS "lock: RevocationLock",
                secret AS "secret: RevocationSecret"
            FROM revocations
            WHERE lock = ?
            "#,
            lock,
        )
        .fetch_all(&mut transaction)
        .await?
        .into_iter()
        .map(|r| (r.lock, r.secret))
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

    async fn fetch_or_initialize_config(
        &self,
        rng: &mut StdRng,
    ) -> sqlx::Result<zkabacus_crypto::merchant::Config> {
        let mut transaction = self.begin().await?;
        let existing_config = sqlx::query!(
            r#"
            SELECT
                signing_keypair
                    AS "signing_keypair: KeyPair",
                revocation_commitment_parameters
                    AS "revocation_commitment_parameters: CommitmentParameters",
                range_proof_parameters
                    AS "range_proof_parameters: RangeProofParameters"
            FROM
                config
            "#,
        )
        .fetch(&mut transaction)
        .next()
        .await?;

        if let Ok(existing_config) = existing_config {
            let signing_keypair = existing_config.signing_keypair;
            let revocation_commitment_parameters = existing_config.revocation_commitment_parameters;
            let range_proof_parameters = existing_config.range_proof_parameters;
            transaction.commit().await?;
            return Ok(zkabacus_crypto::merchant::Config::from_parts(
                signing_keypair,
                revocation_commitment_parameters,
                range_proof_parameters,
            ));
        }

        let new_config = zkabacus_crypto::merchant::Config::new(rng);

        sqlx::query!(
            r#"
            INSERT INTO
                config
            (signing_keypair, revocation_commitment_parameters, range_proof_parameters)
                VALUES
            (?, ?, ?)
            "#,
            new_config.signing_keypair(),
            new_config.revocation_commitment_parameters(),
            new_config.range_proof_parameters(),
        )
        .execute(&mut transaction)
        .await?;

        transaction.commit().await?;
        Ok(new_config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::SqlitePoolOptions;
    use zkabacus_crypto::internal::{
        test_new_nonce, test_new_revocation_lock, test_new_revocation_secret, test_verify_pair,
    };
    use zkabacus_crypto::Verification;

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
        assert_eq!(result.len(), 0,);

        let result = conn.insert_revocation(&lock1, Some(&secret1)).await?;
        assert_valid_pair(&result[0].0, &secret1);

        let result = conn.insert_revocation(&lock1, None).await?;
        assert_valid_pair(&result[0].0, &secret1);
        assert!(result[0].1.is_none(),);
        assert_valid_pair(&result[1].0, &secret1);
        assert!(result[1].1.is_some(),);
        assert_valid_pair(&lock1, result[1].1.as_ref().unwrap());
        assert_eq!(result.len(), 2);

        // Inserting a previously-unseen lock should not return any old pairs.
        let secret2 = test_new_revocation_secret(&mut rng);
        let lock2 = test_new_revocation_lock(&secret2);
        let result = conn.insert_revocation(&lock2, Some(&secret2)).await?;
        assert_eq!(result.len(), 0);

        Ok(())
    }
}
