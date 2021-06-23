use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng, url::Url};

use zkabacus_crypto::{
    merchant::Config as MerchantConfig, ChannelId, CustomerBalance, MerchantBalance,
    MerchantRandomness,
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
        merchant_config: &MerchantConfig,
        database: &(dyn QueryMerchant + Send + Sync),
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

        let response_url = match approve_channel_establish(
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
            todo!("zkabacus public key"),
            todo!("merchant tezos account info"),
            todo!("customer tezos account info"),
        );

        let chan = zkabacus_initialize(
            rng,
            merchant_config,
            session_key,
            channel_id,
            chan,
            merchant_deposit,
            customer_deposit,
        )
        .await
        .context("Failed to initialize channel.")?;

        // TODO receive contract ID
        // Look up contract and ensure it is correctly funded.
        // Fund if necessary.
        // If not, abort.

        proceed!(in chan);
        offer_abort!(in chan as Merchant);
        zkabacus_activate(merchant_config, chan).await?;

        Ok(())
    }
}

async fn approve_channel_establish(
    client: &reqwest::Client,
    approver: &Approver,
    customer_balance: &CustomerBalance,
    merchant_balance: &MerchantBalance,
    payment_note: String,
) -> Result<Option<Url>, Option<String>> {
    todo!()
}

/// The core zkAbacus.Initialize and zkAbacus.Activate protocols.
async fn zkabacus_initialize(
    mut rng: StdRng,
    config: &zkabacus_crypto::merchant::Config,
    session_key: SessionKey,
    channel_id: ChannelId,
    chan: Chan<establish::Initialize>,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
) -> Result<Chan<establish::CustomerSupplyContractInfo>, anyhow::Error> {
    todo!()
}

async fn zkabacus_activate(
    config: &zkabacus_crypto::merchant::Config,
    chan: Chan<establish::Activate>,
) -> Result<(), anyhow::Error> {
    todo!()
}
