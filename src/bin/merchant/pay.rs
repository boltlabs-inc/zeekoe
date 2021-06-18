use {
    anyhow::{anyhow, Context},
    async_trait::async_trait,
    dialectic::prelude::*,
    rand::rngs::StdRng,
    std::sync::Arc,
    url::Url,
};

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

use zkabacus_crypto::{merchant::Config as MerchantConfig, Context as ProofContext, PaymentAmount};

use super::Method;

pub struct Pay {
    pub approve: Arc<Approver>,
}

#[async_trait]
impl Method for Pay {
    type Protocol = protocol::Pay;

    async fn run(
        &self,
        rng: StdRng,
        client: &reqwest::Client,
        service: &Service,
        merchant_config: &MerchantConfig,
        database: &(dyn QueryMerchant + Send + Sync),
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        // Get the payment amount and note in the clear from the customer
        let (payment_amount, chan) = chan.recv().await?;
        let (note, chan) = chan.recv().await?;

        // Determine whether to accept the payment
        let (response_url, chan) =
            match approve_payment(client, &service.approve, &payment_amount, note).await {
                Ok(response_url) => {
                    let chan = choose_continue!(in chan)?;
                    (response_url, chan)
                }
                Err(approval_error) => {
                    // If the payment was not approved, indicate to the client why
                    chan.choose::<0>()
                        .await?
                        .send(format!(
                            "Payment rejected: {}",
                            match approval_error {
                                None => "internal approver service error".into(),
                                Some(err) => err,
                            }
                        ))
                        .await?
                        .close();
                    return Ok(());
                }
            };

        // Run the zkAbacus.Pay protocol
        let pay_result = zkabacus_pay(
            rng,
            merchant_config,
            database,
            session_key,
            chan,
            payment_amount,
        )
        .await;

        match pay_result {
            Ok(chan) => {
                // Send the response note (i.e. the fulfillment of the service) and close the
                // connection to the customer
                let response_note = payment_success(client, response_url).await;
                chan.send(response_note).await?.close();
                Ok(())
            }
            Err(err) => {
                payment_failure(client, response_url).await;
                Err(err)
            }
        }
    }
}

/// Ask the specified approver to approve the payment amount and note (or not), returning either
/// `Ok` if it is approved, and `Err` if it is not approved.
///
/// Approved payments may refer to an `Option<Url>`, where the *result* of the payment may be
/// located, once the pay session completes successfully.
///
/// Rejected payments may provide an `Option<String>` indicating the reason for the payment's
/// rejection, where `None` indicates that it was rejected due to an internal error in the approver
/// service. This information is forwarded directly to the customer, so we do not provide further
/// information about the nature of the internal error, to prevent internal state leakage.
async fn approve_payment(
    client: &reqwest::Client,
    approver: &Approver,
    payment_amount: &PaymentAmount,
    payment_note: String,
) -> Result<Option<Url>, Option<String>> {
    match approver {
        // The automatic approver approves all non-negative payments
        Approver::Automatic => {
            if payment_amount > &PaymentAmount::zero() {
                Ok(None)
            } else {
                Err(Some("amount must be non-negative".into()))
            }
        }
        // A URL-based approver approves a payment iff it returns a success code
        Approver::Url(approver_url) => {
            let amount = payment_amount.to_i64().abs();
            let response = client
                .post(
                    approver_url
                        .join(if payment_amount > &PaymentAmount::zero() {
                            "pay"
                        } else {
                            "refund"
                        })
                        .map_err(|_| None)?,
                )
                .query(&[("amount", amount)])
                .body(payment_note)
                .send()
                .await
                .map_err(|_| None)?;
            if response.status().is_success() {
                if let Some(response_location) = response.headers().get(reqwest::header::LOCATION) {
                    let response_location_str = response_location.to_str().map_err(|_| None)?;
                    let response_url = Url::parse(response_location_str).map_err(|_| None)?;
                    Ok(Some(response_url))
                } else {
                    Ok(None)
                }
            } else {
                Err(response.text().await.map(Some).unwrap_or(None))
            }
        }
    }
}

/// Notify the confirmer, if any, of a payment success, and fetch a payment result, if any, to
/// return directly to the customer.
async fn payment_success(client: &reqwest::Client, response_url: Option<Url>) -> Option<String> {
    if let Some(response_url) = response_url {
        let response = client.get(response_url.clone()).send().await.ok()?;
        if response.status().is_success() {
            let body = response.text().await.ok()?;
            delete_resource(client, response_url, true).await;
            Some(body)
        } else {
            None
        }
    } else {
        Some(String::new())
    }
}

/// Notify the confirmer, if any, of a payment failure.
async fn payment_failure(client: &reqwest::Client, response_url: Option<Url>) {
    if let Some(response_url) = response_url {
        delete_resource(client, response_url, false).await;
    }
}

/// Send a `DELETE` request to a resource at the specified `url`, with the query parameter
/// `?success=true` or `?success=false`, depending on the value of `success`.
///
/// This is common functionality between [`payment_success`] and [`payment_failure`].
async fn delete_resource(client: &reqwest::Client, url: Url, success: bool) {
    client
        .delete(url)
        .query(&[("success", success)])
        .send()
        .await
        .map(|_| ())
        .unwrap_or(());
}

/// The core zkAbacus.Pay protocol.
async fn zkabacus_pay(
    mut rng: StdRng,
    merchant_config: &MerchantConfig,
    database: &(dyn QueryMerchant + Send + Sync),
    session_key: SessionKey,
    chan: Chan<protocol::pay::CustomerStartPayment>,
    payment_amount: PaymentAmount,
) -> Result<Chan<Session! { recv Option<String> }>, anyhow::Error> {
    // Get the nonce and pay proof (this is the start of zkAbacus.Pay)
    let (nonce, chan) = chan.recv().await?;
    let (pay_proof, chan) = chan.recv().await?;

    // Generate the shared context for the proof
    let context = ProofContext::new(&session_key.to_bytes());

    if let Some((unrevoked, closing_signature)) =
        merchant_config.allow_payment(&mut rng, payment_amount, &nonce, pay_proof, &context)
    {
        // Proof verified, so check the nonce
        if database.insert_nonce(&nonce).await? {
            // Nonce was already present, so reject the payment
            choose_abort!(in chan)?;
            return Err(anyhow::anyhow!("nonce was already present in database"));
        } else {
            // Nonce was fresh, so continue
            let chan = choose_continue!(in chan)?.send(closing_signature).await?;

            // Offer the customer the choice of whether to continue after receiving the signature
            let chan = offer_continue!(
                in chan else return Err(anyhow::anyhow!("customer rejected closing signature"))
            )?;

            // Receive the customer's revealed lock, secret, and blinding factor
            let (revocation_lock, chan) = chan.recv().await?;
            let (revocation_secret, chan) = chan.recv().await?;
            let (revocation_blinding_factor, chan) = chan.recv().await?;

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
                    .await?;

                // Abort if the revocation lock was already present in the database
                if !prior_revocations.is_empty() {
                    choose_abort!(in chan)?;
                    return Err(anyhow::anyhow!(
                        "revocation lock was already present in database"
                    ));
                }

                // The revealed information was correct; issue the pay token
                let chan = choose_continue!(in chan)?.send(pay_token).await?;
                return Ok(chan);
            } else {
                // Incorrect information; abort the session and do not issue a pay token. This
                // has the effect of freezing the channel, since the nonce has been recorded,
                // but the customer has no new state to pay from.
                choose_abort!(in chan)?;
                return Err(anyhow::anyhow!(
                    "invalid revealed lock/secret/blinding-factor triple"
                ));
            }
        }
    } else {
        // Proof didn't verify, so don't check the nonce
        choose_abort!(in chan)?;
        return Err(anyhow::anyhow!("payment proof did not verify"));
    };
}
