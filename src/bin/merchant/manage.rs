use super::{database, Command};
use zeekoe::merchant::{
    cli::{List, Show},
    Config,
};
use {
    anyhow::Context,
    async_trait::async_trait,
    comfy_table::{Cell, Table},
};

#[async_trait]
impl Command for List {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let channels = database.get_channels().await?;

        let mut table = Table::new();
        table.set_header(vec!["Channel ID", "Status"]);

        for (channel_id, channel_status) in channels {
            table.add_row(vec![Cell::new(channel_id), Cell::new(channel_status)]);
        }

        println!("{}", table);
        Ok(())
    }
}

#[async_trait]
impl Command for Show {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let details = database.get_channel_details(&self.prefix).await?;

        let mut table = Table::new();
        table.set_header(vec!["Key", "Value"]);
        table.add_row(vec![Cell::new("Channel ID"), Cell::new(details.channel_id)]);
        table.add_row(vec![Cell::new("Status"), Cell::new(details.status)]);
        table.add_row(vec![
            Cell::new("Contract ID"),
            Cell::new(details.contract_id),
        ]);
        table.add_row(vec![
            Cell::new("Merchant Deposit"),
            Cell::new(details.merchant_deposit.into_inner()),
        ]);
        table.add_row(vec![
            Cell::new("Customer Deposit"),
            Cell::new(details.customer_deposit.into_inner()),
        ]);

        println!("{}", table);
        Ok(())
    }
}
