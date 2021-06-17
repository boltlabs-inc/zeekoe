use {async_trait::async_trait, rand::rngs::StdRng, std::sync::Arc};

use zeekoe::{
    choose_abort, choose_continue,
    merchant::{
        config::{Approver, Service},
        database::QueryMerchant,
        server::SessionKey,
        Chan,
    },
    offer_continue, protocol,
};
use zkabacus_crypto::{merchant::Config as MerchantConfig, Context, PaymentAmount};

use super::Method;

pub struct Pay {
    pub approve: Arc<Approver>,
}

#[async_trait]
impl Method for Pay {
    type Protocol = protocol::Pay;

    async fn run(
        &self,
        mut rng: StdRng,
        service: &Service,
        merchant_config: &MerchantConfig,
        database: &(dyn QueryMerchant + Send + Sync),
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        // Get the payment amount and note in the clear from the customer
        let (payment_amount, chan) = chan.recv().await?;
        let (_note, chan) = chan.recv().await?;

        // Signal that the payment was not approved
        struct NotApproved;

        // Determine whether to accept the payment
        let approved = match &service.approve {
            // The automatic approver approves all non-negative payments
            Approver::Automatic => payment_amount > PaymentAmount::zero(),
            Approver::Url(_url) => {
                todo!("External approvers are not yet supported")
            }
        };

        // Continue only if we should permit the payment as stated
        let chan = if !approved {
            choose_abort!(in chan)?;
            return Ok(());
        } else {
            choose_continue!(in chan)?
        };

        let (nonce, chan) = chan.recv().await?;
        let (pay_proof, chan) = chan.recv().await?;

        // Generate the shared context for the proof
        let context = Context::new(&session_key.to_bytes());

        if let Some((unrevoked, closing_signature)) =
            merchant_config.allow_payment(&mut rng, payment_amount, &nonce, pay_proof, &context)
        {
            // Proof verified, so check the nonce
            if database.insert_nonce(&nonce).await? {
                // Nonce was already present, so reject the payment
                choose_abort!(in chan)?;
                return Ok(());
            } else {
                // Nonce was fresh, so continue
                let chan = choose_continue!(in chan)?.send(closing_signature).await?;

                // Offer the customer the choice of whether to continue after receiving the signature
                let chan = offer_continue!(in chan else return Ok(()))?;

                // Receive the customer's revealed lock, secret, and blinding factor
                let (revocation_lock, chan) = chan.recv().await?;
                let (revocation_secret, chan) = chan.recv().await?;
                let (revocation_blinding_factor, chan) = chan.recv().await?;

                // Check to see if the revocation lock was already present in the database
                let prior_revocations = database
                    .insert_revocation(&revocation_lock, Some(&revocation_secret))
                    .await?;

                // Abort if the revocation lock was already present in the database
                if !prior_revocations.is_empty() {
                    choose_abort!(in chan)?;
                    return Ok(());
                }

                // Validate the received information
                if let Ok(pay_token) = unrevoked.complete_payment(
                    &mut rng,
                    &revocation_lock,
                    &revocation_secret,
                    &revocation_blinding_factor,
                ) {
                    // The revealed information was correct; issue the pay token
                    let chan = choose_continue!(in chan)?;
                    chan.send(pay_token).await?.close();
                } else {
                    // Incorrect information; abort the session and do not issue a pay token. This
                    // has the effect of freezing the channel, since the nonce has been recorded,
                    // but the customer has no new state to pay from.
                    choose_abort!(in chan)?;
                }
            }
        } else {
            // Proof didn't verify, so don't check the nonce
            choose_abort!(in chan)?;
            return Ok(());
        };

        Ok(())
    }
}
