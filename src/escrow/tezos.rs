mod establish {
    use crate::escrow::{
        notify::Level,
        types::{ContractId, TezosFundingAccount, TezosPublicKey},
    };
    use zkabacus_crypto::{ChannelId, CustomerBalance, MerchantBalance, PublicKey};
    use {
        serde::{Deserialize, Serialize},
        thiserror::Error,
    };

    /// Set of errors that may arise while establishing a zkChannel.
    ///
    /// Note: Errors noting that an operation has failed to be confirmed on chain only arise when
    /// a specified timeout period has passed. In general, the functions in this module will wait
    /// until operations are successfully confirmed.
    #[derive(Clone, Debug, Error, Serialize, Deserialize)]
    pub enum Error {
        #[error("Encountered a network error")]
        NetworkFailure,
        #[error("The contract has not been confirmed on chain (id = {0})")]
        ContractOriginationFailed(ContractId),
        #[error(
            "The contract is not a valid zkChannels contract with expected storage (id = {0})"
        )]
        ContractOriginationInvalid(ContractId),
        #[error("The contract did not receive confirmed customer funding (id = {0})")]
        CustomerFundingFailed(ContractId),
        #[error("The contract did not receive confirmed merchant funding (id = {0})")]
        MerchantFundingFailed(ContractId),
    }

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

    /// Call the `addFunding` entrypoint with the [`CustomerFundingInformation`].
    ///
    /// This is called by the customer.
    #[allow(unused)]
    pub async fn add_customer_funding(
        contract_id: &ContractId,
        customer_funding_info: &CustomerFundingInformation,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Add merchant funding via the `addFunding` entrypoint to the given [`ContractId`],
    /// according to the [`MerchantFundingInformation`], but only after checking that:
    /// - The contract specified by [`ContractId`] has been correctly originated on
    ///   chain with respect to the expected values.
    /// - The contract has been successfully funded by the customer via the `addFunding`
    ///   entrypoint
    ///
    /// Correct origination requires that:
    /// - Contract encodes the expected zkChannels contract
    /// - Contract storage is correctly instantiated
    /// - Contract is confirmed on chain to the expected depth
    ///
    /// Correct funding requires that:
    /// - The `addFunding` operation is the latest operation to be applied to the contract
    /// - The `addFunding` operation is confirmed on chain to the expected depth
    ///
    /// If the expected merchant funding is 0, this verifies that the contract status is OPEN.
    /// If the merchant funding is non-zero, this verifies that the contract status is
    /// AWAITING_FUNDING, then funds the contract.
    ///
    /// This function will wait until all operations are confirmed at depth
    /// and is called by the merchant.
    #[allow(unused)]
    pub async fn add_merchant_funding(
        contract_id: &ContractId,
        merchant_funding_info: &MerchantFundingInformation,
        customer_funding_info: &CustomerFundingInformation,
        merchant_public_key: &PublicKey,
        channel_id: &ChannelId,
    ) -> Result<(), Error> {
        todo!()
    }
}
