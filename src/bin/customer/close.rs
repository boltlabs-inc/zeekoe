use {async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::customer::{cli::Close, Config};

use super::Command;

#[async_trait]
impl Command for Close {
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        if self.force {
            unilateral_close()
                .await
                .context("Unilateral close failed.")?;
        } else {
            mutual_close().await.context("Mutual close failed.")?;
        }
        todo!()
    }
}

async fn unilateral_close() -> Result<(), Error> {
    todo!()
}

async fn mutual_close() -> Result<(), Error> {
    // Connect and select the Close session
    let (session_key, chan) = connect(&config, address)
        .await
        .context("Failed to connect to merchant")?;

    let chan = chan
        .choose::<3>()
        .await
        .context("Failed selecting close session with merchant")?;

    todo!()
}
