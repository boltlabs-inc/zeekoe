use {
    dialectic_reconnect::Backoff,
    serde::{Deserialize, Serialize},
    std::{
        path::{Path, PathBuf},
        time::Duration,
    },
    tezedge::PrivateKey,
};

pub use super::DatabaseLocation;

use crate::customer::defaults;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
#[non_exhaustive]
pub struct Config {
    pub database: Option<DatabaseLocation>,
    #[serde(default = "defaults::backoff")]
    pub backoff: Backoff,
    #[serde(with = "humantime_serde", default = "defaults::connection_timeout")]
    pub connection_timeout: Option<Duration>,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default = "defaults::max_pending_connection_retries")]
    pub max_pending_connection_retries: usize,
    #[serde(default = "defaults::max_message_length")]
    pub max_message_length: usize,
    #[serde(default = "defaults::max_note_length")]
    pub max_note_length: u64,
    pub private_key: PathBuf,
    #[serde(default)]
    pub trust_certificate: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
#[non_exhaustive]
pub struct DaemonConfig {
    #[serde(default = "defaults::daemon_port")]
    pub port: u16,
    #[serde(default = "defaults::daemon_backoff_max_retries")]
    pub max_retries: usize,
    #[serde(default = "defaults::daemon_backoff_delay")]
    pub retry_delay: Duration,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            port: Default::default(),
            max_retries: defaults::daemon_backoff_max_retries(),
            retry_delay: defaults::daemon_backoff_delay(),
        }
    }
}

impl Config {
    pub async fn load(config_path: impl AsRef<Path>) -> Result<Config, anyhow::Error> {
        let mut config: Config = toml::from_str(&tokio::fs::read_to_string(&config_path).await?)?;

        // Directory containing the configuration path
        let config_dir = config_path
            .as_ref()
            .parent()
            .expect("Merchant configuration path must exist in some parent directory");

        // Adjust contained paths to be relative to the config path
        config.database = config
            .database
            .map(|database| database.relative_to(&config_dir));
        config.trust_certificate = config
            .trust_certificate
            .map(|ref cert_path| config_dir.join(cert_path));

        Ok(config)
    }

    pub async fn load_private_key(&self) -> anyhow::Result<PrivateKey> {
        let contents = tokio::fs::read_to_string(&self.private_key).await?;
        Ok(PrivateKey::from_base58check(&contents)?)
    }
}
