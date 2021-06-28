use {async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::customer::{
    cli::{List, Rename},
    Config,
};

use super::Command;

#[async_trait]
impl Command for List {
    #[allow(unused)]
    async fn run(self, rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}

#[async_trait]
impl Command for Rename {
    #[allow(unused)]
    async fn run(self, rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
