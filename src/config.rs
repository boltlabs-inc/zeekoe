use {
    http::Uri,
    serde::{Deserialize, Serialize},
    std::path::PathBuf,
};

pub mod customer;
pub mod merchant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatabaseLocation {
    InMemory,
    Sqlite(PathBuf),
    #[serde(with = "http_serde::uri")]
    Postgres(Uri),
}

mod defaults {
    pub const fn max_pending_connection_retries() -> usize {
        4
    }

    pub const fn max_message_length() -> usize {
        1024 * 8
    }

    pub const fn port() -> u16 {
        2611
    }
}
