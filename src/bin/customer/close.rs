use {async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::customer::{cli::Close, Config};

use super::Command;

#[async_trait]
impl Command for Close {
    #[allow(unused)]
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
