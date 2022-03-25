use {
    serde::{Deserialize, Serialize},
    std::{
        fmt::{Display, Formatter},
        str::FromStr,
    },
};

pub use crate::cli::{customer as cli, customer::Cli};
pub use crate::config::{customer as config, customer::Config};
pub use crate::database::customer as database;
pub use crate::defaults::customer as defaults;
pub use crate::zkchannels::customer as zkchannels;
pub use transport::client::{self as client, Chan, Client};
pub use transport::server::{self as server, Server};

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize)]
#[sqlx(transparent)]
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

impl Display for AccountName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, sqlx::Type, Serialize, Deserialize)]
#[sqlx(transparent)]
pub struct ChannelName(String);

impl FromStr for ChannelName {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(ChannelName(s.to_string()))
    }
}

impl Display for ChannelName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl ChannelName {
    pub fn new(name: String) -> Self {
        Self(name)
    }
}
