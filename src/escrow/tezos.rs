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
    ///
    /// TODO: Add additional errors if they arise (e.g. a wrapper around tezedge-client errors).
    #[derive(Clone, Debug, Error, Serialize, Deserialize)]
    pub enum Error {
        #[error("Encountered a network error while processing operation {0}")]
        NetworkFailure(Entrypoint),
        #[error("Operation {0} failed to confirm on chain for contract ID {1}")]
        OperationFailure(Entrypoint, ContractId),
        #[error("Unable to post operation {0} because it is invalid for contract ID {1}")]
        OperationInvalid(Entrypoint, ContractId),
        #[error("Originated contract with ID {0} is not a valid zkChannels contract or does not have expected storage")]
        InvalidZkChannelsContract(ContractId),
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
    /// The `originator_key_pair` should belong to whichever party originates the contract.
    /// Currently, this must be called by the customer.
    #[allow(unused)]
    pub async fn originate(
        merchant_funding_info: &MerchantFundingInformation,
        customer_funding_info: &CustomerFundingInformation,
        merchant_public_key: &PublicKey,
        originator_key_pair: &TezosKeyPair,
        channel_id: &ChannelId,
    ) -> Result<(ContractId, Level), Error> {
        todo!()
    }

    /// Call the `addFunding` entrypoint with the [`CustomerFundingInformation`].
    ///
    /// This will wait until the funding operation is confirmed at depth. It is called by
    /// the customer.
    ///
    /// The operation is invalid if:
    /// - the channel status is not AWAITING_FUNDING
    /// - the specified customer address does not match the `cust_addr` field in the contract
    /// - the specified funding information does not match the `custFunding` amount in the contract
    /// - the `addFunding` entrypoint has not been called by the customer address before.
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
    /// This function will return [`Error::InvalidZkChannelsContract`] if the contract is not a valid
    /// zkChannels contract or it does not have the expected storage.
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
    /// This function will wait until the merchant funding operation is confirmed at depth. It
    /// is called by the merchant.
    ///
    /// If the expected merchant funding is non-zero, this operation is invalid if:
    /// - the contract status is not AWAITING_FUNDING
    /// - the specified merchant address does not match the `merch_addr` field in the contract
    /// - the specified funding information does not match the `merchFunding` amount in the contract
    /// - the `addFunding` entrypoint has already been called by the merchant address
    ///
    /// If the expected merchant funding is 0, this operation is invalid if:
    /// - the contract status is not OPEN
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
    /// This function will wait until the customer reclaim operation is confirmed at depth. It is
    /// called by the customer.
    ///
    /// The operation is invalid if:
    /// - the contract status is not AWAITING_FUNDING.
    /// - the `addFunding` entrypoint has not been called by the customer address
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
    /// The operation is invalid if:
    /// - the contract status is not AWAITING_FUNDING.
    /// - the `addFunding` entrypoint has not been called by the merchant address
    #[allow(unused)]
    pub async fn reclaim_merchant_funding(
        contract_id: &ContractId,
        merchant_key_pair: &TezosKeyPair,
    ) -> Result<(), Error> {
        todo!()
    }
}
