use {async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::{
    merchant::{config::Service, database::QueryMerchant, server::SessionKey, Chan},
    protocol,
};

use super::Method;

pub struct Parameters;

#[async_trait]
impl Method for Parameters {
    type Protocol = protocol::Parameters;

    #[allow(unused)]
    async fn run(
        &self,
        rng: StdRng,
        client: &reqwest::Client,
        config: &Service,
        merchant_config: &zkabacus_crypto::merchant::Config,
        database: &dyn QueryMerchant,
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        let customer_config = merchant_config.to_customer_config();
        // chan.send(customer_config.merchant_public_key()).await?;
        // chan.send(customer_config.revocation_commitment_parameters())
        //     .await?;
        Ok(())
    }
}
