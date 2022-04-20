pub mod customer;
pub mod merchant;
pub use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};

use anyhow::Context;
use std::{path::Path, sync::Arc};

pub async fn connect_sqlite<T: AsRef<Path>>(path: T) -> Result<Arc<SqlitePool>, anyhow::Error> {
    let options = SqliteConnectOptions::new()
        .create_if_missing(true)
        .filename(path.as_ref());

    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .with_context(|| {
            format!(
                "Could not open SQLite database at \"{}\"",
                path.as_ref().display()
            )
        })?;

    Ok(Arc::new(pool))
}
