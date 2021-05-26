use {
    dialectic_reconnect::Backoff,
    serde::{Deserialize, Serialize},
    std::{path::PathBuf, time::Duration},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    database: PathBuf,
    backoff: Option<Backoff>,
    #[serde(with = "humantime_serde")]
    connection_timeout: Option<Duration>,
}
