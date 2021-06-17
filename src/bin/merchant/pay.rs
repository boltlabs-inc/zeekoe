use {async_trait::async_trait, rand::rngs::StdRng, std::sync::Arc, url::Url};

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
        client: &reqwest::Client,
        service: &Service,
        merchant_config: &MerchantConfig,
        database: &(dyn QueryMerchant + Send + Sync),
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        // Get the payment amount and note in the clear from the customer
        let (payment_amount, chan) = chan.recv().await?;
        let (_note, chan) = chan.recv().await?;

        // Determine whether to accept the payment
        let (response_url, chan): (Option<Url>, _) = match &service.approve {
            // The automatic approver approves all non-negative payments
            Approver::Automatic => {
                if payment_amount > PaymentAmount::zero() {
                    (None, choose_continue!(in chan)?)
                } else {
                    choose_abort!(in chan)?;
                    return Ok(());
                }
            }
            // A URL-based approver approves a payment iff it returns a success code
            Approver::Url(approver_url) => {
                let response = client
                    .get(
                        approver_url.join(if payment_amount > PaymentAmount::zero() {
                            "pay"
                        } else {
                            "refund"
                        })?,
                    )
                    .query(&[("amount", payment_amount.to_i64().abs() as u64)])
                    .send()
                    .await?;
                if response.status().is_success() {
                    let response_url = response.headers().get(reqwest::header::LOCATION).and_then(
                        |header_value| Some(Url::parse(header_value.to_str().ok()?).ok()?),
                    );
                    (response_url, choose_continue!(in chan)?)
                } else {
                    choose_abort!(in chan)?;
                    return Ok(());
                }
            }
        };

        // Get the nonce and pay proof (this is the start of zkAbacus.Pay)
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
                    let chan = choose_continue!(in chan)?.send(pay_token).await?;

                    // Send the response note (i.e. the fulfillment of the service) and close the
                    // connection to the customer
                    let response_note = if let Some(response_url) = response_url {
                        let response = client.get(response_url).send().await?;
                        if response.status().is_success() {
                            Some(response.text().await.unwrap_or_else(|_| String::new()))
                        } else {
                            None
                        }
                    } else {
                        Some(String::new())
                    };
                    chan.send(response_note).await?.close();
                    // TODO: send deletion command for resource acquired from confirmer. This will
                    // require restructuring this code a bit so that the deletion occurs
                    // unconditionally even if the payment fails.
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
