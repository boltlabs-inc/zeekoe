use {
    anyhow::Context,
    async_trait::async_trait,
    rand::rngs::StdRng,
    std::convert::{Infallible, TryInto},
};

use zkabacus_crypto::{
    customer::{LockMessage, StartMessage},
    Context as ProofContext, PaymentAmount,
};

use zeekoe::{
    abort,
    customer::{
        cli::{Pay, Refund},
        client::SessionKey,
        database::{zkchannels_state, QueryCustomer, QueryCustomerExt, State},
        Chan, ChannelName, Config,
    },
    offer_abort, proceed,
    protocol::{pay, Party::Customer},
    timeout::WithTimeout,
};

use super::{connect, database, Command};

#[async_trait]
impl Command for Pay {
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        let payment_amount = self.pay.try_into()?;

        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;

        // Look up the address and current local customer state for this merchant in the database
        let address = database
            .channel_address(&self.label)
            .await
            .context("Failed to look up channel address in local database")?;

        // Set up communication session with the merchant and select the Pay protocol
        let (session_key, chan) = connect(&config, &address).await?;
        let chan = chan
            .choose::<2>()
            .await
            .context("Failed selecting pay session with merchant")?;

        // Read the contents of the note, if any
        let note = self
            .note
            .unwrap_or_default()
            .read(config.max_note_length)
            .context("Failed to read payment note from standard input or command line")?;

        // Send the payment amount and note to the merchant
        let chan = chan
            .send(payment_amount)
            .await
            .context("Failed to send payment amount")?
            .send(note)
            .await
            .context("Failed to send payment note")?;

        // Allow the merchant to accept or reject the payment and note
        let chan = async {
            offer_abort!(in chan as Customer);
            Ok(chan)
        }
        .with_timeout(config.approval_timeout)
        .await
        .context("Payment timed out while awaiting approval")?
        .context("Payment was not approved by the merchant")?;

        // Run the core zkAbacus.Pay protocol
        // Timeout is set to 10 messages, which includes all sent & received messages and aborts
        let chan = zkabacus_pay(
            &mut rng,
            database.as_ref(),
            &self.label,
            session_key,
            chan,
            payment_amount,
        )
        .with_timeout(10 * config.message_timeout)
        .await
        .context("Payment timed out while updating channel status")?
        .context("Failed to complete pay protocol")?;

        // Receive the response note (i.e. the fulfillment of the service)
        let (response_note, chan) = chan
            .recv()
            .with_timeout(config.approval_timeout)
            .await
            .context("Payment timed out when receiving service")?
            .context("Failed to receive response note")?;

        // Close the communication channel: we are done communicating with the merchant
        chan.close();

        // Print the response note on standard out
        if let Some(response_note) = response_note {
            eprintln!(
                "Payment succeeded with response from merchant: \"{}\"",
                response_note
            );
        } else {
            eprintln!("Payment succeeded with no concluding response from merchant");
        }
        Ok(())
    }
}

/// The core zkAbacus.Pay protocol: receive a valid, updated channel state.
async fn zkabacus_pay(
    mut rng: &mut StdRng,
    database: &dyn QueryCustomer,
    label: &ChannelName,
    session_key: SessionKey,
    chan: Chan<pay::CustomerStartPayment>,
    payment_amount: PaymentAmount,
) -> Result<Chan<pay::MerchantProvideService>, anyhow::Error> {
    // Generate the shared context for proofs
    let context = ProofContext::new(&session_key.to_bytes());

    let zkabacus_config = database.channel_zkabacus_config(label).await?;

    // If a channel in [`State::PendingPayment`] and the merchant posts expiry, we ignore it.
    database
        .with_channel_state(
            label,
            zkchannels_state::Ready,
            |ready| -> Result<_, Infallible> { Ok((State::PendingPayment(ready), ())) },
        )
        .await
        .with_context(|| {
            format!(
                "Failed to update channel {} to PendingPayment status",
                label
            )
        })??;

    // Try to start the zkAbacus payment protocol. If successful, update channel status to `Started`
    let start_message = database
        .with_channel_state(label, zkchannels_state::PendingPayment, |ready| {
            // Try to start the payment using the payment amount and proof context
            match ready.start(&mut rng, payment_amount, &context, &zkabacus_config) {
                Ok((started, start_message)) => Ok((State::Started(started), start_message)),
                Err((_, e)) => Err(pay::Error::StartFailed(e)),
            }
        })
        .await
        .with_context(|| format!("Failed to update channel {} to Started status", &label))??;

    let (chan, lock_message) =
        match zkabacus_lock(chan, label, database, &zkabacus_config, start_message).await {
            Ok(out) => out,
            Err(err) => {
                database
                    .with_channel_state(
                        label,
                        zkchannels_state::Started,
                        |started| -> Result<_, Infallible> {
                            Ok((State::StartedFailed(started), ()))
                        },
                    )
                    .await
                    .with_context(|| {
                        format!("Failed to update channel {} to StartedFailed", &label)
                    })??;
                return Err(err);
            }
        };

    match zkabacus_unlock(chan, label, database, &zkabacus_config, lock_message).await {
        Ok(chan) => Ok(chan),
        Err(err) => {
            database
                .with_channel_state(
                    label,
                    zkchannels_state::Locked,
                    |locked| -> Result<_, Infallible> { Ok((State::LockedFailed(locked), ())) },
                )
                .await
                .with_context(|| {
                    format!("Failed to update channel {} to LockedFailed", &label)
                })??;
            Err(err)
        }
    }
}

async fn zkabacus_lock(
    chan: Chan<pay::CustomerStartPayment>,
    label: &ChannelName,
    database: &dyn QueryCustomer,
    zkabacus_config: &zkabacus_crypto::customer::Config,
    start_message: StartMessage,
) -> Result<(Chan<pay::CustomerChooseAbort>, LockMessage), anyhow::Error> {
    // Send the initial proofs and commitments to the merchant
    let chan = chan
        .send(start_message.nonce)
        .await
        .context("Failed to send nonce")?
        .send(start_message.pay_proof)
        .await
        .context("Failed to send payment proof")?;

    // Allow the merchant to cancel the session at this point, and throw an error if so
    offer_abort!(in chan as Customer);

    // Receive a closing signature from the merchant
    let (closing_signature, chan) = chan
        .recv()
        .await
        .context("Failed to receive closing signature")?;

    // Verify the closing signature and transition into a locked channel state
    match database
        .with_channel_state(label, zkchannels_state::Started, |started| {
            // Attempt to lock the state using the closing signature. If it fails, raise a `pay::Error`.
            match started.lock(closing_signature, zkabacus_config) {
                Ok((locked, lock_message)) => Ok((State::Locked(locked), lock_message)),
                Err(_) => Err(pay::Error::InvalidClosingSignature),
            }
        })
        .await
        .with_context(|| format!("Failed to update channel {} to Locked status", &label))?
    {
        Ok(lock_message) => Ok((chan, lock_message)),
        // An error means that closing signature does not verify; abort the protocol
        Err(_) => abort!(in chan return pay::Error::InvalidPayToken),
    }
}

async fn zkabacus_unlock(
    chan: Chan<pay::CustomerChooseAbort>,
    label: &ChannelName,
    database: &dyn QueryCustomer,
    zkabacus_config: &zkabacus_crypto::customer::Config,
    lock_message: LockMessage,
) -> Result<Chan<pay::MerchantProvideService>, anyhow::Error> {
    proceed!(in chan);

    // If the closing signature verifies, reveal our lock, secret, and blinding factor
    let chan = chan
        .send(lock_message.revocation_pair)
        .await
        .context("Failed to send revocation pair")?
        .send(lock_message.revocation_lock_blinding_factor)
        .await
        .context("Failed to send revocation lock blinding factor")?;

    // Allow the merchant to cancel the session at this point, and throw an error if so
    offer_abort!(in chan as Customer);

    // Receive a pay token from the merchant, which allows us to pay again
    let (pay_token, chan) = chan
        .recv()
        .await
        .context("Failed to receive payment token")?;

    // Try to unlock the payment channel with the new pay token
    database
        .with_channel_state(label, zkchannels_state::Locked, |locked| {
            // Attempt to unlock the state using the pay token
            match locked.unlock(pay_token, zkabacus_config) {
                Ok(ready) => Ok((State::Ready(ready), ())),
                Err(_) => Err(pay::Error::InvalidPayToken),
            }
        })
        .await
        .with_context(|| format!("Failed to update channel {} to Ready status", &label))??;

    Ok(chan)
}

#[async_trait]
impl Command for Refund {
    async fn run(self, rng: StdRng, config: Config) -> Result<(), anyhow::Error> {
        // A refund is merely a negative payment
        self.into_negative_pay().run(rng, config).await
    }
}
