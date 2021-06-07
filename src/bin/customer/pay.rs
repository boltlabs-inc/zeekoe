use async_trait::async_trait;

use zeekoe::customer::{
    cli::{Pay, Refund},
    Config,
};

use super::Command;

#[async_trait]
impl Command for Pay {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}

#[async_trait]
impl Command for Refund {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
