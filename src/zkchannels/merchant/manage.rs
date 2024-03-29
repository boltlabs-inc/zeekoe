use super::{database, Command};
use crate::{
    amount::Amount,
    database::ClosingBalances,
    escrow::types::ContractId,
    merchant::{
        cli::{List, Show},
        database::ChannelDetails,
        Config,
    },
    protocol::ChannelStatus,
};
use anyhow::Context;
use async_trait::async_trait;
use comfy_table::{Cell, Table};
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use zkabacus_crypto::ChannelId;

/// The contents of a row of the database for a particular channel that are suitable to share with
/// the user (especially for testing).
///
/// This should be a subset of [`ChannelDetails`].
#[serde_as]
#[derive(Debug, Serialize, Deserialize)]
pub struct PublicChannelDetails {
    #[serde_as(as = "DisplayFromStr")]
    channel_id: ChannelId,
    status: ChannelStatus,
    contract_id: ContractId,
    closing_balances: ClosingBalances,
}

impl From<ChannelDetails> for PublicChannelDetails {
    fn from(details: ChannelDetails) -> Self {
        PublicChannelDetails {
            status: details.status,
            channel_id: details.channel_id,
            contract_id: details.contract_id,
            closing_balances: details.closing_balances,
        }
    }
}

impl PublicChannelDetails {
    pub fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    pub fn status(&self) -> ChannelStatus {
        self.status
    }

    pub fn contract_id(&self) -> &ContractId {
        &self.contract_id
    }

    pub fn closing_balances(&self) -> &ClosingBalances {
        &self.closing_balances
    }
}

#[async_trait]
impl Command for List {
    type Output = String;

    async fn run(self, config: Config) -> Result<Self::Output, anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let channels = database.get_channels().await?;

        if self.json {
            Ok(serde_json::to_string(
                &channels
                    .into_iter()
                    .map(PublicChannelDetails::from)
                    .collect::<Vec<_>>(),
            )?)
        } else {
            let mut table = Table::new();
            table.load_preset(comfy_table::presets::UTF8_FULL);
            table.set_header(vec!["Channel ID", "Contract ID", "Status"]);

            for channel in channels {
                table.add_row(vec![
                    Cell::new(channel.channel_id),
                    Cell::new(channel.contract_id),
                    Cell::new(channel.status),
                ]);
            }
            Ok(table.to_string())
        }
    }
}

#[async_trait]
impl Command for Show {
    type Output = String;

    async fn run(self, config: Config) -> Result<Self::Output, anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;
        let details = database.get_channel_details_by_prefix(&self.prefix).await?;

        if self.json {
            Ok(serde_json::to_string(&PublicChannelDetails::from(details))?)
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
                Cell::new(Amount::from(details.merchant_deposit)),
            ]);
            table.add_row(vec![
                Cell::new("Customer Deposit"),
                Cell::new(Amount::from(details.customer_deposit)),
            ]);

            Ok(table.to_string())
        }
    }
}
