use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng, url::Url};

use zkabacus_crypto::{
    merchant::Config as ZkAbacusConfig, ChannelId, Context as ProofContext, CustomerBalance,
    MerchantBalance, MerchantRandomness, StateCommitment,
};

use zeekoe::{
    abort,
    merchant::{
        config::{Approver, Service},
        database::QueryMerchant,
        server::SessionKey,
        Chan,
    },
    offer_abort, proceed,
    protocol::{self, establish, Party::Merchant},
};

use super::Method;

pub struct Establish;

#[async_trait]
impl Method for Establish {
    type Protocol = protocol::Establish;

    async fn run(
        &self,
        mut rng: StdRng,
        client: &reqwest::Client,
        service: &Service,
        zkabacus_config: &ZkAbacusConfig,
        _database: &(dyn QueryMerchant + Send + Sync),
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        let (customer_randomness, chan) = chan
            .recv()
            .await
            .context("Failed to receive customer randomness")?;

        let (customer_deposit, chan) = chan
            .recv()
            .await
            .context("Failed to receive customer balance")?;

        let (merchant_deposit, chan) = chan
            .recv()
            .await
            .context("Failed to receive merchant balance")?;

        let (note, chan) = chan
            .recv()
            .await
            .context("Failed to receive establish note")?;

        let _response_url = match approve_channel_establish(
            client,
            &service.approve,
            &customer_deposit,
            &merchant_deposit,
            note,
        )
        .await
        {
            Ok(response_url) => response_url,
            Err(approval_error) => {
                let error =
                    establish::Error::Rejected(approval_error.unwrap_or("internal error".into()));
                abort!(in chan return error);
            }
        };

        proceed!(in chan);

        let merchant_randomness = MerchantRandomness::new(&mut rng);
        let chan = chan
            .send(merchant_randomness)
            .await
            .context("Failed to send merchant randomness for channel ID")?;

        let channel_id = ChannelId::new(
            merchant_randomness,
            customer_randomness,
            zkabacus_config.signing_keypair().public_key(),
            todo!("merchant tezos account info"),
            todo!("customer tezos account info"),
        );

        let (chan, state_commitment) = zkabacus_initialize(
            rng,
            zkabacus_config,
            session_key,
            channel_id,
            chan,
            merchant_deposit,
            customer_deposit,
        )
        .await
        .context("Failed to initialize channel.")?;

        // TODO receive contract ID
        // Look up contract and ensure it is well-formed and correctly funded.
        // Fund if necessary.
        // If not, abort.

        proceed!(in chan);
        offer_abort!(in chan as Merchant);
        zkabacus_activate(rng, zkabacus_config, chan, state_commitment).await?;

        // TODO: send alert to response_url that channel successfully established?

        Ok(())
    }
}

/// Ask the specified approver to approve the new channel balances and note (or not), returning
/// either `Ok` if it is approved, and `Err` if it is not approved.
///
/// Approved channels may refer to an `Option<Url>`, where the *result* of the established
/// channel may be located, once the pay session completes successfully.
///
/// Rejected channels may provide an `Option<String>` indicating the reason for the channel's
/// rejection, where `None` indicates that it was rejected due to an internal error in the approver
/// service. This information is forwarded directly to the customer, so we do not provide further
/// information about the nature of the internal error, to prevent internal state leakage.
async fn approve_channel_establish(
    _client: &reqwest::Client,
    _approver: &Approver,
    _customer_balance: &CustomerBalance,
    _merchant_balance: &MerchantBalance,
    _establish_note: String,
) -> Result<Option<Url>, Option<String>> {
    todo!()
}

/// The core zkAbacus.Initialize protocol.
async fn zkabacus_initialize(
    mut rng: StdRng,
    config: &ZkAbacusConfig,
    session_key: SessionKey,
    channel_id: ChannelId,
    chan: Chan<establish::Initialize>,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
) -> Result<(Chan<establish::CustomerSupplyContractInfo>, StateCommitment), anyhow::Error> {
    let (proof, chan) = chan
        .recv()
        .await
        .context("Failed to receive establish proof")?;

    let context = ProofContext::new(&session_key.to_bytes());
    match config.initialize(
        &mut rng,
        &channel_id,
        customer_balance,
        merchant_balance,
        proof,
        &context,
    ) {
        Some((closing_signature, state_commitment)) => {
            // Send closing signature to customer
            proceed!(in chan);
            let chan = chan
                .send(closing_signature)
                .await
                .context("Failed to send initial closing signature.")?;

            // Allow customer to reject signature if it's invalid
            offer_abort!(in chan as Merchant);
            Ok((chan, state_commitment))
        }
        None => {
            let error = establish::Error::InvalidEstablishProof;
            abort!(in chan return error);
        }
    }
}

/// The core zkAbacus.Activate protocol.
async fn zkabacus_activate(
    mut rng: StdRng,
    config: &ZkAbacusConfig,
    chan: Chan<establish::Activate>,
    state_commitment: StateCommitment,
) -> Result<(), anyhow::Error> {
    // Generate and send pay token.
    let pay_token = config.activate(&mut rng, state_commitment);
    let chan = chan
        .send(pay_token)
        .await
        .context("Failed to send pay token.")?;
    chan.close();
    Ok(())
}
