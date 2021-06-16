use async_trait::async_trait;

use zkabacus_crypto::{
    customer::{LockMessage, Ready, StartMessage},
    PaymentAmount,
};

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
    async fn run(self, config: self::Config) -> Result<(), anyhow::Error> {
        // Look up the address and current local customer state for this merchant in the database
        let address = todo!("look up address in database by `self.label`");
        let ready: Ready = todo!("look up channel state in database by `self.label`");

        // Connect and select the Pay session
        let chan = connect(&config, address).await?.choose::<1>().await?;

        // Read the contents of the note, if any
        let note = self
            .note
            .unwrap_or_else(|| zeekoe::customer::cli::Note::String(String::from("")))
            .read(config.max_note_length)?;

        // Start the payment and get the messages to send to the merchant
        let payment_units: usize = todo!("convert `self.pay: rusty_money::Money` into `usize`");

        // Start the zkAbacus core payment and get fresh proofs and commitments
        let (started, start_message) = ready.start(
            &mut rand::thread_rng(), // TODO: use parameterized Rng
            PaymentAmount::pay_merchant(payment_units),
        );

        // Send the payment amount and note to the merchant
        let chan = chan.send(payment_units).await?.send(note).await?;

        // Allow the merchant to accept or reject the payment and note
        let chan = offer_continue!(in chan else anyhow::anyhow!("merchant rejected payment amount and/or note"))?;

        // Send the initial proofs and commitments to the merchant
        let chan = chan
            .send(start_message.nonce)
            .await?
            .send(start_message.pay_proof)
            .await?
            .send(start_message.revocation_lock_commitment)
            .await?
            .send(start_message.close_state_commitment)
            .await?
            .send(start_message.state_commitment)
            .await?;

        // Allow the merchant to cancel the session at this point, and throw an error if so
        let chan = offer_continue!(in chan else anyhow::anyhow!("merchant aborted"))?;

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
            let chan = offer_continue!(in chan else anyhow::anyhow!("merchant aborted"))?;
            (chan, locked)
        } else {
            // If the closing signature does not verify, inform the merchant we are aborting
            choose_abort!(in chan)?;
            return Err(anyhow::anyhow!("could not lock channel"));
        };

        // Receive a pay token from the merchant, which allows us to pay again
        let (pay_token, chan) = chan.recv().await?;

        // Close the channel: we are done communicating with the merchant
        chan.close();

        // Unlock the payment channel using the pay token
        if let Ok(ready) = locked.unlock(pay_token) {
            todo!("store new channel state in the database")
        } else {
            return Err(anyhow::anyhow!("could not unlock: channel is frozen"));
        };

        Ok(())
    }
}

#[async_trait]
impl Command for Refund {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        self.into_negative_pay().run(config).await
    }
}
