use async_trait::async_trait;

use zeekoe::{
    merchant::{database::QueryMerchant, Chan, Config},
    protocol,
};

use super::Method;

pub struct Parameters;

#[async_trait]
impl Method for Parameters {
    type Protocol = protocol::Parameters;

    async fn run(
        &self,
        config: &Config,
        merchant_config: &zkabacus_crypto::merchant::Config,
        database: &(dyn QueryMerchant + Send + Sync),
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        let customer_config = merchant_config.to_customer_config();
        // chan.send(customer_config.merchant_public_key()).await?;
        // chan.send(customer_config.revocation_commitment_parameters())
        //     .await?;
        Ok(())
    }
}
