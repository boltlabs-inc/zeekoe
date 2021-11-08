use {anyhow::Context, rand::rngs::StdRng, url::Url};

use zeekoe::{
    abort,
    merchant::{config::Service, database::QueryMerchant, server::SessionKey, Chan},
    offer_abort, proceed,
    protocol::{self, pay, Party::Merchant},
    timeout::WithTimeout,
};

use zkabacus_crypto::{Context as ProofContext, PaymentAmount};

use super::approve;

pub struct Pay;

impl Pay {
    pub async fn run(
        &self,
        rng: StdRng,
        client: &reqwest::Client,
        service: &Service,
        database: &dyn QueryMerchant,
        session_key: SessionKey,
        chan: Chan<protocol::Pay>,
    ) -> Result<(), anyhow::Error> {
        // Get the payment amount and context note from the customer
        let (payment_amount, chan) = chan
            .recv()
            .with_timeout(service.message_timeout)
            .await
            .context("Payment timed out while receiving payment amount")??;
        let (payment_note, chan) = chan
            .recv()
            .with_timeout(service.message_timeout)
            .await
            .context("Payment timed out while receiving payment note")??;

        // Query approver service to determine whether to allow the payment
        let (response_url, chan) =
            approve_payment(payment_amount, payment_note, chan, client, service).await?;

        // Run the zkAbacus.Pay protocol
        // Timeout is set to 10 messages, which includes all sent & received messages and aborts
        let maybe_chan = zkabacus_pay(rng, database, session_key, chan, payment_amount)
            .with_timeout(10 * service.message_timeout)
            .await
            .context("Payment timed out while updating channel status")?;

        provide_service(response_url, maybe_chan, client).await?;

        Ok(())
    }
}

/// Query the approver service using payment details provided by the customer to determine whether
/// to allow the payment. If not, terminate the pay session.
async fn approve_payment(
    payment_amount: PaymentAmount,
    payment_note: String,
    chan: Chan<pay::GetPaymentApproval>,
    client: &reqwest::Client,
    service: &Service,
) -> Result<(Option<Url>, Chan<pay::CustomerStartPayment>), anyhow::Error> {
    // Determine whether to accept the payment
    let response_url =
        match approve::payment(client, &service.approve, &payment_amount, payment_note).await {
            Ok(response_url) => response_url,
            Err(approval_error) => {
                // If the payment was not approved, indicate to the client why
                let error =
                    pay::Error::Rejected(approval_error.unwrap_or_else(|| "internal error".into()));
                abort!(in chan return error);
            }
        };

    proceed!(in chan);

    Ok((response_url, chan))
}

/// Inform the approver service whether the payment succeeded and pass the resulting fulfillment
/// to the customer.
async fn provide_service(
    response_url: Option<Url>,
    maybe_chan: Result<Chan<pay::MerchantProvideService>, anyhow::Error>,
    client: &reqwest::Client,
) -> Result<(), anyhow::Error> {
    match maybe_chan {
        Ok(chan) => {
            // Send the response note (i.e. the fulfillment of the service) and close the
            // connection to the customer
            let response_note = approve::payment_success(client, response_url).await;
            let (note, result) = match response_note {
                Err(err) => (None, Err(err)),
                Ok(o) => (o, Ok(())),
            };
            chan.send(note)
                .await
                .context("Failed to send response note")?
                .close();
            result
        }
        Err(err) => {
            approve::failure(client, response_url).await;
            Err(err)
        }
    }
}

/// The core zkAbacus.Pay protocol: provide the customer with a valid, updated channel state.
async fn zkabacus_pay(
    mut rng: StdRng,
    database: &dyn QueryMerchant,
    session_key: SessionKey,
    chan: Chan<pay::CustomerStartPayment>,
    payment_amount: PaymentAmount,
) -> Result<Chan<pay::MerchantProvideService>, anyhow::Error> {
    // Retrieve zkAbacus merchant config
    let merchant_config = database.fetch_or_create_config(&mut rng).await?;

    // Generate the shared context for the proof
    let context = ProofContext::new(&session_key.to_bytes());

    // Get the nonce and pay proof (this is the start of zkAbacus.Pay)
    let (nonce, chan) = chan.recv().await.context("Failed to receive nonce")?;
    let (pay_proof, chan) = chan.recv().await.context("Failed to receive pay proof")?;

    if let Some((unrevoked, closing_signature)) =
        merchant_config.allow_payment(&mut rng, payment_amount, &nonce, pay_proof, &context)
    {
        // Proof verified, so check the nonce
        if !database
            .insert_nonce(&nonce)
            .await
            .context("Failed to insert nonce in database")?
        {
            // Nonce was already present, so reject the payment
            abort!(in chan return pay::Error::ReusedNonce);
        } else {
            // Nonce was fresh, so continue
            proceed!(in chan);
            let chan = chan
                .send(closing_signature)
                .await
                .context("Failed to send closing signature")?;

            // Offer the customer the choice of whether to continue after receiving the signature
            offer_abort!(in chan as Merchant);

            // Receive the customer's revealed lock, secret, and blinding factor
            let (revocation_lock, chan) = chan
                .recv()
                .await
                .context("Failed to send revocation lock")?;
            let (revocation_secret, chan) = chan
                .recv()
                .await
                .context("Failed to send revocation secret")?;
            let (revocation_blinding_factor, chan) = chan
                .recv()
                .await
                .context("Failed to send revocation blinding factor")?;

            // Validate the received information
            if let Ok(pay_token) = unrevoked.complete_payment(
                &mut rng,
                &revocation_lock,
                &revocation_secret,
                &revocation_blinding_factor,
            ) {
                // Check to see if the revocation lock was already present in the database
                let prior_revocations = database
                    .insert_revocation(&revocation_lock, Some(&revocation_secret))
                    .await
                    .context("Failed to insert revocation lock/secret pair in database")?;

                // Abort if the revocation lock was already present in the database
                if !prior_revocations.is_empty() {
                    abort!(in chan return pay::Error::ReusedRevocationLock);
                }

                // The revealed information was correct; issue the pay token
                proceed!(in chan);
                let chan = chan
                    .send(pay_token)
                    .await
                    .context("Failed to send pay token")?;

                // Return the channel, ready for the finalization of the outer protocol
                Ok(chan)
            } else {
                // Incorrect information; abort the session and do not issue a pay token. This
                // has the effect of freezing the channel, since the nonce has been recorded,
                // but the customer has no new state to pay from.
                abort!(in chan return pay::Error::InvalidRevocationOpening);
            }
        }
    } else {
        // Proof didn't verify, so don't check the nonce
        abort!(in chan return pay::Error::InvalidPayProof);
    }
}
