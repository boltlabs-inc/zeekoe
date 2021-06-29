use {
    async_trait::async_trait,
    comfy_table::{Cell, Table},
    rand::rngs::StdRng,
    std::convert::TryInto,
};

use zeekoe::{
    amount::{Amount, XTZ},
    customer::{
        cli::{List, Rename},
        Config,
    },
};

use super::{database, Command};
use anyhow::Context;

#[async_trait]
impl Command for List {
    async fn run(self, _rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let channels = database.get_channels().await?;

        let mut table = Table::new();
        table.load_preset(comfy_table::presets::UTF8_FULL);
        table.set_header(vec![
            "Label",
            "State",
            "Balance",
            "Max Refund",
            "Channel ID",
        ]);

        // TODO: don't hard-code XTZ here, instead store currency in database
        let amount = |b: u64| Amount::from_minor_units_of_currency(b.try_into().unwrap(), XTZ);

        for details in channels {
            table.add_row(vec![
                Cell::new(details.label),
                Cell::new(details.state.state_name()),
                Cell::new(amount(details.state.customer_balance().into_inner())),
                Cell::new(amount(details.state.merchant_balance().into_inner())),
                Cell::new(details.state.channel_id()),
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
        database(&config)
            .await
            .context("Failed to connect to local database")?
            .relabel_channel(&self.old_label, &self.new_label)
            .await
            .context("Failed to rename channel")
    }
}
