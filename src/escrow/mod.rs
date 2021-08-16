pub mod notify;
pub mod tezos;

pub mod types {

    use tezedge::OriginatedAddress;
    use {
        serde::{Deserialize, Serialize},
        std::fmt::{self, Display, Formatter},
        thiserror::Error,
    };

    /// Rename this type to match zkChannels written notation.
    /// Also, so we can easily change the tezedge type in case it is wrong.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ContractId(OriginatedAddress);
    //pub type ContractId = OriginatedAddress;
    zkabacus_crypto::impl_sqlx_for_bincode_ty!(ContractId);

    impl Display for ContractId {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            // TODO: Fill in with actual contract ID
            std::fmt::Debug::fmt(self, f)
        }
    }

    impl ContractId {
        pub fn new(addr: OriginatedAddress) -> Self {
            Self(addr)
        }
    }

    pub type TezosPublicKey = tezedge::PublicKey;
    pub type TezosFundingAccount = tezedge::ImplicitAddress;
    pub struct TezosKeyPair {
        public_key: TezosPublicKey,
        secret_key: tezedge::PrivateKey,
    }

    impl TezosKeyPair {
        /// Form a new `TezosKeyPair` from its constituent parts.
        pub fn new(public_key: TezosPublicKey, secret_key: tezedge::PrivateKey) -> Self {
            // TODO: add some validation that these form a valid keypair?
            Self {
                public_key,
                secret_key,
            }
        }

        /// Get the public key.
        pub fn public_key(&self) -> &TezosPublicKey {
            &self.public_key
        }

        /// Get the secret key.
        pub fn secret_key(&self) -> &tezedge::PrivateKey {
            &self.secret_key
        }
    }

    /// The set of entrypoints on the zkChannels Tezos smart contract.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub enum Entrypoint {
        Originate,
        AddMerchantFunding,
        AddCustomerFunding,
        ReclaimMerchantFunding,
        ReclaimCustomerFunding,
        Expiry,
        CustomerClose,
        MerchantDispute,
        CustomerClaim,
        MerchantClaim,
        MutualClose,
    }

    impl Display for Entrypoint {
        fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
            f.write_str(match self {
                Entrypoint::Originate => "originate",
                Entrypoint::AddMerchantFunding => "addFunding for merchant",
                Entrypoint::AddCustomerFunding => "addFunding for customer",
                Entrypoint::ReclaimMerchantFunding => "reclaimFunding for merchant",
                Entrypoint::ReclaimCustomerFunding => "reclaimFunding for customer",
                Entrypoint::Expiry => "expiry",
                Entrypoint::CustomerClose => "custClose",
                Entrypoint::MerchantDispute => "merchDispute",
                Entrypoint::CustomerClaim => "custClaim",
                Entrypoint::MerchantClaim => "merchClaim",
                Entrypoint::MutualClose => "mutualClose",
            })
        }
    }

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
        #[error("Failed to produce an authorization signature for mutual close operation for contract ID {0}")]
        SigningFailed(ContractId),
    }
}
