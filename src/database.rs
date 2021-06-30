pub mod customer;
pub mod merchant;
pub use sqlx::sqlite::{SqlitePool, SqlitePoolOptions, SqliteRow};

use {anyhow::Context, std::fs::File, std::path::Path, std::sync::Arc};

pub async fn connect_sqlite(path: &Path) -> Result<Arc<SqlitePool>, anyhow::Error> {
    let uri = path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid UTF-8 in SQLite database path {:?}", path))?;

    if !path.exists() {
        // Create a blank sqlite db aka an empty file.
        let file = File::create(path)?;
        file.sync_all()?;
    }

    Ok(Arc::new(SqlitePool::connect(uri).await.with_context(
        || format!("Could not open SQLite database at \"{}\"", uri),
    )?))
}
