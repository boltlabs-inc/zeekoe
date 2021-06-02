use {
    serde::{Deserialize, Serialize},
    std::{net::IpAddr, path::PathBuf, time::Duration},
    url::Url,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    database: super::DatabaseLocation,
    #[serde(rename = "service")]
    services: Vec<Service>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Service {
    #[serde(default = "defaults::address")]
    address: IpAddr,
    #[serde(default = "super::defaults::port")]
    port: u16,
    #[serde(with = "humantime_serde")]
    connection_timeout: Option<Duration>,
    #[serde(default = "super::defaults::max_pending_connection_retries")]
    max_pending_connection_retries: usize,
    #[serde(default = "super::defaults::max_message_length")]
    max_message_length: usize,
    approve: Approver,
    private_key: PathBuf,
    certificate: PathBuf,
}

/// Defaults for the optional fields in the configuration.
mod defaults {
    use std::net::{IpAddr, Ipv6Addr};

    pub const fn address() -> IpAddr {
        IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 0x1))
    }
}

/// A description of how to approve payments.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
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
