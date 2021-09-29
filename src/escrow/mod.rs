pub mod notify;
pub mod tezos;

pub mod types {

    use std::{borrow::Cow, convert::TryFrom, path::PathBuf};

    use super::notify::Level;
    use tezedge::{
        crypto::base58check::ToBase58Check, OriginatedAddress, PrivateKey as TezosPrivateKey,
    };
    use zkabacus_crypto::PublicKey as ZkAbacusPublicKey;
    use {
        serde::{Deserialize, Serialize},
        sha3::{Digest, Sha3_256},
        std::{
            fmt::{self, Display, Formatter},
            path::Path,
        },
        thiserror::Error,
    };

    /// ID for a zkChannels contract originated on Tezos.
    /// Equivalent to the Tezos OriginatedAddress type.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ContractId(OriginatedAddress);
    zkabacus_crypto::impl_sqlx_for_bincode_ty!(ContractId);

    impl ContractId {
        pub fn to_originated_address(self) -> OriginatedAddress {
            self.0
        }
    }

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

    /// Tezos public key.
    pub type TezosPublicKey = tezedge::PublicKey;

    /// Tezos implicit address; the address of a Tezos account that can fund a zkChannels contract.
    /// An address is the hash of a [`TezosPublicKey`].
    pub type TezosFundingAddress = tezedge::ImplicitAddress;

    /// Set of methods to specify a key in the config file, specified in order of preference.
    ///
    /// Rearranging these is a breaking change due to the untagged serialization.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum KeySpecifier {
        Path(PathBuf),
        Alias { alias: String },
    }

    impl KeySpecifier {
        /// If the `KeySpecifier` is a `Path`, updates it to be relative to the given directory.
        pub fn set_relative_path(&mut self, config_dir: &Path) {
            if let KeySpecifier::Path(path) = self {
                *path = config_dir.join(&path)
            }
        }

        /// Convert the `KeySpecifier` into a type that can be passed to `inline_python`.
        /// The output will be a valid input for the `key` parameter of
        /// [`PyTezosClient.using()`](https://pytezos.org/high_level.html?highlight=using#pytezos.client.PyTezosClient.using)
        pub fn into_python_type(&self) -> Cow<str> {
            match self {
                KeySpecifier::Path(path) => path.to_string_lossy(),
                KeySpecifier::Alias { alias } => Cow::from(alias.clone()),
            }
        }
    }

    /// Tezos key material, with public key and contents of key file.
    #[derive(Clone)]
    pub struct TezosKeyMaterial {
        public_key: TezosPublicKey,
        private_key: TezosPrivateKey,
    }

    impl TezosKeyMaterial {
        /// Extract a `TezosKeyPair` from a file.
        ///
        /// The file should use the key file json formatting that is also used by faucet:
        /// https://faucet.tzalpha.net/
        pub fn read_key_pair(key_specifier: &KeySpecifier) -> Result<TezosKeyMaterial, Error> {
            let path = key_specifier.into_python_type();

            // Use pytezos parsing functions to parse account config file.
            let key_context: inline_python::Context = inline_python::python!(
                from pytezos import pytezos;
                client = pytezos.using(key='path)
                public_key = str(client.key.public_key())
                private_key = str(client.key.secret_key())
            );

            // Retrieve key strings from python context
            let public_key_string = key_context.get::<String>("public_key");
            let private_key_string: String = key_context.get::<String>("private_key");

            // Parse strings using tezedge-client methods
            Ok(Self {
                public_key: TezosPublicKey::from_base58check(&public_key_string)
                    .map_err(|_| Error::KeyFileInvalid("Couldn't parse public key".to_string()))?,
                private_key: TezosPrivateKey::from_base58check(&private_key_string)
                    .map_err(|_| Error::KeyFileInvalid("Couldn't parse private key".to_string()))?,
            })
        }

        /// Transform into just the public key.
        pub fn into_keypair(self) -> (TezosPublicKey, TezosPrivateKey) {
            (self.public_key, self.private_key)
        }

        /// Get the public key.
        pub fn public_key(&self) -> &TezosPublicKey {
            &self.public_key
        }

        /// Get the private key.
        pub fn private_key(&self) -> &TezosPrivateKey {
            &self.private_key
        }

        /// Get the funding address.
        pub fn funding_address(&self) -> TezosFundingAddress {
            self.public_key().hash()
        }
    }

    /// Details about the on-chain location and merchant party of a zkChannels contract.
    pub struct ContractDetails {
        /// Public key for the merchant party.
        pub merchant_tezos_public_key: TezosPublicKey,
        /// ID of Tezos contract originated on chain.
        pub contract_id: Option<ContractId>,
        /// Level at which Tezos contract is originated.
        pub contract_level: Option<Level>,
    }

    impl ContractDetails {
        pub fn merchant_funding_address(&self) -> TezosFundingAddress {
            self.merchant_tezos_public_key.hash()
        }
    }

    /// A SHA3-256 hash of the contract's Micheline JSON encoding.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    pub struct ContractHash([u8; 32]);

    impl ContractHash {
        pub fn new(micheline: &str) -> Self {
            let contract_bytes = micheline.as_bytes();
            let mut hasher = Sha3_256::new();

            hasher.update(&contract_bytes);

            let mut digested = [0; 32];
            digested.copy_from_slice(hasher.finalize().as_ref());
            Self(digested)
        }
    }

    /// A SHA3-256 hash of the merchant's public keys.
    #[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
    pub struct KeyHash([u8; 32]);

    impl KeyHash {
        /// Compute the SHA3-256 hash of the merchant's Pointcheval-Sanders [`ZkAbacusPublicKey`],
        /// their [`TezosPublicKey`], and the [`TezosFundingAddress`] associated with that public
        /// key.
        ///
        /// Note: the funding address is hashed from its checked base58 representation, rather than
        /// the raw bytes.
        pub fn new(
            zkabacus_public_key: &ZkAbacusPublicKey,
            funding_address: TezosFundingAddress,
            tezos_public_key: &TezosPublicKey,
        ) -> Self {
            let mut hasher = Sha3_256::new();

            hasher.update(zkabacus_public_key.to_bytes());
            hasher.update(funding_address.to_base58check());
            hasher.update(tezos_public_key);

            let mut digested = [0; 32];
            digested.copy_from_slice(hasher.finalize().as_ref());
            Self(digested)
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

    /// The set of statuses that a zkChannels contract can enter.
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum ContractStatus {
        AwaitingCustomerFunding = 0,
        AwaitingMerchantFunding = 1,
        Open = 2,
        Expiry = 3,
        CustomerClose = 4,
        Closed = 5,
        FundingReclaimed = 6,
    }

    #[derive(Error, Debug)]
    #[error("Failed to convert {0} to a ContractStatus")]
    pub struct ParseContractStatusError(i32);

    impl TryFrom<i32> for ContractStatus {
        type Error = ParseContractStatusError;

        fn try_from(v: i32) -> Result<Self, Self::Error> {
            match v {
                x if x == ContractStatus::AwaitingCustomerFunding as i32 => {
                    Ok(ContractStatus::AwaitingCustomerFunding)
                }
                x if x == ContractStatus::AwaitingMerchantFunding as i32 => {
                    Ok(ContractStatus::AwaitingMerchantFunding)
                }
                x if x == ContractStatus::Open as i32 => Ok(ContractStatus::Open),
                x if x == ContractStatus::Expiry as i32 => Ok(ContractStatus::Expiry),
                x if x == ContractStatus::CustomerClose as i32 => Ok(ContractStatus::CustomerClose),
                x if x == ContractStatus::Closed as i32 => Ok(ContractStatus::Closed),
                x if x == ContractStatus::FundingReclaimed as i32 => {
                    Ok(ContractStatus::FundingReclaimed)
                }
                x => Err(ParseContractStatusError(x)),
            }
        }
    }

    /// Set of errors that may arise while establishing a zkChannel.
    ///
    /// Note: Errors noting that an operation has failed to be confirmed on chain only arise when
    /// a specified timeout period has passed. In general, the functions in this module will wait
    /// until operations are successfully confirmed.
    ///
    /// TODO: Add additional errors if they arise (e.g. a wrapper around tezedge-client errors).
    #[derive(Debug, Error, Serialize, Deserialize)]
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
        #[error("Mutual close authorization signature is invalid for contract ID {0}")]
        InvalidAuthorizationSignature(ContractId),
        #[error("Key file was invalid: {0}")]
        KeyFileInvalid(String),
    }

    #[cfg(test)]
    mod test {
        use super::*;

        #[test]
        fn decode_python_string() {
            let public_key_string = "edpku5Ei6Dni4qwoJGqXJs13xHfyu4fhUg6zqZkFyiEh1mQhFD3iZE";
            let secret_key_string = "edsk2pfUZ7NAbo7ekr5RHW6Dni2GYKS935mqXXcrbXtTn8dCfTfViZ";

            TezosPublicKey::from_base58check(public_key_string).unwrap();
            tezedge::PrivateKey::from_base58check(secret_key_string).unwrap();
        }
    }
}
