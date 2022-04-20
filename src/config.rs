use http::Uri;
use serde::{de, Deserialize, Deserializer, Serialize};
use std::path::{Path, PathBuf};

pub mod customer;
pub mod merchant;

#[cfg(all(not(debug_assertions), feature = "allow_custom_self_delay"))]
compile_error!(
    "crate cannot be built for release with the `allow_custom_self_delay` feature enabled"
);

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

pub fn deserialize_self_delay<'de, D: Deserializer<'de>>(
    _deserializer: D,
) -> Result<u64, D::Error> {
    #[cfg(feature = "allow_custom_self_delay")]
    {
        let num = u64::deserialize(_deserializer)?;
        if num < 10 {
            return Err(de::Error::invalid_value(
                de::Unexpected::Unsigned(num as u64),
                &"at least 10",
            ));
        }
        Ok(num)
    }

    #[cfg(not(feature = "allow_custom_self_delay"))]
    {
        eprintln!(
            "Ignoring explicitly specified self-delay value because \
            this binary was built to only use the default value (24 hours)"
        );
        Ok(crate::defaults::shared::self_delay())
    }
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
