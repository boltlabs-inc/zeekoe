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
};

use zeekoe::{
    abort,
    customer::{
        cli::Close,
        client::ZkChannelAddress,
        database::{zkchannels_state, QueryCustomer, QueryCustomerExt, State},
        Chan, ChannelName, Config,
    },
    escrow::{self, tezos},
    offer_abort, proceed,
    protocol::{close, Party::Customer},
};
use zkabacus_crypto::{
    customer::ClosingMessage, ChannelId, CloseState, CloseStateSignature, CustomerBalance,
    MerchantBalance, RevocationLock,
};

use super::{connect, connect_daemon, database, load_tezos_client, Command};
use anyhow::Context;

#[async_trait]
impl Command for Close {
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;

        if self.force {
            unilateral_close(
                &self.label,
                &config,
                self.off_chain,
                &mut rng,
                database.as_ref(),
            )
            .await
            .context("Unilateral close failed")?;
        } else {
            mutual_close(&self, rng, config)
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
pub async fn unilateral_close(
    channel_name: &ChannelName,
    config: &Config,
    off_chain: bool,
    rng: &mut StdRng,
    database: &dyn QueryCustomer,
) -> Result<(), anyhow::Error> {
    // Read the closing message and set the channel state to PendingClose
    let close_message = get_close_message(rng, database, channel_name)
        .await
        .context("Failed to fetch closing message from database")?;

    // If the customer balance is non-zero, update state to indicate the customer will not respond to expiry.
    if close_message.customer_balance().into_inner() == 0 {
        database
            .with_channel_state(
                channel_name,
                zkchannels_state::PendingClose,
                |closing_message| -> Result<_, Infallible> {
                    Ok((State::PendingExpiry(closing_message), ()))
                },
            )
            .await
            .context(format!(
                "Failed to update channel status to PendingExpiry for {}",
                channel_name
            ))??;
        return Ok(());
    }
    if !off_chain {
        // Call the custClose entrypoint and wait for it to be confirmed on chain
        let tezos_client = load_tezos_client(config, channel_name, database).await?;
        tezos::close::cust_close(&tezos_client, &close_message).await?;
    } else {
        // TODO: Print out information necessary to produce custClose transaction
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

    // React to a successfully posted custClose: update final merchant balance
    finalize_customer_close(database, channel_name, *close_message.merchant_balance()).await?;

    // Notify the on-chain monitoring daemon this channel has started to close.
    //refresh_daemon(&config).await
    Ok(())
}

/// Update channel balances when merchant receives payout in unilateral close flows.
///
/// **Usage**: this function is called when the
/// custClose entrypoint call/operation is confirmed on chain at an appropriate depth.
async fn finalize_customer_close(
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
    merchant_balance: MerchantBalance,
) -> anyhow::Result<()> {
    // TODO: assert that the db status is PendingClose,

    // Indicate that the merchant balance has been paid out to the merchant
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
//
// Note to developers: This function reverts the status update if the `cust_claim` entrypoint call
// fails. This revert is only valid if no other state changes in this function!
// DO NOT ADD STATE CHANGES without first removing the status update.
pub async fn claim_funds(
    database: &dyn QueryCustomer,
    config: &Config,
    channel_name: &ChannelName,
) -> Result<(), anyhow::Error> {
    // Retrieve channel information
    let channel_details = database.get_channel(channel_name).await.context(format!(
        "Failed to retrieve channel details to claim funds for {}",
        channel_name.clone()
    ))?;

    match channel_details.state {
        // Carry on to call the custClaim entrypoint
        State::PendingClose(_) => {},
        // Don't claim funds if the channel is disputed or already closed
        State::Dispute(_) | State::Closed(_) => return Ok(()),
        // Anything else is an error
        _ => return Err(anyhow::anyhow!(format!(
            "Failed to claim customer funds for {}. Unexpected channel state: expected PendingClose, Dispute, or Closed; got {}",
            channel_name.clone(),
            channel_details.state.state_name(),
        ))),
    }

    // Update channel status to PendingCustomerClaim
    database
        .with_channel_state(
            channel_name,
            zkchannels_state::PendingClose,
            |closing_message| -> Result<_, Infallible> {
                Ok((State::PendingCustomerClaim(closing_message), ()))
            },
        )
        .await
        .context(format!(
            "Failed to update channel status to PendingCustomerClaim for {}",
            channel_name
        ))??;

    // Post custClaim entrypoint on chain and wait for it to be confirmed
    let tezos_client = load_tezos_client(config, channel_name, database).await?;
    match tezos::close::cust_claim(&tezos_client)
        .await
        .context(format!(
            "Failed to claim customer funds for {}",
            channel_name.clone()
        )) {
        Ok(_) => Ok(()),
        Err(e) => {
            // If `custClaim` didn't post correctly, revert state back to PendingClose
            database
                .with_channel_state(
                    channel_name,
                    zkchannels_state::PendingCustomerClaim,
                    |closing_message| -> Result<_, Infallible> {
                        Ok((State::PendingClose(closing_message), ()))
                    },
                )
                .await??;
            Err(e)
        }
    }
}

/// Update channel to indicate a dispute.
///
/// **Usage**: this function is called in response to a merchDispute entrypoint call/operation that is
/// confirmed on chain at any depth.
pub async fn process_dispute(
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
        ))??;

    Ok(())
}

/// Update channel state once a disputed unilateral close flow is finalized.
///
/// **Usage**: this function is called when a merchDispute entrypoint call/operation is confirmed
/// on chain to the required confirmation depth.
pub async fn finalize_dispute(
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
) -> Result<(), anyhow::Error> {
    // Update channel status from Dispute to Closed
    let (customer_balance, merchant_balance) = database
        .with_channel_state(
            channel_name,
            zkchannels_state::Dispute,
            |closing_message| -> Result<_, anyhow::Error> {
                let balances = transfer_balances_to_merchant(
                    *closing_message.customer_balance(),
                    *closing_message.merchant_balance(),
                )?;
                Ok((State::Closed(closing_message), balances))
            },
        )
        .await
        .context(format!(
            "Failed to update channel status to Closed for {}",
            channel_name
        ))??;

    // Indicate that all balances are paid out to the merchant
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
pub async fn finalize_customer_claim(
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
) -> Result<(), anyhow::Error> {
    // Update status from PendingCustomerClaim to Closed
    let (merchant_balance, customer_balance) = database
        .with_channel_state(
            channel_name,
            zkchannels_state::PendingCustomerClaim,
            |closing_message| -> Result<_, Infallible> {
                let balances = (
                    *closing_message.merchant_balance(),
                    *closing_message.customer_balance(),
                );
                Ok((State::Closed(closing_message), balances))
            },
        )
        .await
        .context(format!(
            "Failed to update channel status to Closed for {}",
            channel_name
        ))??;

    // Update final balances to indicate that the customer balance is paid out to the customer
    database
        .update_closing_balances(channel_name, merchant_balance, Some(customer_balance))
        .await
        .context(format!(
            "Failed to save final channel balances for {} after successful close",
            channel_name
        ))?;

    Ok(())
}

/// Update channel state after the merchant claims the full channel balances; this happens in the
/// expiry close flow if the customer _does not_ post corrected channel balances via custCluse.
///
/// **Usage**: this function is called as response to an on-chain event:
/// - a merchClaim entrypoint call operation is confirmed on chain at the required confirmation depth
pub async fn finalize_expiry(
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
) -> Result<(), anyhow::Error> {
    // Update status from PendingExpiry to Closed
    // Calculate updated balances (all money going to the merchant)
    let (customer_balance, merchant_balance) = database
        .with_channel_state(
            channel_name,
            zkchannels_state::PendingExpiry,
            |closing_message| -> Result<_, anyhow::Error> {
                let balances = transfer_balances_to_merchant(
                    *closing_message.customer_balance(),
                    *closing_message.merchant_balance(),
                )?;
                Ok((State::Closed(closing_message), balances))
            },
        )
        .await
        .context(format!(
            "Failed to update channel status to Closed for {}",
            channel_name
        ))??;

    // Save final balances (with all money going to the merchant)
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
) -> Result<(), anyhow::Error> {
    let database = database(&config)
        .await
        .context("Failed to connect to local database")?;

    let channel_details = database.get_channel(&close.label).await.context(format!(
        "Failed to get channel details for {}",
        close.label.clone()
    ))?;

    // Run zkAbacus mutual close, which sets the channel status to PendingClose and gives the
    // customer authorization to call the mutual close entrypoint
    let (close_state, chan) = zkabacus_close(
        rng,
        database.as_ref(),
        &close.label,
        &config,
        &channel_details.address,
    )
    .await
    .context("zkAbacus close failed.")?;

    // Receive an authorization signature from merchant under the merchant's EdDSA Tezos key
    let (authorization_signature, chan) = chan
        .recv()
        .await
        .context("Failed to receive authorization signature from the merchant.")?;

    // Call the mutual close entrypoint
    let tezos_client = load_tezos_client(&config, &close.label, database.as_ref()).await?;
    let mutual_close_result = tezos::close::mutual_close(
        &tezos_client,
        close_state.channel_id(),
        close_state.customer_balance(),
        close_state.merchant_balance(),
        &authorization_signature,
    )
    .await;

    // If the mutual close entrypoint call fails due to invalid authorization signature, abort!()
    if let Err(escrow::types::Error::InvalidAuthorizationSignature(_)) = mutual_close_result {
        abort!(in chan return close::Error::InvalidMerchantAuthSignature)
    }

    // Otherwise, close the dialectic channel...
    proceed!(in chan);
    chan.close();

    // ...and raise the appropriate error if one exists.
    // The customer has the option to retry or initiate a unilateral close.
    // We should consider having the customer automatically initiate a unilateral close after a
    // random delay.
    mutual_close_result.context(format!(
        "Failed to call mutual close for {}",
        close.label.clone()
    ))?;

    // Finalize the result of the mutual close entrypoint call
    finalize_mutual_close(
        database.as_ref(),
        &config,
        &close.label,
        *close_state.merchant_balance(),
        *close_state.customer_balance(),
    )
    .await
}

/// Update the channel state from PendingClose to Closed at completion of mutual close.
///
/// **Usage**: This should be called when the customer receives a confirmation from the blockchain
/// that the mutual close operation has been applied and has reached required confirmation depth.
/// It will only be called after a successful execution of [`mutual_close()`].
async fn finalize_mutual_close(
    database: &dyn QueryCustomer,
    _config: &self::Config,
    channel_name: &ChannelName,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
) -> Result<(), anyhow::Error> {
    // Update status from PendingClose to Closed
    database
        .with_channel_state(
            channel_name,
            zkchannels_state::PendingClose,
            |closing_message| Ok::<_, Infallible>((State::Closed(closing_message), ())),
        )
        .await
        .context(format!(
            "Failed to update channel status to Closed for {}",
            channel_name
        ))??;

    // Update final balances to indicate that the customer balance is paid out to the customer
    database
        .update_closing_balances(channel_name, merchant_balance, Some(customer_balance))
        .await
        .context(format!(
            "Failed to save final channel balances for {} after successful close",
            channel_name
        ))?;

    // Notify the on-chain monitoring daemon this channel is closed
    // refresh_daemon(config).await
    Ok(())
}

async fn zkabacus_close(
    mut rng: StdRng,
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
    config: &self::Config,
    address: &ZkChannelAddress,
) -> Result<(CloseState, Chan<close::MerchantSendAuthorization>), anyhow::Error> {
    // Connect communication channel to the merchant
    let (_session_key, chan) = connect(config, address)
        .await
        .context("Failed to connect to merchant")?;

    // Select the Close session
    let chan = chan
        .choose::<3>()
        .await
        .context("Failed selecting close session with merchant")?;

    // Generate the closing message and update state to PendingClose
    let closing_message = get_close_message(&mut rng, database, channel_name)
        .await
        .context("Failed to generate mutual close data.")?;

    let (close_signature, close_state) = closing_message.into_parts();

    // Send the pieces of the CloseMessage
    let chan = chan
        .send(close_signature)
        .await
        .context("Failed to send close state signature")?
        .send(close_state.clone())
        .await
        .context("Failed to send close state")?;

    // Let merchant reject an invalid or outdated `CloseMessage`
    offer_abort!(in chan as Customer);

    Ok((close_state, chan))
}

/// Extract the close message from the saved channel status (including the current state
/// any stored signatures) and update the channel state to PendingClose atomically.
async fn get_close_message(
    rng: &mut StdRng,
    database: &dyn QueryCustomer,
    channel_name: &ChannelName,
) -> Result<ClosingMessage, anyhow::Error> {
    let closing_message = database
        .with_closeable_channel(channel_name, |state| {
            let close_message = match state {
                State::Inactive(inactive) => inactive.close(rng),
                State::Originated(inactive) => inactive.close(rng),
                State::CustomerFunded(inactive) => inactive.close(rng),
                State::MerchantFunded(inactive) => inactive.close(rng),
                State::Ready(ready) => ready.close(rng),
                State::Started(started) => started.close(rng),
                State::Locked(locked) => locked.close(rng),
                State::PendingClose(close_message) => close_message,
                // Cannot enter PendingClose on a channel that has passed that point
                State::PendingExpiry(_)
                | State::PendingCustomerClaim(_)
                | State::Dispute(_)
                | State::Closed(_) => {
                    return Err(close::Error::UncloseableState(state.state_name()))
                }
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

fn transfer_balances_to_merchant(
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
) -> Result<(CustomerBalance, MerchantBalance), anyhow::Error> {
    Ok((
        CustomerBalance::try_new(0)?,
        MerchantBalance::try_new(customer_balance.into_inner() + merchant_balance.into_inner())?,
    ))
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

#[allow(unused)]
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
