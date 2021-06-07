use async_trait::async_trait;

use zeekoe::{
    merchant::{Chan, Config},
    protocol,
};

use super::Method;

pub struct Parameters(());

#[async_trait]
impl Method for Parameters {
    type Protocol = protocol::Parameters;

    async fn run(&self, config: &Config, chan: Chan<Self::Protocol>) -> Result<(), anyhow::Error> {
        todo!()
    }
}
