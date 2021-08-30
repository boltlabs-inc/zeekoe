//* Close functionalities for a merchant.
use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng};

use super::{database, Command};
use rand::SeedableRng;

use zeekoe::{
    abort,
    escrow::{tezos, types::TezosKeyMaterial},
    merchant::{
        cli,
        config::Service,
        database::{Error, QueryMerchant},
        server::SessionKey,
        Chan, Config,
    },
    offer_abort, proceed,
    protocol::{self, close, ChannelStatus, Party::Merchant},
};

use zkabacus_crypto::{
    merchant::Config as MerchantConfig, ChannelId, CloseState, CustomerBalance, MerchantBalance,
    RevocationLock, Verification,
};

use super::Method;

pub struct Close;

#[async_trait]
impl Method for Close {
    type Protocol = protocol::Close;

    async fn run(
        &self,
        _rng: StdRng,
        _client: &reqwest::Client,
        tezos_key_material: TezosKeyMaterial,
        _service: &Service,
        merchant_config: &MerchantConfig,
        database: &dyn QueryMerchant,
        _session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        // Run zkAbacus close and update channel status to PendingClose
        let (chan, close_state) = zkabacus_close(merchant_config, database, chan)
            .await
            .context("Mutual close failed")?;

        // Get contract ID for this channel
        let (contract_id, _) = database
            .contract_details(close_state.channel_id())
            .await
            .context(format!(
                "Failed to retrieve contract ID (id: {})",
                close_state.channel_id()
            ))?;

        // Generate an authorization signature under the merchant's EdDSA Tezos key
        let authorization_signature =
            tezos::close::authorize_mutual_close(&contract_id, &close_state, &tezos_key_material)
                .await
                .context("Failed to post mutualClose entrypoint")?;

        let chan = chan
            .send(authorization_signature)
            .await
            .context("Failed to send mutual close authorization signature")?;

        // Give the customer the opportunity to reject an invalid authorization signature
        offer_abort!(in chan as Merchant);

        // Close the dialectic channel.
        chan.close();

        Ok(())
    }
}

/// Process a customer close event.
///
/// **Usage**: this should be called after receiving a notification that a custClose entrypoint
/// call is confirmed on chain at any depth.
#[allow(unused)]
async fn process_customer_close(
    database: &dyn QueryMerchant,
    tezos_key_material: &TezosKeyMaterial,
    channel_id: &ChannelId,
    revocation_lock: &RevocationLock,
) -> Result<(), anyhow::Error> {
    // Retrieve current channel status.
    let current_status = database
        .get_channel_status(channel_id)
        .await
        .context(format!(
            "Failed to check channel status (id: {})",
            channel_id
        ))?;

    // Save the provided revocation lock (from the entrypoint call) and retrieve any existing
    // revocation secrets associated with it.
    let possible_secrets = database
        .insert_revocation(revocation_lock, None)
        .await
        .context(format!(
            "Failed to look up revocation lock (id: {})",
            channel_id
        ))?;

    // Get the first secret, if it exists.
    match possible_secrets.iter().flatten().next() {
        // If the lock *does not* have a revocation secret, update channel status to PendingClose.
        None => {
            database
                .compare_and_swap_channel_status(
                    channel_id,
                    &current_status,
                    &ChannelStatus::PendingClose,
                )
                .await
                .with_context(|| {
                    format!(
                        "Failed to update channel to PendingClose status (id: {})",
                        &channel_id
                    )
                })?;
            Ok(())
        }
        // If the lock already has a revocation secret, start the dispute process.
        Some(revocation_secret) => {
            // Update channel status to Dispute
            database
                .compare_and_swap_channel_status(
                    channel_id,
                    &current_status,
                    &ChannelStatus::Dispute,
                )
                .await
                .with_context(|| {
                    format!(
                        "Failed to update channel to Dispute status (id: {})",
                        &channel_id
                    )
                })?;

            // Retrieve contract ID
            let (contract_id, _) = database
                .contract_details(channel_id)
                .await
                .context("Failed to retrieve contract details")?;

            // Call the merchDispute entrypoint and wait for it to be confirmed
            let final_balances =
                tezos::close::merch_dispute(&contract_id, revocation_secret, tezos_key_material)
                    .await
                    .context(format!(
                        "Failed to post merchDispute entrypoint (id: {})",
                        &channel_id
                    ))?;

            // React to successfully confirmed dispute
            finalize_dispute(
                database,
                channel_id,
                final_balances.customer_balance(),
                final_balances.merchant_balance(),
            )
            .await
            .context(format!("Failed to finalize dispute (id: {})", channel_id))
        }
    }
}

/// Process a confirmed customer close event.
///
/// **Usage**: this should be called after receiving a notification that a custClose entrypoint
/// call is confirmed on chain *at the required confirmation depth*.
#[allow(unused)]
async fn finalize_customer_close(
    database: &dyn QueryMerchant,
    channel_id: &ChannelId,
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
) -> Result<(), anyhow::Error> {
    // Retrieve current channel status.
    let current_status = database
        .get_channel_status(channel_id)
        .await
        .context("Failed to check channel status")?;

    match current_status {
        // If database status is PendingClose, update channel status to Closed
        ChannelStatus::PendingClose => {
            database
                .compare_and_swap_channel_status(
                    channel_id,
                    &current_status,
                    &ChannelStatus::Closed,
                )
                .await
                .with_context(|| {
                    format!(
                        "Failed to update channel to Closed status (id: {})",
                        &channel_id
                    )
                })?;
            // Set final balances as specified by the custClose entrypoint call
            database
                .update_closing_balances(
                    channel_id,
                    &ChannelStatus::Closed,
                    merchant_balance,
                    Some(customer_balance),
                )
                .await
                .context(format!(
                    "Failed to save final balances for after successful close (id = {})",
                    channel_id
                ))
        }
        // If database status is Dispute, update merchant final balance to include the merchant
        // balance.
        ChannelStatus::Dispute => database
            .update_closing_balances(channel_id, &ChannelStatus::Dispute, merchant_balance, None)
            .await
            .context(format!(
                "Failed to save merchant's final balance for after successful dispute (id = {})",
                channel_id
            )),
        _ => Err(Error::UnexpectedChannelStatus {
            channel_id: *channel_id,
            expected: vec![ChannelStatus::PendingClose, ChannelStatus::Dispute],
            found: current_status,
        }
        .into()),
    }
}

/// Process a confirmed merchant dispute event.
///
/// **Usage**: this should be called after receiving a notification that a merchDispute
/// entrypoint call/operation is confirmed at the required confirmation depth.
#[allow(unused)]
async fn finalize_dispute(
    database: &dyn QueryMerchant,
    channel_id: &ChannelId,
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
) -> Result<(), anyhow::Error> {
    // Update channel status from Dispute to Closed.
    database
        .compare_and_swap_channel_status(
            channel_id,
            &ChannelStatus::Dispute,
            &ChannelStatus::Closed,
        )
        .await
        .context(format!(
            "Failed to update channel to Closed status (id: {})",
            &channel_id
        ))?;

    // Update final balances to indicate successful dispute (i.e., that the transfer of the
    // customer's balance to merchant is confirmed).
    // TODO: assert that customer/merchant balance matches expected pattern of 0/all?
    Ok(database
        .update_closing_balances(
            channel_id,
            &ChannelStatus::Closed,
            merchant_balance,
            Some(customer_balance),
        )
        .await
        .context(format!(
            "Failed to save final balances after successful dispute (id = {})",
            channel_id
        ))?)
}

// Process a mutual close event.
//
// **Usage**: this should be called after receiving a notification that a mutualClose entrypoint call/operation
// is confirmed to the required depth.
#[allow(unused)]
async fn finalize_mutual_close(
    merchant_config: &MerchantConfig,
    database: &dyn QueryMerchant,
    channel_id: &ChannelId,
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
) -> Result<(), anyhow::Error> {
    // Update database to indicate the channel closed successfully.
    database
        .compare_and_swap_channel_status(
            channel_id,
            &ChannelStatus::PendingClose,
            &ChannelStatus::Closed,
        )
        .await
        .context(format!(
            "Failed to update channel to Closed status (id: {})",
            &channel_id
        ))?;

    // Update database to final channel balances as indicated by the mutualClose entrypoint call.
    database
        .update_closing_balances(
            channel_id,
            &ChannelStatus::Closed,
            merchant_balance,
            Some(customer_balance),
        )
        .await
        .context(format!(
            "Failed to save final balances after successful mutual close (id = {})",
            channel_id
        ))?;

    Ok(())
}

/// Run the zkAbacus.Close protocol, including updating the database to PendingClose and validating
/// customer messages.
async fn zkabacus_close(
    merchant_config: &MerchantConfig,
    database: &dyn QueryMerchant,
    chan: Chan<close::CustomerSendSignature>,
) -> Result<(Chan<close::MerchantSendAuthorization>, CloseState), anyhow::Error> {
    // Receive close signature and state from customer.
    let (close_signature, chan) = chan
        .recv()
        .await
        .context("Failed to receive close state signature")?;

    let (close_state, chan) = chan.recv().await.context("Failed to receive close state")?;

    // Update database to indicate channel is now PendingClose.
    // Note: mutual close can only be called on an active channel. Any other state requires
    // a unilateral close.
    database
        .compare_and_swap_channel_status(
            close_state.channel_id(),
            &ChannelStatus::Active,
            &ChannelStatus::PendingClose,
        )
        .await
        .context(format!(
            "Failed to update channel to PendingClose status (id: {})",
            close_state.channel_id()
        ))?;

    // Confirm that customer sent a valid Pointcheval-Sanders signature under the merchant's
    // zkAbacus public key on the given close state.
    // If so, atomically check that the close state contains a fresh revocation lock and add it
    // to the database.
    // Otherwise, abort with an error.
    match merchant_config.check_close_signature(close_signature, &close_state) {
        Verification::Verified => {
            // Check that the revocation lock is fresh and insert
            if database
                .insert_revocation(close_state.revocation_lock(), None)
                .await
                .context("Failed to insert revocation lock into database")?
                .is_empty()
            {
                // If it's fresh, continue with protocol
                proceed!(in chan);
                Ok((chan, close_state))
            } else {
                // If it has been seen before, abort
                abort!(in chan return close::Error::KnownRevocationLock)
            }
        }
        // Abort if the close signature was invalid
        Verification::Failed => abort!(in chan return close::Error::InvalidCloseStateSignature),
    }
}

#[async_trait]
impl Command for cli::Close {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        // Retrieve zkAbacus config from the database
        let database = database(&config).await?;

        // Load Tezos keys from file
        let tezos_key_material = TezosKeyMaterial::read_key_pair(config.tezos_account.clone())?;

        // Either initialize the merchant's config afresh, or get existing config if it exists
        // (it should already exist)
        let merchant_config = database
            .fetch_or_create_config(&mut StdRng::from_entropy()) // TODO: allow determinism
            .await?;

        // Make sure exactly one correct command line option is satisfied
        match (self.channel, self.all) {
            (Some(channel_id), false) => {
                expiry(
                    &merchant_config,
                    database.as_ref(),
                    &channel_id,
                    &tezos_key_material,
                )
                .await
            }
            // TODO: iterate through database; call expiry for every channel
            (None, true) => Err(anyhow::anyhow!(
                "Closing all channels is not yet implemented."
            )),
            _ => unreachable!(),
        }
    }
}

/// Initiate close procedures with an expiry transaction.
///
/// **Usage**: this is called directly from the command line.
async fn expiry(
    _merchant_config: &MerchantConfig,
    database: &dyn QueryMerchant,
    channel_id: &ChannelId,
    tezos_key_material: &TezosKeyMaterial,
) -> Result<(), anyhow::Error> {
    // Retrieve current channel status
    let current_status = database
        .get_channel_status(channel_id)
        .await
        .context("Failed to retrieve current channel status")?;

    // Update database status to PendingClose
    database
        .compare_and_swap_channel_status(channel_id, &current_status, &ChannelStatus::PendingClose)
        .await
        .context(format!(
            "Failed to update channel to PendingClose status (id: {})",
            &channel_id
        ))?;

    // Retrieve contract details
    let (contract_id, _) = database
        .contract_details(channel_id)
        .await
        .context(format!(
            "Failed to retrieve contract details (id: {})",
            &channel_id
        ))?;

    // Call expiry entrypoint
    tezos::close::expiry(&contract_id, tezos_key_material)
        .await
        .context(format!(
            "Failed to initiate expiry close flow (id: {})",
            &channel_id
        ))
}

/// Claim the channel balances.
///
/// **Usage**: this is called in response to an on-chain event: when the expiry operation
/// is confirmed on chain _and_ the timelock period has passed without
/// any other operation to the contract (i.e., a custClose entrypoint call) confirmed on chain.
#[allow(unused)]
async fn claim_expiry_funds(
    database: &dyn QueryMerchant,
    channel_id: &ChannelId,
    tezos_key_material: &TezosKeyMaterial,
) -> Result<(), anyhow::Error> {
    // Assert database status is PendingClose
    let channel_status = database
        .get_channel_status(channel_id)
        .await
        .context("Failed to retrieve current channel status")?;
    if channel_status != ChannelStatus::PendingClose {
        return Err(Error::UnexpectedChannelStatus {
            channel_id: *channel_id,
            expected: vec![ChannelStatus::PendingClose],
            found: channel_status,
        }
        .into());
    }

    // Retrieve contract details
    let (contract_id, _) = database
        .contract_details(channel_id)
        .await
        .context(format!(
            "Failed to retrieve contract details (id: {})",
            &channel_id
        ))?;

    // Call merchClaim entrypoint and retrieve final channel balances
    let final_balances = tezos::close::merch_claim(&contract_id, tezos_key_material)
        .await
        .context(format!(
            "Failed to claim merchant funds (id: {})",
            &channel_id
        ))?;

    // React to successfully confirmed merchClaim
    finalize_expiry_close(
        database,
        channel_id,
        final_balances.merchant_balance(),
        final_balances.customer_balance(),
    )
    .await
}

/// Finalize the channel balances. This is called during a unilateral merchant close flow if the
/// customer does not call the custClose entrypoint and the merchClaim entrypoint is confirmed to
/// the required depth.
///
/// **Usage**: this is called after the merchClaim operation is confirmed on chain to an appropriate
/// depth.
#[allow(unused)]
async fn finalize_expiry_close(
    database: &dyn QueryMerchant,
    channel_id: &ChannelId,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
) -> Result<(), anyhow::Error> {
    // Update channel status to Closed
    database
        .compare_and_swap_channel_status(
            channel_id,
            &ChannelStatus::PendingClose,
            &ChannelStatus::Closed,
        )
        .await
        .context(format!(
            "Failed to update channel to Closed status (id: {})",
            channel_id
        ))?;

    // Indicate that all balances are paid out to the merchant
    Ok(database
        .update_closing_balances(
            channel_id,
            &ChannelStatus::Closed,
            merchant_balance,
            Some(customer_balance),
        )
        .await
        .context(format!(
            "Failed to save final balances after successful close (id = {})",
            channel_id
        ))?)
}
