use async_trait::async_trait;

use zeekoe::customer::{cli::Establish, Config};

use super::Command;

#[async_trait]
impl Command for Establish {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
