use {async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::customer::{cli::Close, Config};

use super::{connect, database, Command};
use anyhow::Context;

#[async_trait]
impl Command for Close {
    #[allow(unused)]
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        if self.force {
            unilateral_close(&self, rng, config)
                .await
                .context("Unilateral close failed.")?;
        } else {
            mutual_close(&self, rng, config)
                .await
                .context("Mutual close failed.")?;
        }
        todo!()
    }
}

async fn unilateral_close(close: &Close, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
    todo!()
}

async fn mutual_close(close: &Close, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
    let database = database(&config)
        .await
        .context("Failed to connect to local database")?;

    // Look up the address and current local customer state for this merchant in the database
    let address = match database
        .channel_address(&close.label)
        .await
        .context("Failed to look up channel address in local database")?
    {
        None => return Err(anyhow::anyhow!("Unknown channel label: {}", close.label)),
        Some(address) => address,
    };

    // Connect and select the Close session
    let (_session_key, chan) = connect(&config, &address)
        .await
        .context("Failed to connect to merchant")?;

    let chan = chan
        .choose::<3>()
        .await
        .context("Failed selecting close session with merchant")?;

    todo!()
}
