use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng, std::convert::TryInto};

use zkabacus_crypto::{
    ChannelId, ClosingSignature, CustomerBalance, EstablishProof, MerchantBalance, PayToken,
};

use zeekoe::{
    abort,
    customer::{cli::Establish, Config},
    offer_abort, proceed,
    protocol::{
        establish,
        Party::{Customer, Merchant},
    },
};

use super::{connect, Command};

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

        // TODO: send customer chain-specific things + randomness

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

        // TODO: receive merchant chain-specific things + randomness

        let channel_id: ChannelId = todo!("generate channel id");

        Ok(())
    }
}
