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
    async fn run(self, rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        if self.force {
            unilateral_close(&self, rng, config)
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
    chan.close();

    // TODO: generate customer authorization signature.

    // TODO: call escrow agent disburse / mutual close endpoint. Raise error if it fails.

    // TODO: Update database channel status from PendingClose to Closed.

    todo!()
}

async fn zkabacus_close(
    rng: StdRng,
    database: &dyn QueryCustomer,
    label: &ChannelName,
    chan: Chan<close::Close>,
) -> Result<Chan<close::MerchantSendAuthorization>, anyhow::Error> {
    let closing_message = get_close_message(rng, database, label)
        .await
        .context("Failed to retrieve close state and corresponding signature.")?;

    let (close_signature, close_state) = closing_message.into_parts();

    // send the pieces of the CloseMessage.
    let chan = chan
        .send(close_signature)
        .await
        .context("Failed to send close state signature")?
        .send(close_state)
        .await
        .context("Failed to send close state")?;

    // offer abort to merchant.
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
                _ => return Err(anyhow::anyhow!("Expected closeable state")),
            };

            // Set the new PendingClose state in the database.
            *state = Some(State::PendingClose(closing_message));
            // @Kenny / @Mukund: do we need to store the pending close message in the db? Or
            // do we only need to know that we *tried* to close?

            //Ok(closing_message)
            todo!()
        })
        .await
        .context("Database error while fetching initial pay state")??
}
