use {anyhow::Context, url::Url};

use zkabacus_crypto::{CustomerBalance, MerchantBalance, PaymentAmount};

use zeekoe::merchant::config::Approver;

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
pub async fn payment(
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

            // POST /pay?amount=<amount>
            // body: payment_note
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
                    // An error converting a `Location` header into a URL is an internal error
                    // (represented as `Err(None)`)
                    let response_location_str = response_location.to_str().map_err(|_| None)?;
                    let response_url = Url::parse(response_location_str).map_err(|_| None)?;

                    // Valid URL in `Location` header, so pingback after payment
                    Ok(Some(response_url))
                } else {
                    // No `Location` header, so don't pingback after payment
                    Ok(None)
                }
            } else {
                // Return the non-success body response to the customer
                Err(response.text().await.map(Some).unwrap_or(None))
            }
        }
    }
}

/// Ask the specified approver to approve the new channel balances and note (or not), returning
/// either `Ok(())` if it is approved, and `Err` if it is not approved.
///
/// Approved payments may refer to an `Option<Url>`, where the success or failure of the
/// establishment may be reported.
///
/// Rejected channels may provide an `Option<String>` indicating the reason for the channel's
/// rejection, where `None` indicates that it was rejected due to an internal error in the approver
/// service. This information is forwarded directly to the customer, so we do not provide further
/// information about the nature of the internal error, to prevent internal state leakage.
pub async fn establish(
    client: &reqwest::Client,
    approver: &Approver,
    customer_balance: &CustomerBalance,
    merchant_balance: &MerchantBalance,
    establish_note: String,
) -> Result<Option<Url>, Option<String>> {
    match approver {
        // The automatic approver approves all establishment requests
        Approver::Automatic => {
            if merchant_balance.into_inner() == 0 {
                Ok(None)
            } else {
                Err(Some(
                    "merchant declined to contribute to initial channel balance".into(),
                ))
            }
        }

        // A URL-based approver approves a payment iff it returns a success code
        Approver::Url(approver_url) => {
            let customer_balance = customer_balance.into_inner();
            let merchant_balance = merchant_balance.into_inner();

            // POST /establish?customer-amount=<customer_balance>&merchant-amount=<merchant_balance>
            // body: establish_note
            let response = client
                .post(approver_url.join("establish").map_err(|_| None)?)
                .query(&[
                    ("customer-amount", customer_balance),
                    ("merchant-amount", merchant_balance),
                ])
                .body(establish_note)
                .send()
                .await
                .map_err(|_| None)?;

            if response.status().is_success() {
                if let Some(response_location) = response.headers().get(reqwest::header::LOCATION) {
                    // An error converting a `Location` header into a URL is an internal error
                    let response_location_str = response_location.to_str().map_err(|_| None)?;
                    let response_url = Url::parse(response_location_str).map_err(|_| None)?;

                    // Valid URL in `Location` header, so pingback after establishment
                    Ok(Some(response_url))
                } else {
                    // No `Location` header, so don't pingback after establishment
                    Ok(None)
                }
            } else {
                // Return the non-success body response to the customer
                Err(response.text().await.map(Some).unwrap_or(None))
            }
        }
    }
}

/// Notify the confirmer, if any, of a payment success, and fetch a payment result, if any, to
/// return directly to the customer.
pub async fn payment_success(
    client: &reqwest::Client,
    response_url: Option<Url>,
) -> Result<Option<String>, anyhow::Error> {
    if let Some(response_url) = response_url {
        // Request the good/service at the url
        let response = client
            .get(response_url.clone())
            .send()
            .await
            .with_context(|| format!("Failed to get resource at {}", response_url.clone()))?;

        // If success, delete the resource and return it
        if response.status().is_success() {
            let body = response.text().await?;
            delete_resource(client, response_url, true).await;
            Ok(Some(body))
        } else {
            Ok(None)
        }
    } else {
        Ok(Some(String::new()))
    }
}

/// Notify the confirmer, if any, of a failure (of payment or establishment).
pub async fn failure(client: &reqwest::Client, response_url: Option<Url>) {
    if let Some(response_url) = response_url {
        delete_resource(client, response_url, false).await;
    }
}

/// Notify the confirmer, if any, of a successful establishment.
pub async fn establish_success(client: &reqwest::Client, response_url: Option<Url>) {
    if let Some(response_url) = response_url {
        delete_resource(client, response_url, true).await;
    }
}

/// Send a `DELETE` request to a resource at the specified `url`, with the query parameter
/// `?success=true` or `?success=false`, depending on the value of `success`.
///
/// This is common functionality between [`payment_success`] and [`payment_failure`].
pub async fn delete_resource(client: &reqwest::Client, url: Url, success: bool) {
    client
        .delete(url)
        .query(&[("success", success)])
        .send()
        .await
        .map(|_| ())
        .unwrap_or(());
}
