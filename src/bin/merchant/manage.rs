use super::{database, Command};
use zeekoe::{
    amount::{Amount, XTZ},
    merchant::{
        cli::{List, Show},
        Config,
    },
};
use {
    anyhow::Context,
    async_trait::async_trait,
    comfy_table::{Cell, Table},
    std::convert::TryInto,
};

#[async_trait]
impl Command for List {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let channels = database.get_channels().await?;

        if config.json {
            println!("{}", serde_json::to_string(&channels).unwrap());
        } else {
            let mut table = Table::new();
            table.load_preset(comfy_table::presets::UTF8_FULL);
            table.set_header(vec!["Channel ID", "Status"]);

            for channel in channels {
                table.add_row(vec![
                    Cell::new(channel.channel_id),
                    Cell::new(channel.status),
                ]);
            }

            println!("{}", table);
        }
        Ok(())
    }
}

#[async_trait]
impl Command for Show {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let details = database.get_channel_details_by_prefix(&self.prefix).await?;

        // TODO: don't hard-code XTZ here, instead store currency in database
        let amount = |b: u64| Amount::from_minor_units_of_currency(b.try_into().unwrap(), XTZ);

        if config.json {
            println!("{}", serde_json::to_string(&details).unwrap());
        } else {
            let mut table = Table::new();
            table.load_preset(comfy_table::presets::UTF8_FULL);
            table.set_header(vec!["Key", "Value"]);
            table.add_row(vec![Cell::new("Channel ID"), Cell::new(details.channel_id)]);
            table.add_row(vec![Cell::new("Status"), Cell::new(details.status)]);
            table.add_row(vec![
                Cell::new("Contract ID"),
                Cell::new(details.contract_id),
            ]);
            table.add_row(vec![
                Cell::new("Merchant Deposit"),
                Cell::new(amount(details.merchant_deposit.into_inner())),
            ]);
            table.add_row(vec![
                Cell::new("Customer Deposit"),
                Cell::new(amount(details.customer_deposit.into_inner())),
            ]);

            println!("{}", table);
        }
        Ok(())
    }
}
