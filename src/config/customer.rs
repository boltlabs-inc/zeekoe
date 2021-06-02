use {
    dialectic_reconnect::Backoff,
    serde::{Deserialize, Serialize},
    std::time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    database: super::DatabaseLocation,
    #[serde(default = "defaults::backoff")]
    backoff: Backoff,
    #[serde(with = "humantime_serde", default = "defaults::connection_timeout")]
    connection_timeout: Option<Duration>,
    #[serde(default = "super::defaults::max_pending_connection_retries")]
    max_pending_connection_retries: usize,
    #[serde(default = "super::defaults::max_message_length")]
    max_message_length: usize,
}

mod defaults {
    use super::*;

    pub fn backoff() -> Backoff {
        Backoff::with_delay(Duration::from_secs(1))
    }

    pub const fn connection_timeout() -> Option<Duration> {
        Some(Duration::from_secs(60))
    }
}
