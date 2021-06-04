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
