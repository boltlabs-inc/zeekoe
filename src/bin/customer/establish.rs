use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng, std::convert::TryInto};

use zkabacus_crypto::{
    customer::{Inactive, Ready, Requested},
    ChannelId, Context as ProofContext, CustomerBalance, CustomerRandomness, MerchantBalance,
};

use zeekoe::{
    abort,
    customer::{
        cli::Establish,
        client::{SessionKey, ZkChannelAddress},
        Chan, ChannelName, Config,
    },
    offer_abort, proceed,
    protocol::{
        establish,
        Party::{Customer, Merchant},
    },
};

use super::{connect, database, Command};

#[async_trait]
impl Command for Establish {
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        // Connect and select the Establish session
        let (session_key, chan) = connect(&config, self.merchant)
            .await
            .context("Failed to connect to merchant")?;
        let chan = chan
            .choose::<2>()
            .await
            .context("Failed to select channel establishment session")?;

        // TODO: send customer chain-specific things

        let customer_randomness = CustomerRandomness::new(&mut rng);
        let chan = chan
            .send(customer_randomness)
            .await
            .context("Failed to send customer randomness for channel ID")?;

        // Format deposit amounts as the correct types
        let customer_deposit = CustomerBalance::try_new(
            self.deposit
                .as_minor_units()
                .ok_or(establish::Error::InvalidDeposit(Customer))?
                .try_into()?,
        )
        .map_err(|_| establish::Error::InvalidDeposit(Customer))?;

        let merchant_deposit: MerchantBalance =
            MerchantBalance::try_new(match self.merchant_deposit {
                None => 0,
                Some(d) => d
                    .as_minor_units()
                    .ok_or(establish::Error::InvalidDeposit(Merchant))?
                    .try_into()?,
            })
            .map_err(|_| establish::Error::InvalidDeposit(Merchant))?;

        // Read the contents of the note, if any
        let note = self
            .note
            .unwrap_or_else(|| zeekoe::customer::cli::Note::String(String::from("")))
            .read(config.max_note_length)?;

        // Send the request for the funding of the channel
        let chan = chan
            .send(customer_deposit)
            .await
            .context("Failed to send customer deposit amount")?
            .send(merchant_deposit)
            .await
            .context("Failed to send merchant deposit amount")?
            .send(note)
            .await
            .context("Failed to send channel establishment note")?;

        // Allow the merchant to reject the funding of the channel, else continue
        offer_abort!(in chan as Customer);

        // TODO: receive merchant chain-specific things

        let (merchant_randomness, chan) = chan
            .recv()
            .await
            .context("Failed to recieve merchant randomness for channel ID")?;

        let channel_id = ChannelId::new(
            merchant_randomness,
            customer_randomness,
            todo!("zkabacus public key"),
            todo!("merchant tezos account info"),
            todo!("customer tezos account info"),
        );

        // Connect to the customer database
        let database = database(&config).await?;

        // Run a **separate** session to get the merchant's public parameters
        let customer_config: zkabacus_crypto::customer::Config =
            get_parameters(&config, &self.merchant).await?;

        let (inactive, chan) = zkabacus_initialize(
            rng,
            customer_config,
            session_key,
            channel_id,
            chan,
            merchant_deposit,
            customer_deposit,
        )
        .await
        .context("Failed to initialize channel.")?;

        // Store the inactive channel state in the database
        match database
            .new_channel(
                &self
                    .label
                    .unwrap_or_else(|| ChannelName::new(format!("{}", self.merchant))),
                &self.merchant,
                inactive,
            )
            .await
        {
            Ok(_) => todo!(),
            Err(_) => todo!(),
        }

        // TODO: initialize contract on-chain via escrow agent.
        // TODO: fund contract via escrow agent.
        // TODO: send contract id to merchant

        // Allow the merchant to indicate whether it funded the channel.
        offer_abort!(in chan as Customer);

        // TODO: check that merchant funding was successful. If not, recommend unilateral close.
        let merchant_funding_successful: bool = todo!("query tezos to check for merchant funding.");

        // for now, assume it was.
        if merchant_funding_successful {
            abort!(in chan return establish::Error::FailedMerchantFunding);
        }
        proceed!(in chan);

        let _ready = zkabacus_activate(customer_config, inactive, chan).await?;

        // TODO: store ready state in db.

        Ok(())
    }
}

/// Fetch the merchant's public parameters.
async fn get_parameters(
    config: &Config,
    address: &ZkChannelAddress,
) -> Result<zkabacus_crypto::customer::Config, anyhow::Error> {
    todo!()
}

/// The core zkAbacus.Initialize and zkAbacus.Activate protocols.
async fn zkabacus_initialize(
    mut rng: StdRng,
    config: zkabacus_crypto::customer::Config,
    session_key: SessionKey,
    channel_id: ChannelId,
    chan: Chan<establish::Initialize>,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
) -> Result<(Inactive, Chan<establish::CustomerSupplyContractInfo>), anyhow::Error> {
    let context = ProofContext::new(&session_key.to_bytes());

    let (requested, proof) = Requested::new(
        &mut rng,
        config,
        channel_id,
        merchant_balance,
        customer_balance,
        &context,
    );

    let chan = chan.send(proof).await.context("Failed to send proof")?;

    offer_abort!(in chan as Customer);

    let (closing_signature, chan) = chan
        .recv()
        .await
        .context("Failed to receive closing signature")?;

    match requested.complete(closing_signature) {
        Ok(inactive) => {
            proceed!(in chan);
            return Ok((inactive, chan));
        }
        Err(_) => {
            abort!(in chan return establish::Error::InvalidClosingSignature);
        }
    }
}

async fn zkabacus_activate(
    config: zkabacus_crypto::customer::Config,
    inactive: Inactive,
    chan: Chan<establish::Activate>,
) -> Result<Ready, anyhow::Error> {
    let (blinded_pay_token, chan) = chan
        .recv()
        .await
        .context("Failed to receive blinded pay token.")?;

    chan.close();

    match inactive.activate(blinded_pay_token) {
        Ok(ready) => Ok(ready),
        Err(_) => Err(establish::Error::InvalidPayToken.into()),
    }
}
