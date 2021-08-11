mod establish {
    use crate::escrow::{
        notify::Level,
        types::{ContractId, TezosFundingAccount, TezosPublicKey},
    };
    use zkabacus_crypto::{ChannelId, CustomerBalance, MerchantBalance, PublicKey};

    pub enum Error {}

    #[allow(unused)]
    pub struct CustomerFundingInformation {
        pub balance: CustomerBalance,
        pub account: TezosFundingAccount,
        pub public_key: TezosPublicKey,
    }

    #[allow(unused)]
    pub struct MerchantFundingInformation {
        pub balance: MerchantBalance,
        pub account: TezosFundingAccount,
        pub public_key: TezosPublicKey,
    }

    /// Originate a contract on chain.
    ///
    /// This call will wait until the contract is confirmed at depth.
    /// It returns the new [`ContractId`] and the [`Level`] of the block that contains the
    /// originated contract.
    ///
    /// This is typically called by the customer.
    #[allow(unused)]
    pub async fn originate(
        merchant_funding_info: &MerchantFundingInformation,
        customer_funding_info: &CustomerFundingInformation,
        merchant_public_key: &PublicKey,
        channel_id: &ChannelId,
    ) -> Result<(ContractId, Level), Error> {
        todo!()
    }

    /// Verify that the contract specified by [`ContractId`] has been correctly originated on
    /// chain with respect to the expected values.
    ///
    /// Check that
    /// - Contract encodes the expected zkChannels contract
    /// - Contract storage is correctly instantiated
    /// - Contract is confirmed on chain to the expected depth
    /// This call will wait until the contract is confirmed at depth.
    ///
    /// This function is typically called by the merchant.
    #[allow(unused)]
    pub async fn verify_origination(
        contract_id: &ContractId,
        merchant_funding_info: &MerchantFundingInformation,
        customer_funding_info: &CustomerFundingInformation,
        merchant_public_key: &PublicKey,
        channel_id: &ChannelId,
    ) -> Result<(), Error> {
        todo!()
    }
}
