pub mod notify;
pub mod tezos;

pub mod types {

    use core::fmt;
    use serde::{Deserialize, Serialize};
    use std::fmt::{Display, Formatter};

    use tezedge::OriginatedAddress;

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
}
