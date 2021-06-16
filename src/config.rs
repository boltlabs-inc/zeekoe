use {
    http::Uri,
    serde::{Deserialize, Serialize},
};

pub mod customer;
pub mod merchant;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DatabaseLocation {
    InMemory,
    Sqlite(String),
    #[serde(with = "http_serde::uri")]
    Postgres(Uri),
}
