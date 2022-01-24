use crate::{
    merchant::{Chan, Config},
    protocol,
};

pub struct Parameters;

impl Parameters {
    pub async fn run(
        &self,
        config: &Config,
        merchant_config: &zkabacus_crypto::merchant::Config,
        chan: Chan<protocol::Parameters>,
    ) -> Result<(), anyhow::Error> {
        // Extract the components of the merchant's public zkAbacus parameters
        let (public_key, commitment_parameters, range_constraint_parameters) =
            merchant_config.extract_customer_config_parts();

        // Extract public parts of the tezos parameters
        let tezos_key_material = config.load_tezos_key_material()?;
        let tezos_public_key = tezos_key_material.into_keypair().0;
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
