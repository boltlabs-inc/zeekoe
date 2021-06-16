use {async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::customer::{cli::Establish, Config};

use super::Command;

#[async_trait]
impl Command for Establish {
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
