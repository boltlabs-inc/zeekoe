pub mod customer;
pub mod merchant;
pub use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};
use zkabacus_crypto::{CustomerBalance, MerchantBalance};

/// The balances of a channel at closing. These may change during a close flow.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
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
