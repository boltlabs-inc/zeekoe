use {
    dialectic_reconnect::Backoff,
    serde::{Deserialize, Serialize},
    std::{
        path::{Path, PathBuf},
        time::Duration,
    },
};

use http::Uri;

pub use super::{deserialize_confirmation_depth, deserialize_self_delay, DatabaseLocation};

use crate::{
    customer::defaults,
    escrow::types::{KeySpecifier, TezosKeyMaterial},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
#[non_exhaustive]
pub struct Config {
    pub database: Option<DatabaseLocation>,
    #[serde(default = "defaults::backoff")]
    pub backoff: Backoff,
    #[serde(with = "humantime_serde", default = "defaults::connection_timeout")]
    pub connection_timeout: Option<Duration>,
    #[serde(default = "defaults::daemon_port")]
    pub daemon_port: u16,
    #[serde(default = "defaults::max_pending_connection_retries")]
    pub max_pending_connection_retries: usize,
    #[serde(default = "defaults::message_timeout")]
    pub message_timeout: u64,
    #[serde(default = "defaults::approval_timeout")]
    pub approval_timeout: u64,
    #[serde(default = "defaults::max_message_length")]
    pub max_message_length: usize,
    #[serde(default = "defaults::max_note_length")]
    pub max_note_length: u64,
    #[serde(with = "http_serde::uri")]
    pub tezos_uri: Uri,
    pub tezos_account: KeySpecifier,
    #[serde(
        default = "defaults::self_delay",
        deserialize_with = "deserialize_self_delay"
    )]
    pub self_delay: u64,
    #[serde(
        default = "defaults::confirmation_depth",
        deserialize_with = "deserialize_confirmation_depth"
    )]
    pub confirmation_depth: u64,
    #[serde(default)]
    pub trust_certificate: Option<PathBuf>,
}

impl Config {
    pub async fn load(config_path: impl AsRef<Path>) -> Result<Config, anyhow::Error> {
        let mut config: Config = toml::from_str(&tokio::fs::read_to_string(&config_path).await?)?;

        // Directory containing the configuration path
        let config_dir = config_path
            .as_ref()
            .parent()
            .expect("Merchant configuration path must exist in some parent directory");

        if config.self_delay < 120 {
            eprintln!("Warning: `self_delay` should not be less than 120 outside of");
            eprintln!("testing. If this is an error, please update the customer");
            eprintln!("configuration.");
        }

        // Adjust contained paths to be relative to the config path
        config.database = config
            .database
            .map(|database| database.relative_to(&config_dir));
        config.trust_certificate = config
            .trust_certificate
            .map(|ref cert_path| config_dir.join(cert_path));
        config.tezos_account.set_relative_path(config_dir);

        Ok(config)
    }

    pub fn load_tezos_key_material(&self) -> anyhow::Result<TezosKeyMaterial> {
        Ok(TezosKeyMaterial::read_key_pair(&self.tezos_account)?)
    }
}
