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
        _rng: StdRng,
        _client: &reqwest::Client,
        _config: &Service,
        merchant_config: &zkabacus_crypto::merchant::Config,
        database: &dyn QueryMerchant,
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        // Extract the components of the merchant's public zkAbacus parameters
        let (public_key, commitment_parameters, range_proof_parameters) =
            merchant_config.extract_customer_config_parts();

        // Send those parameters to the customer
        chan.send(public_key)
            .await?
            .send(commitment_parameters)
            .await?
            .send(range_proof_parameters)
            .await?
            // TODO: Send the merchant's tz1 address and tezos public key
            .close();
        Ok(())
    }
}
