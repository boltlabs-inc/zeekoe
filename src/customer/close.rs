use async_trait::async_trait;

use crate::customer::{
    cli::{Close, Command},
    Config,
};

#[async_trait]
impl Command for Close {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
