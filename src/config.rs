use {
    http::Uri,
    serde::{de, Deserialize, Deserializer, Serialize},
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

pub fn deserialize_self_delay<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    let num = u64::deserialize(deserializer)?;

    if num < 10 {
        return Err(de::Error::invalid_value(
            de::Unexpected::Unsigned(num as u64),
            &"at least 10",
        ));
    }

    Ok(num)
}

pub fn deserialize_confirmation_depth<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<u64, D::Error> {
    let num = u64::deserialize(deserializer)?;

    if num < 1 {
        return Err(de::Error::invalid_value(
            de::Unexpected::Unsigned(num as u64),
            &"greater than 0",
        ));
    }

    Ok(num)
}
