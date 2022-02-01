use {
    anyhow::Context,
    async_trait::async_trait,
    comfy_table::{Cell, Table},
    rand::rngs::StdRng,
    serde::{Deserialize, Serialize},
    serde_json::json,
    serde_with::{serde_as, DisplayFromStr},
    zkabacus_crypto::{ChannelId, CustomerBalance, MerchantBalance},
};

use crate::{
    amount::Amount,
    customer::{
        cli::{List, Rename, Show},
        ChannelName, Config,
    },
    database::customer::{ChannelDetails, StateName},
    escrow::types::ContractId,
};

use super::{database, Command};

/// The contents of a row of the database for a particular channel that are suitable to share with
/// the user.
///
/// This should be a subset of the [`ChannelDetails`].
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct PublicChannelDetails {
    label: ChannelName,
    state: StateName,
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
    #[serde_as(as = "DisplayFromStr")]
    channel_id: ChannelId,
    contract_id: Option<ContractId>,
}

impl From<ChannelDetails> for PublicChannelDetails {
    fn from(details: ChannelDetails) -> Self {
        PublicChannelDetails {
            label: details.label,
            state: details.state.state_name(),
            customer_balance: details.state.customer_balance(),
            merchant_balance: details.state.merchant_balance(),
            channel_id: *details.state.channel_id(),
            contract_id: details.contract_details.contract_id,
        }
    }
}

#[async_trait]
impl Command for Show {
    type Output = String;
    async fn run(self, _rng: StdRng, config: self::Config) -> Result<Self::Output, anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let channel_details = database.get_channel(&self.label).await?;

        if self.json {
            Ok(serde_json::to_string(&PublicChannelDetails::from(
                channel_details,
            ))?)
        } else {
            todo!("non-JSON show is not yet implemented")
        }
    }
}

#[async_trait]
impl Command for List {
    type Output = ();

    async fn run(self, _rng: StdRng, config: self::Config) -> Result<Self::Output, anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let channels = database.get_channels().await?;

        if self.json {
            let mut output = Vec::new();
            for details in channels {
                output.push(json!({
                    "label": details.label,
                    "state": details.state.state_name(),
                    "balance": format!("{}", Amount::from(details.state.customer_balance())),
                    "max_refund": format!("{}", Amount::from(details.state.merchant_balance())),
                    "channel_id": format!("{}", details.state.channel_id()),
                    "contract_id": details.contract_details.contract_id.map_or_else(|| "N/A".to_string(), |contract_id| format!("{}", contract_id)),
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
                "Contract ID",
            ]);

            for details in channels {
                table.add_row(vec![
                    Cell::new(details.label),
                    Cell::new(details.state.state_name()),
                    Cell::new(Amount::from(details.state.customer_balance())),
                    Cell::new(Amount::from(details.state.merchant_balance())),
                    Cell::new(details.state.channel_id()),
                    Cell::new(details.contract_details.contract_id.map_or_else(
                        || "N/A".to_string(),
                        |contract_id| format!("{}", contract_id),
                    )),
                ]);
            }

            println!("{}", table);
        }
        Ok(())
    }
}

#[async_trait]
impl Command for Rename {
    type Output = ();

    async fn run(self, _rng: StdRng, config: self::Config) -> Result<Self::Output, anyhow::Error> {
        database(&config)
            .await
            .context("Failed to connect to local database")?
            .rename_channel(&self.old_label, &self.new_label)
            .await
            .context("Failed to rename channel")
    }
}
