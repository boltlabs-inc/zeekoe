use async_trait::async_trait;

use zeekoe::{
    merchant::{Chan, Config},
    protocol,
};

use super::Method;

pub struct Pay(());

#[async_trait]
impl Method for Pay {
    type Protocol = protocol::Pay;

    fn from_config(config: &Config) -> Self {
        todo!()
    }

    async fn run(&self, chan: Chan<Self::Protocol>) -> Result<(), anyhow::Error> {
        todo!()
    }
}
