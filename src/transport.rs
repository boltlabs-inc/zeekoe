use crate::customer;
use crate::transport::client::Address;
use http::uri::InvalidUri;
use http::Uri;
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;
use thiserror::Error;
use webpki::{DnsName, DnsNameRef, InvalidDnsNameError};

use transport::client;

/// The address of a zkChannels merchant: a URI of the form `zkchannel://some.domain.com:2611` with
/// an optional port number.
#[derive(Debug, Clone, serde_with::SerializeDisplay, serde_with::DeserializeFromStr)]
pub struct ZkChannelAddress {
    host: DnsName,
    port: Option<u16>,
}

impl Address for ZkChannelAddress {
    fn get_host(&self) -> &DnsName {
        &self.host
    }

    fn get_port(&self) -> u16 {
        self.port.unwrap_or_else(customer::defaults::port)
    }
}

zkabacus_crypto::impl_sqlx_for_bincode_ty!(ZkChannelAddress);

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InvalidZkChannelAddress {
    #[error("Incorrect URI scheme: expecting `zkchannel://`")]
    IncorrectScheme,
    #[error("Unexpected non-root path in `zkchannel://` address")]
    UnsupportedPath,
    #[error("Unexpected query string in `zkchannel://` address")]
    UnsupportedQuery,
    #[error("Missing hostname in `zkchannel://` address")]
    MissingHost,
    #[error("Invalid DNS hostname in `zkchannel://` address: {0}")]
    InvalidDnsName(InvalidDnsNameError),
    #[error("Invalid `zkchannel://` address: {0}")]
    InvalidUri(InvalidUri),
}

impl FromStr for ZkChannelAddress {
    type Err = InvalidZkChannelAddress;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uri: Uri = s.parse().map_err(InvalidZkChannelAddress::InvalidUri)?;
        if uri.scheme_str() != Some("zkchannel") {
            Err(InvalidZkChannelAddress::IncorrectScheme)
        } else if uri.path() != "" && uri.path() != "/" {
            Err(InvalidZkChannelAddress::UnsupportedPath)
        } else if uri.query().is_some() {
            Err(InvalidZkChannelAddress::UnsupportedQuery)
        } else if let Some(host) = uri.host() {
            Ok(ZkChannelAddress {
                host: DnsNameRef::try_from_ascii_str(host)
                    .map_err(InvalidZkChannelAddress::InvalidDnsName)?
                    .to_owned(),
                port: uri.port_u16(),
            })
        } else {
            Err(InvalidZkChannelAddress::MissingHost)
        }
    }
}

impl Display for ZkChannelAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let host: &str = self.host.as_ref().into();
        write!(f, "zkchannel://{}", host)?;
        if let Some(port) = self.port {
            write!(f, ":{}", port)?;
        }
        Ok(())
    }
}
