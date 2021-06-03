use {
    dialectic_reconnect::Backoff,
    serde::{Deserialize, Serialize},
    std::{fs, path::Path, time::Duration},
};

use crate::customer::defaults;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    database: Option<super::DatabaseLocation>,
    #[serde(default = "defaults::backoff")]
    backoff: Backoff,
    #[serde(with = "humantime_serde", default = "defaults::connection_timeout")]
    connection_timeout: Option<Duration>,
    #[serde(default = "defaults::max_pending_connection_retries")]
    max_pending_connection_retries: usize,
    #[serde(default = "defaults::max_message_length")]
    max_message_length: usize,
}

pub fn load(path: impl AsRef<Path>) -> Result<Config, anyhow::Error> {
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}
