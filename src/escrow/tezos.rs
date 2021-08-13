mod establish {
    use crate::escrow::{notify::Level, types::*};
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
        #[error("Unable to reclaim customer funding because the operation is invalid (id = {0})")]
        CustomerReclaimInvalid(ContractId),
        #[error("Unable to reclaim merchant funding because the operation is invalid (id = {0})")]
        MerchantReclaimInvalid(ContractId),
        #[error("Failed to reclaim customer funding (id = {0})")]
        CustomerReclaimFailed(ContractId),
        #[error("Failed to reclaim merchant funding (id = {0})")]
        MerchantReclaimFailed(ContractId),
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
    /// This is called by the customer.
    ///
    /// Special errors
    /// - [`Error::ContractOriginationFailed`]: The operation did not get confirmed on
    ///   chain at the correct depth.
    #[allow(unused)]
    pub async fn originate(
        merchant_funding_info: &MerchantFundingInformation,
        customer_funding_info: &CustomerFundingInformation,
        merchant_public_key: &PublicKey,
        customer_key_pair: &TezosKeyPair,
        channel_id: &ChannelId,
    ) -> Result<(ContractId, Level), Error> {
        todo!()
    }

    /// Call the `addFunding` entrypoint with the [`CustomerFundingInformation`].
    ///
    /// This will wait until the funding operation is confirmed at depth and is called by
    /// the customer.
    ///
    /// Special errors
    /// - [`Error::CustomerFundingFailed`]: The operation was invalid or did not get confirmed on
    ///   chain at the correct depth.
    #[allow(unused)]
    pub async fn add_customer_funding(
        contract_id: &ContractId,
        customer_funding_info: &CustomerFundingInformation,
        customer_key_pair: &TezosKeyPair,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Verify that the contract specified by [`ContractId`] has been correctly originated on
    /// chain with respect to the expected values.
    ///
    /// Correct origination requires that:
    /// - Contract encodes the expected zkChannels contract
    /// - Contract storage is correctly instantiated
    /// - Contract is confirmed on chain to the expected depth
    ///
    /// This function will wait until the origination operation is confirmed at depth
    /// and is called by the merchant.
    ///
    /// Special errors
    /// - [`Error::ContractOriginationInavalid`]: The contract is not a valid zkChannels contract
    ///   or it does not have the expected storage.
    /// - [`Error::ContractOriginationFailed`]: The operation did not get confirmed on
    ///   chain at the correct depth.
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

    /// Verify that the customer has sucessfully funded the contract via the `addFunding`
    /// entrypoint
    ///
    /// Correct funding requires that:
    /// - The `addFunding` operation is the latest operation to be applied to the contract
    /// - The `addFunding` operation is confirmed on chain to the expected depth
    ///
    /// This function will wait until the customer's funding operation is confirmed at depth
    /// and is called by the merchant.
    #[allow(unused)]
    pub async fn verify_customer_funding(
        contract_id: &ContractId,
        customer_funding_info: &CustomerFundingInformation,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Add merchant funding via the `addFunding` entrypoint to the given [`ContractId`],
    /// according to the [`MerchantFundingInformation`]
    ///
    /// This should only be called if [`verify_origination()`] and [`verify_customer_funding()`]
    /// both returned successfully.
    ///
    /// If the expected merchant funding is 0, this verifies that the contract status is OPEN.
    /// If the merchant funding is non-zero, this verifies that the contract status is
    /// AWAITING_FUNDING, then funds the contract.
    ///
    /// This function will wait until the merchant funding operation is confirmed at depth
    /// and is called by the merchant.
    ///
    /// Special errors
    /// - [`Error::MerchantFundingFailed`]: The operation was invalid or did not get confirmed on
    ///   chain at the correct depth.
    #[allow(unused)]
    pub async fn add_merchant_funding(
        contract_id: &ContractId,
        merchant_funding_info: &MerchantFundingInformation,
        merchant_key_pair: &TezosKeyPair,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Reclaim customer funding via the `reclaimFunding` entrypoint on the given [`ContractId`].
    ///
    /// This function will wait until the customer reclaim operation is confirmed at deth and is
    /// called by the customer.
    ///
    /// Special errors
    /// - [`Error::CustomerReclaimInvalid`]: The operation was not valid and was not accepted by
    ///   the chain (e.g. the channel status was not AWAITING_FUNDING)
    /// - [`Error::CustomerReclaimFailed`]: The operation was valid but did get confirmed on chain
    ///   at the expected depth.
    #[allow(unused)]
    pub async fn reclaim_customer_funding(
        contract_id: &ContractId,
        customer_key_pair: &TezosKeyPair,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Reclaim merchant funding via the `reclaimFunding` entrypoint on the given [`ContractId`].
    ///
    /// This function will wait until the merchant reclaim operation is confirmed at deth and is
    /// called by the merchant.
    ///
    /// Special errors
    /// - [`Error::MerchantReclaimInvalid`]: The operation was not valid and was not accepted by
    ///   the chain (e.g. the channel status was not AWAITING_FUNDING)
    /// - [`Error::MerchantReclaimFailed`]: The operation was valid but did get confirmed on chain
    ///   at the expected depth.
    ///   
    #[allow(unused)]
    pub async fn reclaim_merchant_funding(
        contract_id: &ContractId,
        merchant_key_pair: &TezosKeyPair,
    ) -> Result<(), Error> {
        todo!()
    }
}
