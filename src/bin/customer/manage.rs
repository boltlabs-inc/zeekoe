use {
    async_trait::async_trait,
    comfy_table::{Cell, Table},
    rand::rngs::StdRng,
};

use zeekoe::customer::{
    cli::{List, Rename},
    Config,
};

use super::{database, Command};
use anyhow::Context;

#[async_trait]
impl Command for List {
    async fn run(self, rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let channels = database.get_channels().await?;

        let mut table = Table::new();
        table.set_header(vec![
            "Label",
            "Address",
            "State",
            "Customer Deposit",
            "Merchant Deposit",
        ]);

        for (label, state, address, customer_deposit, merchant_deposit) in channels {
            table.add_row(vec![
                Cell::new(label),
                Cell::new(address),
                Cell::new(state.state_name()),
                Cell::new(customer_deposit.into_inner()),
                Cell::new(merchant_deposit.into_inner()),
            ]);
        }

        println!("{}", table);
        Ok(())
    }
}

#[async_trait]
impl Command for Rename {
    #[allow(unused)]
    async fn run(self, rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        todo!()
    }
}
