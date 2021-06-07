use async_trait::async_trait;

use zeekoe::customer::{cli::Close, Config};

use super::Command;

#[async_trait]
impl Command for Close {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
