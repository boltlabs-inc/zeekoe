use {
    http::Uri,
    serde::{Deserialize, Serialize},
    std::path::{Path, PathBuf},
};

pub mod customer;
pub mod merchant;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatabaseLocation {
    Ephemeral,
    Sqlite(PathBuf),
    #[serde(with = "http_serde::uri")]
    Postgres(Uri),
}

impl DatabaseLocation {
    pub fn relative_to(self, path: impl AsRef<Path>) -> Self {
        if let DatabaseLocation::Sqlite(db_path) = self {
            DatabaseLocation::Sqlite(path.as_ref().join(db_path))
        } else {
            self
        }
    }
}
