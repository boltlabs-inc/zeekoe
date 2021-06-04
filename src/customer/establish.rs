use async_trait::async_trait;

use crate::customer::{
    cli::{Command, Establish},
    Config,
};

#[async_trait]
impl Command for Establish {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
