//* Close functionalities for a customer.
//*
//* In the current design, the customer requires either a watchtower or a notification service
//* to finalize the channel close.
//* This architecture is flexible; we could alternately allow the customer CLI to wait (hang) until
//* it receives confirmation (e.g. call `process_mutual_close_confirmation` directly from
//* `mutual_close()`).
use {async_trait::async_trait, rand::rngs::StdRng, std::convert::Infallible};

use zeekoe::{
    customer::{
        cli::Close,
        database::{self, Closed, QueryCustomer, QueryCustomerExt},
        Chan, ChannelName, Config,
    },
    offer_abort, proceed,
    protocol::{close, Party::Customer},
};
use zkabacus_crypto::customer::{ClosingMessage, Inactive, Locked, Ready, Started};

use super::{connect, database, Command};
use anyhow::Context;

#[async_trait]
impl Command for Close {
    #[allow(unused)]
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        if self.force {
            unilateral_close(&self, rng, config)
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

async fn unilateral_close(
    _close: &Close,
    _rng: StdRng,
    _config: self::Config,
) -> Result<(), anyhow::Error> {
    todo!()
}

async fn mutual_close(
    close: &Close,
    rng: StdRng,
    config: self::Config,
) -> Result<(), anyhow::Error> {
    let database = database(&config)
        .await
        .context("Failed to connect to local database")?;

    // Look up the address and current local customer state for this merchant in the database
    let address = database
        .channel_address(&close.label)
        .await
        .context("Failed to look up channel address in local database")?;

    // Connect and select the Close session
    let (_session_key, chan) = connect(&config, &address)
        .await
        .context("Failed to connect to merchant")?;

    let chan = chan
        .choose::<3>()
        .await
        .context("Failed selecting close session with merchant")?;

    let chan = zkabacus_close(rng, database.as_ref(), &close.label, chan)
        .await
        .context("zkAbacus close failed.")?;

    // TODO: Receive an authorization signature from merchant under the merchant's EDDSA Tezos key.
    // The signature should be over a tuple with
    // (contract id, "zkChannels mutual close", channel id, customer balance, merchant balance).
    /*
    let merchant_authorization_signature = chan
        .recv()
        .await
        .context("Failed to receive authorization signature from the merchant.")?;
    */

    // TODO: Verify that signature is a valid EDDSA signature with respect to the merchant's Tezos
    // public key on the tuple:
    // (contract id, "zkChannels mutual close", channel id, customer balance, merchant balance).
    //
    // abort!() if invalid with error InvalidMerchantAuthSignature.
    //
    // The customer has the option to retry or initiate a unilateral close.
    // We should consider having the customer automatically initiate a unilateral close after a
    // random delay.
    proceed!(in chan);

    // Close the dialectic channel - all remaining operations are with the escrow agent.
    chan.close();

    // TODO: Call the mutual close entrypoint which will take:
    // - current channel balances
    // - merchant authorization signature
    // - contract ID
    // - channel ID
    // abort!() if it fails with error ArbiterRejectedMutualClose.
    //
    // This function will:
    // - Generate customer authorization EDDSA signature on the operation with the customer's
    //   Tezos public key.
    // - Send operation to blockchain
    // - Raises an error if the operation fails to post. This may include relevant information
    //   (e.g. insufficient fees) or may be more generic.

    Ok(())
}

/// Update the channel state from pending to closed.
///
/// **Usage**: This should be called when the customer receives a confirmation from the blockchain
/// that the mutual close operation has been applied and has reached required confirmation depth.
/// It will only be called after a successful execution of [`mutual_close()`].
#[allow(unused)]
async fn process_mutual_close_confirmation(
    rng: &mut StdRng,
    config: self::Config,
    label: ChannelName,
) -> Result<(), anyhow::Error> {
    let database = database(&config)
        .await
        .context("Failed to connect to local database")?;

    // Update database channel status from PendingClose to Closed.
    database
        .with_channel_state(&label, |pending: ClosingMessage| {
            Ok((
                Closed::new(
                    pending.channel_id().clone(),
                    pending.customer_balance().clone(),
                    pending.merchant_balance().clone(),
                ),
                (),
            ))
        })
        .await
        .context("Database error while updating status to closed")?
}

async fn zkabacus_close(
    rng: StdRng,
    database: &dyn QueryCustomer,
    label: &ChannelName,
    chan: Chan<close::Close>,
) -> Result<Chan<close::MerchantSendAuthorization>, anyhow::Error> {
    // Generate the closing message and update state to pending-close.
    let closing_message = get_close_message(rng, database, label)
        .await
        .context("Failed to generate mutual close data.")?;

    let (close_signature, close_state) = closing_message.into_parts();

    // Send the pieces of the CloseMessage.
    let chan = chan
        .send(close_signature)
        .await
        .context("Failed to send close state signature")?
        .send(close_state)
        .await
        .context("Failed to send close state")?;

    // Let merchant reject an invalid or outdated `CloseMessage`.
    offer_abort!(in chan as Customer);

    Ok(chan)
}

macro_rules! try_close {
    ($rng:expr, $database:expr, $label:expr, $ty:ty) => {{
        let result = $database
            .with_channel_state(&$label, |state: $ty| {
                let message = state.close(&mut $rng);
                Ok::<_, Infallible>((message.clone(), message))
            })
            .await;

        match result {
            Ok(message) => match message {
                Ok(message) => return Ok(message),
                Err(infallible) => match infallible {},
            },
            Err(error) => match error {
                database::Error::UnexpectedState(_) => {}
                _ => return Err(error).context("Failed to set state to pending close in database"),
            },
        }
    };};
}

async fn get_close_message(
    mut rng: StdRng,
    database: &dyn QueryCustomer,
    label: &ChannelName,
) -> Result<ClosingMessage, anyhow::Error> {
    try_close!(rng, database, label, Inactive);
    try_close!(rng, database, label, Ready);
    try_close!(rng, database, label, Started);
    try_close!(rng, database, label, Locked);

    let result = database
        .with_channel_state(&label, |message: ClosingMessage| {
            Ok::<_, Infallible>((message.clone(), message))
        })
        .await;

    match result {
        Ok(message) => match message {
            Ok(message) => return Ok(message),
            Err(infallible) => match infallible {},
        },
        Err(error) => match error {
            database::Error::UnexpectedState(_) => {}
            _ => return Err(error).context("Failed to set state to pending close in database"),
        },
    }

    return Err(anyhow::anyhow!(
        "The channel with label \"{}\" was already closed",
        label
    ));
}
