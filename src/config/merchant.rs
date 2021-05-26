use {
    serde::{Deserialize, Serialize},
    std::{net::IpAddr, time::Duration},
    url::Url,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    services: Vec<SingleConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SingleConfig {
    address: IpAddr,
    port: u16,
    #[serde(with = "humantime_serde")]
    connection_timeout: Option<Duration>,
    approve: Approver,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Approver {
    Automatic,
    External { url: Url },
}
