use {
    http::Uri,
    serde::{Deserialize, Serialize},
    std::{net::IpAddr, path::Path, path::PathBuf, time::Duration},
    url::Url,
};

pub use super::{deserialize_confirmation_depth, deserialize_self_delay, DatabaseLocation};

use crate::{
    escrow::types::{KeySpecifier, TezosKeyMaterial},
    merchant::defaults,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
#[non_exhaustive]
pub struct Config {
    pub database: DatabaseLocation,
    pub tezos_account: KeySpecifier,
    #[serde(with = "http_serde::uri")]
    pub tezos_uri: Uri,
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
    #[serde(rename = "service")]
    pub services: Vec<Service>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
#[non_exhaustive]
pub struct Service {
    #[serde(default = "defaults::address")]
    pub address: IpAddr,
    #[serde(default = "defaults::port")]
    pub port: u16,
    #[serde(with = "humantime_serde", default)]
    pub connection_timeout: Option<Duration>,
    #[serde(default = "defaults::max_pending_connection_retries")]
    pub max_pending_connection_retries: usize,
    #[serde(default = "defaults::message_timeout")]
    pub message_timeout: u64,
    #[serde(default = "defaults::transaction_timeout")]
    pub transaction_timeout: u64,
    #[serde(default = "defaults::max_message_length")]
    pub max_message_length: usize,
    #[serde(default)]
    pub approve: Approver,
    pub private_key: PathBuf,
    pub certificate: PathBuf,
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
            eprintln!("testing. If this is an error, please update the merchant");
            eprintln!("configuration.");
        }

        // Adjust contained paths to be relative to the config path
        config.database = config.database.relative_to(config_dir);
        config.tezos_account.set_relative_path(config_dir);
        for service in config.services.as_mut_slice() {
            service.private_key = config_dir.join(&service.private_key);
            service.certificate = config_dir.join(&service.certificate);
        }

        Ok(config)
    }

    pub fn load_tezos_key_material(&self) -> Result<TezosKeyMaterial, anyhow::Error> {
        Ok(TezosKeyMaterial::read_key_pair(&self.tezos_account)?)
    }
}

impl Service {
    pub fn message_timeout(&self) -> Duration {
        Duration::from_secs(self.message_timeout)
    }

    pub fn transaction_timeout(&self) -> Duration {
        Duration::from_secs(self.transaction_timeout)
    }
}

/// A description of how to approve payments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approver {
    /// Approve all non-negative payments.
    Automatic,
    /// Request approval from an external service at the URL, via a `GET` request containing the
    /// transaction amount in the query string and the transaction note in the body of the request.
    ///
    /// An external approver is considered to approve a transaction if it returns an "Ok 200" code,
    /// and otherwise to disapprove it. The body of the approver's response is forwarded to the
    /// customer.
    Url(Url),
}

impl Default for Approver {
    fn default() -> Self {
        Approver::Automatic
    }
}
