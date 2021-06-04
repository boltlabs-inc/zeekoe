use async_trait::async_trait;

use crate::customer::{
    cli::{Command, Import, List, Remove, Rename},
    Config,
};

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
