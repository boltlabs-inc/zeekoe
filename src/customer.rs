use std::str::FromStr;

pub use crate::cli::{customer as cli, customer::Cli};
pub use crate::config::{customer as config, customer::Config};
pub use crate::database::customer as database;
pub use crate::defaults::customer as defaults;
pub use crate::transport::client::{self as client, Chan, Client};

#[derive(Debug)]
pub struct AccountName(String);

impl FromStr for AccountName {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(AccountName(s.to_string()))
    }
}

impl AccountName {
    pub fn new(name: String) -> Self {
        Self(name)
    }
}

#[derive(Debug)]
pub struct ChannelName(String);

impl FromStr for ChannelName {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ChannelName(s.to_string()))
    }
}

impl ChannelName {
    pub fn new(name: String) -> Self {
        Self(name)
    }
}
