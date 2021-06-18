use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng};

use zkabacus_crypto::{customer::Ready, Context as ProofContext, PaymentAmount};

use zeekoe::{
    choose_abort, choose_continue,
    customer::{
        cli::{Pay, Refund},
        Config,
    },
    offer_abort,
    protocol::pay,
};

use super::{connect, Command};

#[async_trait]
impl Command for Pay {
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        // Convert the payment amount appropriately
        let minor_units: i64 = self.pay.as_minor_units().ok_or_else(|| {
            anyhow::anyhow!("Payment amount invalid for currency or out of range for channel")
        })?;
        let payment_amount = (if minor_units < 0 {
            PaymentAmount::pay_customer
        } else {
            PaymentAmount::pay_merchant
        })(minor_units.abs() as u64)
        .context("Payment amount out of range")?;

        // Look up the address and current local customer state for this merchant in the database
        let address = todo!("look up address in database by `self.label`");
        let ready: Ready = todo!("look up channel state in database by `self.label`");

        // Connect and select the Pay session
        let (session_key, chan) = connect(&config, address)
            .await
            .context("Failed to connect to merchant")?;
        let chan = chan
            .choose::<1>()
            .await
            .context("Failed selecting Pay session with merchant")?;

        // Read the contents of the note, if any
        let note = self
            .note
            .unwrap_or_else(|| zeekoe::customer::cli::Note::String(String::from("")))
            .read(config.max_note_length)
            .context("Failed to read payment note from standard input")?;

        // Send the payment amount and note to the merchant
        let chan = chan
            .send(payment_amount)
            .await
            .context("Failed to send payment amount")?
            .send(note)
            .await
            .context("Failed to send payment note")?;

        // Allow the merchant to accept or reject the payment and note
        let chan = offer_abort!(in chan);

        // Generate the shared context for proofs
        let context = ProofContext::new(&session_key.to_bytes());

        // Start the zkAbacus core payment and get fresh proofs and commitments
        let (started, start_message) = ready
            .start(&mut rng, payment_amount, &context)
            .context("Failed to generate nonce and payment proof")?;

        // Send the initial proofs and commitments to the merchant
        let chan = chan
            .send(start_message.nonce)
            .await
            .context("Failed to send nonce")?
            .send(start_message.pay_proof)
            .await
            .context("Failed to send payment proof")?;

        // Allow the merchant to cancel the session at this point, and throw an error if so
        let chan = offer_abort!(in chan);

        // Receive a closing signature from the merchant
        let (closing_signature, chan) = chan
            .recv()
            .await
            .context("Failed to receive closing signature")?;

        // Verify the closing signature and transition into a locked state
        let (chan, locked) = if let Ok((locked, lock_message)) = started.lock(closing_signature) {
            // If the closing signature verifies, reveal our lock, secret, and blinding factor
            let chan = choose_continue!(in chan)
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
            let chan = offer_abort!(in chan);
            (chan, locked)
        } else {
            // If the closing signature does not verify, inform the merchant we are aborting
            choose_abort!(in chan return pay::Error::InvalidClosingSignature);
        };

        // Receive a pay token from the merchant, which allows us to pay again
        let (pay_token, chan) = chan
            .recv()
            .await
            .context("Failed to receive payment token")?;

        // Receive the response note (i.e. the fulfillment of the service)
        let (response_note, chan) = chan
            .recv()
            .await
            .context("Failed to receive response note")?;

        // Close the communication channel: we are done communicating with the merchant
        chan.close();

        // Unlock the payment channel using the pay token
        if let Ok(ready) = locked.unlock(pay_token) {
            todo!("store new channel state in the database")
        } else {
            return Err(anyhow::anyhow!("Invalid pay token: channel is frozen"));
        };

        // Print the response note on standard out
        if let Some(response_note) = response_note {
            println!("{}", response_note);
        }

        Ok(())
    }
}

#[async_trait]
impl Command for Refund {
    async fn run(self, rng: StdRng, config: Config) -> Result<(), anyhow::Error> {
        // A refund is merely a negative payment
        self.into_negative_pay().run(rng, config).await
    }
}
