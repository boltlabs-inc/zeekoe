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

    async fn run(
        &self,
        _rng: StdRng,
        _client: &reqwest::Client,
        _config: &Service,
        merchant_config: &zkabacus_crypto::merchant::Config,
        _database: &(dyn QueryMerchant + Send + Sync),
        _session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        let (public_key, commitment_parameters, range_proof_parameters) =
            merchant_config.extract_customer_config_parts();
        chan.send(public_key)
            .await?
            .send(commitment_parameters)
            .await?
            .send(range_proof_parameters)
            .await?
            .close();
        Ok(())
    }
}
