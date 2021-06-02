use crate::database::SqlitePool;
use async_trait::async_trait;
use futures::stream::TryStreamExt;
use zkabacus_crypto::Nonce;

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
        revocation: (&str, Option<&str>),
    ) -> sqlx::Result<Vec<(String, Option<String>)>>;
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
        revocation: (&str, Option<&str>),
    ) -> sqlx::Result<Vec<(String, Option<String>)>> {
        let existing_pairs = sqlx::query!(
            "SELECT lock, secret FROM revocations WHERE lock = ?",
            revocation.0
        )
        .fetch(self)
        .map_ok(|rev| (rev.lock, rev.secret))
        .try_collect()
        .await?;

        sqlx::query!(
            "INSERT INTO revocations (lock, secret) VALUES (?, ?)",
            revocation.0,
            revocation.1
        )
        .execute(self)
        .await?;

        Ok(existing_pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::SqlitePoolOptions;

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

        let nonce = Nonce::new(&mut rng);
        assert!(conn.insert_nonce(&nonce).await?);
        assert!(!conn.insert_nonce(&nonce).await?);

        let nonce2 = Nonce::new(&mut rng);
        assert!(conn.insert_nonce(&nonce2).await?);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_insert_revocation() -> Result<(), anyhow::Error> {
        let conn = create_migrated_db().await?;
        assert_eq!(conn.insert_revocation(("test-lock-1", None)).await?, []);

        assert_eq!(
            conn.insert_revocation(("test-lock-1", Some("test-secret-1")))
                .await?,
            [("test-lock-1".to_string(), None)]
        );

        assert_eq!(
            conn.insert_revocation(("test-lock-1", None)).await?,
            [
                ("test-lock-1".into(), None),
                ("test-lock-1".into(), Some("test-secret-1".into()))
            ]
        );

        assert_eq!(conn.insert_revocation(("test-lock-2", None)).await?, []);
        Ok(())
    }
}
