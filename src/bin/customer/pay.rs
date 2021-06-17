use {
    async_trait::async_trait, rand::rngs::StdRng, rust_decimal::Decimal,
    rusty_money::FormattableCurrency, std::convert::TryInto,
};

use zkabacus_crypto::{customer::Ready, Context, PaymentAmount};

use zeekoe::{
    choose_abort, choose_continue,
    customer::{
        cli::{Pay, Refund},
        Config,
    },
    offer_continue,
};

use super::{connect, Command};

#[async_trait]
impl Command for Pay {
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        // Convert the payment amount appropriately
        let minor_units: i64 = self.pay.as_minor_units().ok_or_else(|| {
            anyhow::anyhow!("payment amount invalid for currency or out of range for channel")
        })?;
        let payment_amount = (if minor_units < 0 {
            PaymentAmount::pay_customer
        } else {
            PaymentAmount::pay_merchant
        })(minor_units.abs() as u64)?;

        // Look up the address and current local customer state for this merchant in the database
        let address = todo!("look up address in database by `self.label`");
        let ready: Ready = todo!("look up channel state in database by `self.label`");

        // Connect and select the Pay session
        let (session_key, chan) = connect(&config, address).await?;
        let chan = chan.choose::<1>().await?;

        // Read the contents of the note, if any
        let note = self
            .note
            .unwrap_or_else(|| zeekoe::customer::cli::Note::String(String::from("")))
            .read(config.max_note_length)?;

        // Send the payment amount and note to the merchant
        let chan = chan.send(payment_amount).await?.send(note).await?;

        // Allow the merchant to accept or reject the payment and note
        let chan = offer_continue!(in chan else return Err(anyhow::anyhow!("merchant rejected payment amount and/or note")))?;

        // Generate the shared context for proofs
        let context = Context::new(&session_key.to_bytes());

        // Start the zkAbacus core payment and get fresh proofs and commitments
        let (started, start_message) = ready.start(&mut rng, payment_amount, &context)?;

        // Send the initial proofs and commitments to the merchant
        let chan = chan
            .send(start_message.nonce)
            .await?
            .send(start_message.pay_proof)
            .await?;

        // Allow the merchant to cancel the session at this point, and throw an error if so
        let chan = offer_continue!(
            in chan else return Err(anyhow::anyhow!("merchant aborted before providing closing signature"))
        )?;

        // Receive a closing signature from the merchant
        let (closing_signature, chan) = chan.recv().await?;

        // Verify the closing signature and transition into a locked state
        let (chan, locked) = if let Ok((locked, lock_message)) = started.lock(closing_signature) {
            // If the closing signature verifies, reveal our lock, secret, and blinding factor
            let chan = choose_continue!(in chan)?;
            let chan = chan
                .send(lock_message.revocation_lock)
                .await?
                .send(lock_message.revocation_secret)
                .await?
                .send(lock_message.revocation_lock_blinding_factor)
                .await?;

            // Allow the merchant to cancel the session at this point, and throw an error if so
            let chan = offer_continue!(
                in chan else return Err(anyhow::anyhow!("merchant aborted before providing pay token"))
            )?;
            (chan, locked)
        } else {
            // If the closing signature does not verify, inform the merchant we are aborting
            choose_abort!(in chan)?;
            return Err(anyhow::anyhow!("could not lock channel"));
        };

        // Receive a pay token from the merchant, which allows us to pay again
        let (pay_token, chan) = chan.recv().await?;

        // Receive the response note (i.e. the fulfillment of the service)
        let (response_note, chan) = chan.recv().await?;

        // Close the communication channel: we are done communicating with the merchant
        chan.close();

        // Unlock the payment channel using the pay token
        if let Ok(ready) = locked.unlock(pay_token) {
            todo!("store new channel state in the database")
        } else {
            return Err(anyhow::anyhow!("could not unlock: channel is frozen"));
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
