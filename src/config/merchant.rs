use {
    serde::{Deserialize, Serialize},
    std::{net::IpAddr, path::Path, path::PathBuf, time::Duration},
    url::Url,
};

pub use super::DatabaseLocation;

use crate::merchant::defaults;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
#[non_exhaustive]
pub struct Config {
    pub database: DatabaseLocation,
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

        // Adjust contained paths to be relative to the config path
        config.database = config.database.relative_to(config_dir);
        for service in config.services.as_mut_slice() {
            service.private_key = config_dir.join(&service.private_key);
            service.certificate = config_dir.join(&service.certificate);
        }

        Ok(config)
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
