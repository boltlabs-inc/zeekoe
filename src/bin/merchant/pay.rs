use {
    async_trait::async_trait, dialectic::prelude::*, rand::rngs::StdRng, std::sync::Arc, url::Url,
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
        let (note, chan) = chan.recv().await?;

        // Determine whether to accept the payment
        let (response_url, chan) =
            match approve_payment(client, service, &payment_amount, note).await {
                Ok(response_url) => {
                    let chan = choose_continue!(in chan)?;
                    (response_url, chan)
                }
                Err(approval_error) => {
                    choose_abort!(in chan)?;
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
                let response_note = get_success_note(client, response_url).await;
                chan.send(response_note).await?.close();
                Ok(())
            }
            Err(err) => {
                notify_failure(client, response_url).await;
                Err(err)
            }
        }
    }
}

async fn approve_payment(
    client: &reqwest::Client,
    service: &Service,
    payment_amount: &PaymentAmount,
    payment_note: String,
) -> Result<Option<Url>, String> {
    match &service.approve {
        // The automatic approver approves all non-negative payments
        Approver::Automatic => {
            if payment_amount > &PaymentAmount::zero() {
                Ok(None)
            } else {
                Err("payment amount must be non-negative".into())
            }
        }
        // A URL-based approver approves a payment iff it returns a success code
        Approver::Url(approver_url) => {
            let amount = payment_amount.to_i64().abs() as u64;
            let response = client
                .post(
                    approver_url.join(if payment_amount > &PaymentAmount::zero() {
                        "pay"
                    } else {
                        "refund"
                    }).map_err(|err| {
                        String::from("payment rejected due to misconfigured approver service: could not parse approver URL")
                    })?,
                )
                .query(&[("amount", amount)])
                .body(payment_note)
                .send()
                .await
                .map_err(|err| format!("payment rejected due to approver service error: {}", err))?;
            if response.status().is_success() {
                let response_url = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|header_value| Some(Url::parse(header_value.to_str().ok()?).ok()?));
                Ok(response_url)
            } else {
                Err(response.text().await.unwrap_or_else(|_| {
                    "payment rejected by approver service for unknown reason (non-UTF-8 error response)".into()
                }))
            }
        }
    }
}

async fn get_success_note(client: &reqwest::Client, response_url: Option<Url>) -> Option<String> {
    if let Some(response_url) = response_url {
        let response = client.get(response_url.clone()).send().await.ok()?;
        if response.status().is_success() {
            let body = response.text().await.ok()?;
            client
                .delete(response_url)
                .query(&[("success", true)])
                .send()
                .await
                .map(|_| ())
                .unwrap_or(());
            Some(body)
        } else {
            None
        }
    } else {
        Some(String::new())
    }
}

async fn notify_failure(client: &reqwest::Client, response_url: Option<Url>) {
    if let Some(response_url) = response_url {
        client
            .delete(response_url)
            .query(&[("success", false)])
            .send()
            .await
            .map(|_| ())
            .unwrap_or(());
    }
}

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
    let context = Context::new(&session_key.to_bytes());

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
