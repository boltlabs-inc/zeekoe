use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng, std::convert::TryInto};

use zkabacus_crypto::{
    customer::{LockMessage, StartMessage},
    ClosingSignature, Context as ProofContext, PayToken, PaymentAmount,
};

use zeekoe::{
    abort,
    customer::{
        cli::{Note, Pay, Refund},
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
    async fn run(self, rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        let payment_amount = self.pay.try_into()?;

        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;

        let (session_key, chan) = open_session(database.as_ref(), &config, &self.label).await?;

        let chan = request_payment(&config, chan, payment_amount, self.note)
            .with_timeout(config.approval_timeout)
            .await
            .context("Payment timed out while awaiting approval")?
            .context("Payment was not approved by the merchant")?;

        // Run the core zkAbacus.Pay protocol
        // Timeout is set to 10 messages, which includes all sent & received messages and aborts
        let chan = zkabacus_pay(
            rng,
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

        receive_service(chan)
            .with_timeout(config.approval_timeout)
            .await
            .context("Payment timed out when receiving service")??;

        Ok(())
    }
}

/// Set up the communication channel with the merchant.
async fn open_session(
    database: &dyn QueryCustomer,
    config: &Config,
    channel_name: &ChannelName,
) -> Result<(SessionKey, Chan<pay::Pay>), anyhow::Error> {
    // Look up the address and current local customer state for this merchant in the database
    let address = database
        .channel_address(channel_name)
        .await
        .context("Failed to look up channel address in local database")?;

    // Connect and select the Pay session
    let (session_key, chan) = connect(config, &address).await?;
    let chan = chan
        .choose::<2>()
        .await
        .context("Failed selecting pay session with merchant")?;

    Ok((session_key, chan))
}

/// Request approval for the payment request from the merchant, aborting the session if it is not
/// granted.
async fn request_payment(
    config: &Config,
    chan: Chan<pay::Pay>,
    payment_amount: PaymentAmount,
    payment_note: Option<Note>,
) -> Result<Chan<pay::CustomerStartPayment>, anyhow::Error> {
    // Read the contents of the note, if any
    let note = payment_note
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
    offer_abort!(in chan as Customer);

    Ok(chan)
}

/// Receive the paid-for service from the merchant, printing the outcome if there is one and
/// closing the communication channel.
async fn receive_service(chan: Chan<pay::MerchantProvideService>) -> Result<(), anyhow::Error> {
    // Receive the response note (i.e. the fulfillment of the service)
    let (response_note, chan) = chan
        .recv()
        .await
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

/// The core zkAbacus.Pay protocol: receive a valid, updated channel state.
async fn zkabacus_pay(
    mut rng: StdRng,
    database: &dyn QueryCustomer,
    label: &ChannelName,
    session_key: SessionKey,
    chan: Chan<pay::CustomerStartPayment>,
    payment_amount: PaymentAmount,
) -> Result<Chan<pay::MerchantProvideService>, anyhow::Error> {
    // Generate the shared context for proofs
    let context = ProofContext::new(&session_key.to_bytes());

    // Start the zkAbacus core payment and get fresh proofs and commitments
    let start_message = start_payment(&mut rng, database, label, payment_amount, context).await?;

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

    // Verify the closing signature and transition into a locked state
    let chan = if let Some(lock_message) = lock_payment(database, label, closing_signature).await? {
        proceed!(in chan);

        // If the closing signature verifies, reveal our lock, secret, and blinding factor
        let chan = chan
            .send(lock_message.revocation_lock)
            .await
            .context("Failed to send revocation lock")?
            .send(lock_message.revocation_secret)
            .await
            .context("Failed to send revocation secret")?
            .send(lock_message.revocation_lock_blinding_factor)
            .await
            .context("Failed to send revocation lock blinding factor")?;

        // Allow the merchant to cancel the session at this point, and throw an error if so
        offer_abort!(in chan as Customer);
        chan
    } else {
        // If the closing signature does not verify, inform the merchant we are aborting
        abort!(in chan return pay::Error::InvalidPayToken);
    };

    // Receive a pay token from the merchant, which allows us to pay again
    let (pay_token, chan) = chan
        .recv()
        .await
        .context("Failed to receive payment token")?;

    // Unlock the payment channel using the pay token
    unlock_payment(database, label, pay_token).await?;

    Ok(chan)
}

/// Attempt to start the payment for the channel of the given label, using the given
/// [`PaymentAmount`] and [`ProofContext`].
///
/// Returns the [`StartMessage`] for broadcast to the merchant if successful.
async fn start_payment(
    rng: &mut StdRng,
    database: &dyn QueryCustomer,
    label: &ChannelName,
    payment_amount: PaymentAmount,
    context: ProofContext,
) -> Result<StartMessage, anyhow::Error> {
    let zkabacus_config = database.channel_zkabacus_config(label).await?;
    // Try to start the payment. If successful, update channel status to `Started`.
    database
        .with_channel_state(label, zkchannels_state::Ready, |ready| {
            // Try to start the payment using the payment amount and proof context
            match ready.start(rng, payment_amount, &context, &zkabacus_config) {
                Ok((started, start_message)) => Ok((State::Started(started), start_message)),
                Err((_, e)) => Err(pay::Error::StartFailed(e)),
            }
        })
        .await
        .with_context(|| format!("Failed to update channel {} to Started status", &label))?
        .map_err(|e| e.into())
}

/// Attempt to lock a started payment for the channel of the given label, using the given
/// [`ClosingSignature`].
///
/// Returns the [`LockMessage`] for broadcast to the merchant if successful, or `None` if the
/// database operations succeeded but the closing signature was invalid.
async fn lock_payment(
    database: &dyn QueryCustomer,
    label: &ChannelName,
    closing_signature: ClosingSignature,
) -> Result<Option<LockMessage>, anyhow::Error> {
    let zkabacus_config = database.channel_zkabacus_config(label).await?;
    // Try to continue (lock) the payment. If successful, update channel status to `Locked`.
    database
        .with_channel_state(label, zkchannels_state::Started, |started| {
            // Attempt to lock the state using the closing signature. If it fails, raise a `pay::Error`.
            match started.lock(closing_signature, &zkabacus_config) {
                Ok((locked, lock_message)) => Ok((State::Locked(locked), lock_message)),
                Err(_) => Err(pay::Error::InvalidClosingSignature),
            }
        })
        .await
        .map(Result::ok)
        .with_context(|| format!("Failed to update channel {} to Locked status", &label))
}

/// Attempt to unlock a locked payment for a channel of the given label, using the given
/// [`PayToken`].
///
/// If successful, this updates the state in the database for the channel so that it is ready for
/// the next payment.
async fn unlock_payment(
    database: &dyn QueryCustomer,
    label: &ChannelName,
    pay_token: PayToken,
) -> Result<(), anyhow::Error> {
    let zkabacus_config = database.channel_zkabacus_config(label).await?;
    // Try to finish (unlock) the payment. If successful, update channel status to `Ready`.
    database
        .with_channel_state(label, zkchannels_state::Locked, |locked| {
            // Attempt to unlock the state using the pay token
            match locked.unlock(pay_token, &zkabacus_config) {
                Ok(ready) => Ok((State::Ready(ready), ())),
                Err(_) => Err(pay::Error::InvalidPayToken),
            }
        })
        .await
        .with_context(|| format!("Failed to update channel {} to Ready status", &label))?
        .map_err(|e| anyhow::anyhow!(e))
}

#[async_trait]
impl Command for Refund {
    async fn run(self, rng: StdRng, config: Config) -> Result<(), anyhow::Error> {
        // A refund is merely a negative payment
        self.into_negative_pay().run(rng, config).await
    }
}
