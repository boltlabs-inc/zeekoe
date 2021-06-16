use {async_trait::async_trait, std::sync::Arc};

use zeekoe::{
    merchant::{config::Approver, Chan, Config},
    protocol,
};

use super::Method;

pub struct Pay {
    pub approve: Arc<Approver>,
}

#[async_trait]
impl Method for Pay {
    type Protocol = protocol::Pay;

    async fn run(
        &self,
        config: &Config,
        merchant_config: &zkabacus_crypto::merchant::Config,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        todo!()
    }
}
