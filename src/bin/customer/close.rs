use {async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::{
    customer::{
        cli::Close,
        database::{QueryCustomer, QueryCustomerExt, State},
        Chan, ChannelName, Config,
    },
    offer_abort, proceed,
    protocol::{close, Party::Customer},
};
use zkabacus_crypto::customer::ClosingMessage;

use super::{connect, database, Command};
use anyhow::Context;

#[async_trait]
impl Command for Close {
    #[allow(unused)]
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        if self.force {
            initiate_unilateral_close(&self, rng, config)
                .await
                .context("Unilateral close failed.")?;
        } else {
            mutual_close(&self, rng, config)
                .await
                .context("Mutual close failed.")?;
        }
        todo!()
    }
}

/// Initiate a unilateral close of the channel, either initially or in response to the merchant posting
/// an expiry to the contract.
///
/// **Usage**: this function registers the customer's claim on the funds. It can be called directly
/// or as a response to an on-chain event.
async fn initiate_unilateral_close(
    close: &Close,
    rng: StdRng,
    config: self::Config,
) -> Result<(), anyhow::Error> {
    let database = database(&config)
        .await
        .context("Failed to connect to local database")?;

    // Retrieve the close state and update channel status to PENDING_CLOSE.
    let _close_message = get_close_message(rng, database.as_ref(), &close.label)
        .await
        .context("Failed to get closing information.")?;

    // TODO: Produce an authorization signature based on the closing message.

    // TODO: Send a disbursement request to the escrow agent.

    Ok(())
}

/// Claim final balance of the channel.
///
/// **Usage**: this function is called as a response to an on-chain event. It is only called after
/// the contract claim delay has passed.
#[allow(unused)]
async fn claim_channel_balance(close: &Close, config: self::Config) -> Result<(), anyhow::Error> {
    let database = database(&config)
        .await
        .context("Failed to connect to local database")?;

    // TODO: Send claim request to escrow agent (maybe you will have to first extract the ClosingMessage with state
    // information from the database?)

    // TODO: update state in db from PENDING to CLOSED
    
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

    // Look up the address and current local customer state for this merchant in the database
    let address = match database
        .channel_address(&close.label)
        .await
        .context("Failed to look up channel address in local database")?
    {
        None => return Err(anyhow::anyhow!("Unknown channel label: {}", close.label)),
        Some(address) => address,
    };

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

    // TODO: get auth signature from merchant
    /*
    let merchant_authorization_signature = chan
        .recv()
        .await
        .context("Failed to receive authorization signature from the merchant.")?;
    */

    // TODO: verify the signature. Raise error if invalid.
    proceed!(in chan);

    // Close the channel - all remaining operations are with the escrow agent.
    chan.close();

    // TODO: generate customer authorization signature.

    // TODO: call escrow agent disburse / mutual close endpoint. Raise error if it fails.

    // Update database channel status from PendingClose to Closed.
    database
        .with_channel_state(&close.label, |state| match state.take() {
            Some(State::PendingClose(_)) => *state = None,
            not_pending_state => {
                *state = not_pending_state;
                anyhow::anyhow!(format!(
                    "Expecting the channel \"{}\" to be in a different state",
                    &close.label
                ));
            }
        })
        .await
        .context("Database error while updating state to Closed")??;

    Ok(())
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

async fn get_close_message(
    mut rng: StdRng,
    database: &dyn QueryCustomer,
    label: &ChannelName,
) -> Result<ClosingMessage, anyhow::Error> {
    // Extract current state from the db.
    database
        .with_channel_state(label, |state| {
            // Extract the close message.
            let closing_message = match state.take() {
                Some(State::Inactive(inactive)) => inactive.close(&mut rng),
                Some(State::Ready(ready)) => ready.close(&mut rng),
                Some(State::Started(started)) => started.close(&mut rng),
                Some(State::Locked(locked)) => locked.close(&mut rng),
                // TODO: name the state (Pending vs Closed)
                uncloseable_state => {
                    *state = uncloseable_state;
                    return Err(anyhow::anyhow!("Expected closeable state"));
                }
            };

            // Set the new PendingClose state in the database.
            *state = Some(State::PendingClose(closing_message.clone()));

            Ok(closing_message)
        })
        .await
        .context("Database error while fetching initial pay state")??
}
