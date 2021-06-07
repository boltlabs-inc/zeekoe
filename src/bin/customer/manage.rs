use async_trait::async_trait;

use zeekoe::customer::{
    cli::{Import, List, Remove, Rename},
    Config,
};

use super::Command;

#[async_trait]
impl Command for Import {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}

#[async_trait]
impl Command for Remove {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}

#[async_trait]
impl Command for List {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}

#[async_trait]
impl Command for Rename {
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
