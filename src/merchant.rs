use async_trait::async_trait;

pub use crate::cli::{merchant as cli, merchant::Merchant as Cli};
pub use crate::config::{merchant as config, merchant::Config};
pub use crate::defaults::merchant as defaults;
pub use crate::transport::server::{self as server, Chan, Server};

use crate::{
    merchant::cli::{Command, Run},
    protocol,
};

#[async_trait]
impl Command for Run {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}

pub async fn pay(config: &Config, chan: Chan<protocol::Pay>) -> Result<(), anyhow::Error> {
    todo!()
}
