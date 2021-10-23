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
use serde_json::json;

#[async_trait]
impl Command for List {
    async fn run(self, _rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let channels = database.get_channels().await?;

        // TODO: don't hard-code XTZ here, instead store currency in database
        let amount = |b: u64| Amount::from_minor_units_of_currency(b.try_into().unwrap(), XTZ);

        if self.json {
            let mut output = Vec::new();
            for details in channels {
                output.push(json!({
                    "label": details.label,
                    "state": details.state.state_name(),
                    "balance": format!("{}", amount(details.state.customer_balance().into_inner())),
                    "max_refund": format!("{}", amount(details.state.merchant_balance().into_inner())),
                    "channel_id": format!("{}", details.state.channel_id()),
                }));
            }
            println!("{}", json!(output).to_string());
        } else {
            let mut table = Table::new();
            table.load_preset(comfy_table::presets::UTF8_FULL);
            table.set_header(vec![
                "Label",
                "State",
                "Balance",
                "Max Refund",
                "Channel ID",
            ]);

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
        }
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
            .rename_channel(&self.old_label, &self.new_label)
            .await
            .context("Failed to rename channel")
    }
}
