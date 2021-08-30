//* Close functionalities for a customer.
//*
//* In the current design, the customer requires either a watchtower or a notification service
//* to finalize the channel close.
//* This architecture is flexible; we could alternately allow the customer CLI to wait (hang) until
//* it receives confirmation (e.g. call `process_mutual_close_confirmation` directly from
//* `mutual_close()`).
use {
    async_trait::async_trait,
    rand::rngs::StdRng,
    serde::Serialize,
    std::{convert::Infallible, fs::File, path::PathBuf},
    thiserror::Error,
};

use zeekoe::{
    abort,
    customer::{
        cli::Close,
        database::{zkchannels_state, QueryCustomer, QueryCustomerExt, State},
        Chan, ChannelName, Config,
    },
    escrow::{self, tezos, types::TezosKeyMaterial},
    offer_abort, proceed,
    protocol::{close, Party::Customer},
};
use zkabacus_crypto::{
    customer::ClosingMessage, ChannelId, CloseState, CloseStateSignature, CustomerBalance,
    MerchantBalance, RevocationLock,
};

use super::{connect, connect_daemon, database, Command};
use anyhow::Context;

#[derive(Debug, Error)]
enum Error {
    #[error("Cannot initiate close because there are no stored contract details.")]
    NoContractDetails(ChannelName),
}

#[async_trait]
impl Command for Close {
    #[allow(unused)]
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        let tezos_key_material = config
            .load_tezos_key_material()
            .await
            .context("Failed to load Tezos key material")?;

        if self.force {
            unilateral_close(&self, rng, config, tezos_key_material)
                .await
                .context("Unilateral close failed")?;
        } else {
            mutual_close(&self, rng, config, tezos_key_material)
                .await
                .context("Mutual close failed")?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
struct Closing {
    channel_id: ChannelId,
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
    closing_signature: CloseStateSignature,
    revocation_lock: RevocationLock,
}

/// Initiate channel closure on the current balances as part of a unilateral customer or a
/// unilateral merchant close.
///
/// **Usage**: This function can be called
/// - directly from the command line to initiate unilateral customer channel closure.
/// - in response to a unilateral merchant close: upon receipt of a notification that an
/// operation calling the expiry entrypoint is confirmed on chain at any depth.
async fn unilateral_close(
    close: &Close,
    rng: StdRng,
    config: self::Config,
    tezos_key_material: TezosKeyMaterial,
) -> Result<(), anyhow::Error> {
    let database = database(&config)
        .await
        .context("Failed to connect to local database")?;

    // Read the closing message without changing the database state
    let close_message = get_close_message(rng, database.as_ref(), &close.label)
        .await
        .context("Failed to fetch closing message from database")?;

    if !close.off_chain {
        // Only correct the balances if the customer balance is non-zero.
        if close_message.customer_balance().into_inner() != 0 {
            let contract_id = match database
                .contract_details(&close.label)
                .await
                .context(format!("Failed to retrieve contract for {}", &close.label))?
                .contract_id
            {
                Some(contract_id) => contract_id,
                None => return Err(Error::NoContractDetails(close.label.clone()).into()),
            };

            // Call the custClose entrypoint and wait for it to be confirmed on chain.
            tezos::close::cust_close(&contract_id, &close_message, &tezos_key_material)
                .await
                .context("Failed to post custClose transaction")?;

            // React to a successfully posted custClose.
            finalize_customer_close(
                database.as_ref(),
                &close.label,
                *close_message.merchant_balance(),
            )
            .await?;
        }
    } else {
        // TODO: Print out custClose transaction
        // Wait for customer confirmation that it posted

        let closing = Closing {
            merchant_balance: *close_message.merchant_balance(),
            customer_balance: *close_message.customer_balance(),
            closing_signature: close_message.closing_signature().clone(),
            revocation_lock: close_message.revocation_lock().clone(),
            channel_id: *close_message.channel_id(),
        };
        write_close_json(&closing)?;
    }

    // Notify the on-chain monitoring daemon this channel has started to close.
    // TODO: Do we need to alert the polling service about the new timeout potential?
    refresh_daemon(&config).await
}

/// Update channel balances when merchant receives payout in unilateral close flows.
///
/// **Usage**: this function is called when the
/// custClose entrypoint call/operation is confirmed on chain at an appropriate depth.
#[allow(unused)]
async fn finalize_customer_close(
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
    merchant_balance: MerchantBalance,
) -> anyhow::Result<()> {
    // TODO: assert that the db status is PendingClose,

    // Indicate that the merchant balance has been paid out to the merchant.
    database
        .update_closing_balances(channel_name, merchant_balance, None)
        .await
        .context(format!(
            "Failed to save channel balances for {} after successful close",
            channel_name
        ))?;

    Ok(())
}

/// Claim final balance of the channel via the custClaim entrypoint.
///
/// **Usage**: this function is called when
/// the contract's customer claim delay has passed *and* the custClose entrypoint call/operation
/// is confirmed on chain at any depth.
#[allow(unused)]
async fn claim_funds(
    database: &dyn QueryCustomer,
    close: &Close,
    config: self::Config,
    customer_key_material: TezosKeyMaterial,
) -> Result<(), anyhow::Error> {
    // Retrieve channel information.
    let channel_details = database.get_channel(&close.label).await.context(format!(
        "Failed to retrieve channel details to claim funds for {}",
        close.label.clone()
    ))?;

    // if database status is PendingClose, call the custClaim entrypoint.
    match channel_details.status {
        State::PendingClose(_) => tezos::close::cust_claim(
            channel_details.contract_details.contract_id,
            &customer_key_material,
        )
        .await
        .context(format!(
            "Failed to claim customer funds for {}",
            close.label.clone()
        )),
        // If it is Dispute, do nothing.
        State::Dispute(_) => (),
        _ => Err(anyhow::anyhow!(format!(
            "Unexpected state: expected PendingClose or Dispute, got {}",
            channel_details.status
        ))),
    }
}

/// Update channel to indicate a dispute.
///
/// **Usage**: this function is called in response to a merchDispute entrypoint call/operation that is
/// confirmed on chain at any depth.
#[allow(unused)]
async fn process_dispute(
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
) -> Result<(), anyhow::Error> {
    // Update channel status to Dispute
    database
        .with_channel_state(
            channel_name,
            zkchannels_state::PendingClose,
            |closing_message| -> Result<_, Infallible> {
                Ok((State::Dispute(closing_message), ()))
            },
        )
        .await
        .context(format!(
            "Failed to update channel status to Dispute for {}",
            channel_name
        ))?;

    Ok(())
}

/// Update channel state once a disputed unilateral close flow is finalized.
///
/// **Usage**: this function is called when a merchDispute entrypoint call/operation is confirmed
/// on chain to the required confirmation depth.
#[allow(unused)]
async fn finalize_dispute(
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
) -> Result<(), anyhow::Error> {
    // Update channel status from Dispute to Closed
    database
        .with_channel_state(
            channel_name,
            zkchannels_state::Dispute,
            |closing_message| -> Result<_, Infallible> { Ok((State::Closed(closing_message), ())) },
        )
        .await
        .context(format!(
            "Failed to update channel status to Closed for {}",
            channel_name
        ))?;

    // Indicate that all balances are paid out to the merchant.
    database
        .update_closing_balances(channel_name, merchant_balance, Some(customer_balance))
        .await
        .context(format!(
            "Failed to save final channel balances for {} after successful dispute",
            channel_name
        ))?;

    Ok(())
}

/// Update channel state once an undisputed unilateral close flow is complete.
/// This is either a customer unilateral close or an expiry close flow.
///
/// **Usage**: this function is called as response to an on-chain event:
/// - a custClaim entrypoint call operation is confirmed on chain at the required confirmation depth
/// - a merchClaim entrypoint call operation is confirmed on chain at the required confirmation depth
#[allow(unused)]
async fn finalize_close(
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
) -> Result<(), anyhow::Error> {
    // Update status from PendingClose to Closed.
    database
        .with_channel_state(
            channel_name,
            zkchannels_state::PendingClose,
            |closing_message| -> Result<_, Infallible> { Ok((State::Closed(closing_message), ())) },
        )
        .await
        .context(format!(
            "Failed to update channel status to Closed for {}",
            channel_name
        ))?;

    // Update final balances to indicate that the customer balance is paid out to the customer.
    database
        .update_closing_balances(channel_name, merchant_balance, Some(customer_balance))
        .await
        .context(format!(
            "Failed to save final channel balances for {} after successful close",
            channel_name
        ))?;

    Ok(())
}

async fn mutual_close(
    close: &Close,
    rng: StdRng,
    config: self::Config,
    tezos_key_material: TezosKeyMaterial,
) -> Result<(), anyhow::Error> {
    let database = database(&config)
        .await
        .context("Failed to connect to local database")?;

    let channel_details = database.get_channel(&close.label).await.context(format!(
        "Failed to get channel details for {}",
        close.label.clone()
    ))?;

    // Connect and select the Close session
    let (_session_key, chan) = connect(&config, &channel_details.address)
        .await
        .context("Failed to connect to merchant")?;

    let chan = chan
        .choose::<3>()
        .await
        .context("Failed selecting close session with merchant")?;

    // Run zkAbacus mutual close, which sets the channel status to PendingClose and gives the
    // customer authorization to call the mutual close entrypoint.
    let (close_state, chan) = zkabacus_close(rng, database.as_ref(), &close.label, chan)
        .await
        .context("zkAbacus close failed.")?;

    // Receive an authorization signature from merchant under the merchant's EdDSA Tezos key.
    let (authorization_signature, chan) = chan
        .recv()
        .await
        .context("Failed to receive authorization signature from the merchant.")?;

    // Retrieve contract info
    let contract_id = channel_details
        .contract_details
        .contract_id
        .ok_or_else(|| {
            anyhow::anyhow!("No saved contract details; cannot complete mutual close")
        })?;

    // Call the mutual close entrypoint
    let mutual_close_result = tezos::close::mutual_close(
        &contract_id,
        close_state.channel_id(),
        close_state.customer_balance(),
        close_state.merchant_balance(),
        authorization_signature,
        &tezos_key_material,
    )
    .await;

    // If the mutual close entrypoint call fails due to invalid authorization signature, abort!()
    if let Err(escrow::types::Error::InvalidAuthorizationSignature(_)) = mutual_close_result {
        abort!(in chan return close::Error::InvalidMerchantAuthSignature)
    }

    // Otherwise, close the dialectic channel
    proceed!(in chan);
    chan.close();

    // ...and raise the appropriate error if one exists.
    // The customer has the option to retry or initiate a unilateral close.
    // We should consider having the customer automatically initiate a unilateral close after a
    // random delay.
    let final_balances = mutual_close_result.context(format!(
        "Failed to call mutual close for {}",
        close.label.clone()
    ))?;

    // Finalize the result of the mutual close entrypoint call.
    finalize_mutual_close(
        database.as_ref(),
        &config,
        &close.label,
        final_balances.merchant_balance(),
        final_balances.customer_balance(),
    )
    .await
}

/// Update the channel state from PendingClose to Closed at completion of mutual close.
///
/// **Usage**: This should be called when the customer receives a confirmation from the blockchain
/// that the mutual close operation has been applied and has reached required confirmation depth.
/// It will only be called after a successful execution of [`mutual_close()`].
#[allow(unused)]
async fn finalize_mutual_close(
    database: &dyn QueryCustomer,
    config: &self::Config,
    channel_name: &ChannelName,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
) -> Result<(), anyhow::Error> {
    // Update database channel status from PendingClose to Closed.
    // and save final balances (should match those in the ClosingMessage)
    finalize_close(database, channel_name, merchant_balance, customer_balance)
        .await
        .context("Failed to finalize mutual close");

    // Notify the on-chain monitoring daemon this channel is closed.
    refresh_daemon(config).await
}

async fn zkabacus_close(
    rng: StdRng,
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
    chan: Chan<close::Close>,
) -> Result<(CloseState, Chan<close::MerchantSendAuthorization>), anyhow::Error> {
    // Generate the closing message and update state to PendingClose
    let closing_message = get_close_message(rng, database, channel_name)
        .await
        .context("Failed to generate mutual close data.")?;

    let (close_signature, close_state) = closing_message.into_parts();

    // Send the pieces of the CloseMessage.
    let chan = chan
        .send(close_signature)
        .await
        .context("Failed to send close state signature")?
        .send(close_state.clone())
        .await
        .context("Failed to send close state")?;

    // Let merchant reject an invalid or outdated `CloseMessage`.
    offer_abort!(in chan as Customer);

    Ok((close_state, chan))
}

/// Extract the close message from the saved channel status (including the current state
/// any stored signatures) and update the channel state to PendingClose atomically.
async fn get_close_message(
    mut rng: StdRng,
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
) -> Result<ClosingMessage, anyhow::Error> {
    let closing_message = database
        .with_closeable_channel(channel_name, |state| {
            let close_message = match state {
                State::Inactive(inactive) => inactive.close(&mut rng),
                State::Originated(inactive) => inactive.close(&mut rng),
                State::CustomerFunded(inactive) => inactive.close(&mut rng),
                State::MerchantFunded(inactive) => inactive.close(&mut rng),
                State::Ready(ready) => ready.close(&mut rng),
                State::Started(started) => started.close(&mut rng),
                State::Locked(locked) => locked.close(&mut rng),
                State::PendingClose(close_message) => close_message,
                // Cannot close on Disputed or Closed channels
                _ => return Err(close::Error::UncloseableState(state.state_name())),
            };
            Ok((State::PendingClose(close_message.clone()), close_message))
        })
        .await
        .context(format!(
            "Failed to update channel status to PendingClose for {}",
            channel_name
        ))??;

    Ok(closing_message)
}

fn write_close_json(closing: &Closing) -> Result<(), anyhow::Error> {
    let close_json_path = PathBuf::from(format!(
        "{}.close.json",
        hex::encode(closing.channel_id.to_bytes())
    ));
    let mut close_file = File::create(&close_json_path)
        .with_context(|| format!("Could not open file for writing: {:?}", &close_json_path))?;
    serde_json::to_writer(&mut close_file, &closing)
        .with_context(|| format!("Could not write close data to file: {:?}", &close_json_path))?;

    eprintln!("Closing data written to {:?}", &close_json_path);
    Ok(())
}

async fn refresh_daemon(config: &Config) -> anyhow::Result<()> {
    let (_session_key, chan) = connect_daemon(config)
        .await
        .context("Failed to connect to daemon")?;

    chan.choose::<0>()
        .await
        .context("Failed to select daemon Refresh")?
        .close();

    Ok(())
}
