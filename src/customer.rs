use serde::{Deserialize, Serialize};
use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

pub use crate::{
    cli::{customer as cli, customer::Cli},
    config::{customer as config, customer::Config},
    database::customer as database,
    defaults::customer as defaults,
    zkchannels::customer as zkchannels,
};
pub use transport::{
    client::{self, Chan, Client},
    server::{self, Server},
};

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
