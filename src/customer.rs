use std::str::FromStr;

use crate::{amount::Amount, transport::client::ZkChannelAddress};

pub use crate::config::customer as config;
pub use crate::defaults::customer as defaults;

pub fn pay(merchant: &ZkChannelAddress, pay: &Amount, note: &str) -> Result<(), anyhow::Error> {
    todo!()
}

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
