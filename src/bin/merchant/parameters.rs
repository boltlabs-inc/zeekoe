use {async_trait::async_trait, rand::rngs::StdRng};

use zeekoe::{
    escrow::types::TezosKeyPair,
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
        config: &Service,
        merchant_config: &zkabacus_crypto::merchant::Config,
        database: &dyn QueryMerchant,
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        // Extract the components of the merchant's public zkAbacus parameters
        let (public_key, commitment_parameters, range_constraint_parameters) =
            merchant_config.extract_customer_config_parts();

        let tezos_public_key = TezosKeyPair::read_key_pair(&config.tezos_account)?
            .public_key()
            .clone();
        let tezos_address = tezos_public_key.hash();

        // Send those parameters to the customer
        chan.send(public_key)
            .await?
            .send(commitment_parameters)
            .await?
            .send(range_constraint_parameters)
            .await?
            .send(tezos_address)
            .await?
            .send(tezos_public_key)
            .await?
            .close();
        Ok(())
    }
}
