use {
    dialectic_reconnect::Backoff,
    serde::{Deserialize, Serialize},
    std::{path::Path, time::Duration},
};

pub use super::DatabaseLocation;

use crate::customer::defaults;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Config {
    pub database: Option<DatabaseLocation>,
    #[serde(default = "defaults::backoff")]
    pub backoff: Backoff,
    #[serde(with = "humantime_serde", default = "defaults::connection_timeout")]
    pub connection_timeout: Option<Duration>,
    #[serde(default = "defaults::max_pending_connection_retries")]
    pub max_pending_connection_retries: usize,
    #[serde(default = "defaults::max_message_length")]
    pub max_message_length: usize,
    #[serde(default = "defaults::max_note_length")]
    pub max_note_length: u64,
}

impl Config {
    pub async fn load(path: impl AsRef<Path>) -> Result<Config, anyhow::Error> {
        Ok(toml::from_str(&tokio::fs::read_to_string(path).await?)?)
    }
}
