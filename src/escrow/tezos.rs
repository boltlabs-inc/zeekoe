use std::convert::TryFrom;
use std::str::FromStr;
use std::time::{Duration, SystemTime};

use crate::escrow::types::*;
use futures::Future;
use inline_python::python;
use tezedge::{OriginatedAddress, ToBase58Check};
use tokio::task::JoinError;
use zkabacus_crypto::{CustomerBalance, MerchantBalance, RevocationLock};

/// The Michelson contract code for the ZkChannels contract.
static CONTRACT_CODE: &str = include_str!("zkchannel_contract.tz");

/// The default confirmation depth to consider chain operations to be final.
pub const DEFAULT_CONFIRMATION_DEPTH: u64 = 1; // FIXME: put this back to 20 after testing

/// The default `self_delay` parameter: 2 days, in seconds.
pub const DEFAULT_SELF_DELAY: u64 = 2 * 24 * 60 * 60;

lazy_static::lazy_static! {
    /// The ZkChannels close scalar as bytes
    static ref CLOSE_SCALAR_HEX_STRING: String =
        hex_string(zkabacus_crypto::CLOSE_SCALAR.to_bytes().to_vec());

    /// The python execution context used for all pytezos operations.
    static ref PYTHON: inline_python::Context = {
        let close_scalar = &*CLOSE_SCALAR_HEX_STRING;

        python! {
            from pytezos import pytezos, Contract, ContractInterface
            import json

            main_code = ContractInterface.from_michelson('CONTRACT_CODE)

            close_scalar_bytes = 'close_scalar

            // Originate a contract on chain
            def originate(
                uri,
                cust_addr, merch_addr,
                cust_acc,
                cust_pubkey, merch_pubkey,
                channel_id,
                merch_g2, merch_y2s, merch_x2,
                cust_funding, merch_funding,
                min_confirmations,
                self_delay
            ):
                // Customer pytezos interface
                cust_py = pytezos.using(key=cust_acc, shell=uri)

                initial_storage = {"cid": channel_id,
                "close_scalar": close_scalar_bytes,
                "context_string": "zkChannels mutual close",
                "customer_address": cust_addr,
                "customer_balance": cust_funding,
                "customer_public_key": cust_pubkey,
                "delay_expiry": "1970-01-01T00:00:00Z",
                "g2": merch_g2,
                "merchant_address": merch_addr,
                "merchant_balance": merch_funding,
                "merchant_public_key": merch_pubkey,
                "y2s_0": merch_y2s[0],
                "y2s_1": merch_y2s[1],
                "y2s_2": merch_y2s[2],
                "y2s_3": merch_y2s[3],
                "y2s_4": merch_y2s[4],
                "x2": merch_x2,
                "revocation_lock": "0x00",
                "self_delay": self_delay,
                "status": 0}

                print(initial_storage) // FIXME: delete this

                // Originate main zkchannel contract
                out = cust_py.origination(script=main_code.script(initial_storage=initial_storage)).autofill().sign().send(min_confirmations=min_confirmations)

                // Get address, status, and level of main zkchannel contract
                op_info = pytezos.using(shell=uri).shell.blocks[-20:].find_operation(out.hash())
                contents = op_info["contents"][0]
                contract_id = contents["metadata"]["operation_result"]["originated_contracts"][0]
                status = contents["metadata"]["operation_result"]["status"]
                block = op_info["branch"]
                level = pytezos.using(shell=uri).shell.blocks[block].level()

                return (contract_id, status, level)

            // Call the `addCustFunding` entrypoint of an extant contract
            def add_customer_funding(
                uri,
                cust_acc,
                contract_id,
                cust_funding,
                min_confirmations
            ):
                // Customer pytezos interface
                cust_py = pytezos.using(key=cust_acc, shell=uri)

                // Customer zkchannel contract interface
                cust_ci = cust_py.contract(contract_id)

                // Call the addCustFunding entrypoint
                out = cust_ci.addCustFunding().with_amount(cust_funding).send(min_confirmations=min_confirmations)

                // Get status and level of the addCustFunding operation
                op_info = pytezos.using(shell=uri).shell.blocks[-20:].find_operation(out.hash())
                contents = op_info["contents"][0]
                status = contents["metadata"]["operation_result"]["status"]
                block = op_info["branch"]
                level = pytezos.using(shell=uri).shell.blocks[block].level()

                return (status, level)

            def contract_state(
                uri,
                cust_acc,
                contract_id
            ):
                cust_py = pytezos.using(key=cust_acc, shell=uri)
                cust_ci = cust_py.contract(contract_id)

                return (
                    cust_ci.storage["status"](),
                    cust_ci.storage["delay_expiry"](),
                    cust_ci.storage["revocation_lock"](),
                    cust_ci.storage["customer_balance"](),
                    cust_ci.storage["merchant_balance"]()
                )
        }
    };
}

#[derive(Debug, Clone, Copy)]
pub struct FinalBalances {
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
}

impl FinalBalances {
    pub fn merchant_balance(&self) -> MerchantBalance {
        self.merchant_balance
    }

    pub fn customer_balance(&self) -> CustomerBalance {
        self.customer_balance
    }
}

/// Convert a byte vector into a string like "0xABC123".
fn hex_string(bytes: Vec<u8>) -> String {
    format!("0x{}", hex::encode(bytes))
}

/// Convert a Pointcheval-Sanders public key to its three components in string-encoded form suitable
/// for pytezos.
fn pointcheval_sanders_public_key_to_python_input(
    public_key: &zkabacus_crypto::PublicKey,
) -> (String, Vec<String>, String) {
    let zkabacus_crypto::PublicKey { g2, y2s, x2, .. } = public_key;
    let g2 = hex_string(g2.to_uncompressed().to_vec());
    let y2s = y2s
        .iter()
        .map(|y2| hex_string(y2.to_uncompressed().to_vec()))
        .collect::<Vec<_>>();
    let x2 = hex_string(x2.to_uncompressed().to_vec());

    (g2, y2s, x2)
}

/// State of a zkChannels contract at a point in time.
#[derive(Debug)]
pub struct ContractState {
    /// Current contract status.
    status: ContractStatus,
    /// Indicator to whether the timeout on the contract has expired, if it was set.
    timeout_expired: Option<bool>,
    /// Revocation lock from the contract, if it was set.
    revocation_lock: Option<RevocationLock>,
    /// Final balances from the contract, if they have been determined.
    final_balances: Option<FinalBalances>,
}

impl ContractState {
    /// Get the current status of the contract.
    pub fn status(&self) -> ContractStatus {
        self.status
    }

    /// Get the indicator to whether the timeout was set and, if so, whether it has expired.
    pub fn timeout_expired(&self) -> Option<bool> {
        self.timeout_expired
    }

    // Get the revocation lock from the contract, if it has been set.
    pub fn revocation_lock(&self) -> Option<&RevocationLock> {
        self.revocation_lock.as_ref()
    }

    /// Get the final balances on the contract if they are determined.
    pub fn final_balances(&self) -> Option<&FinalBalances> {
        self.final_balances.as_ref()
    }
}

/// The result of attempting an operation.
pub enum OperationStatus {
    /// The operation successfully was applied and included in the head block.
    Applied,
    /// The operation failed to be applied at all.
    Failed,
    /// The operation was backtracked.
    Backtracked,
    /// The operation was skipped.
    Skipped,
}

#[derive(Debug, thiserror::Error)]
#[error("Could not parse `OperationStatus` {0}")]
pub struct OperationStatusParseError(String);

#[derive(Debug, thiserror::Error)]
#[error("Could not get contract status: {0}")]
pub struct GetContractStateError(#[from] JoinError);

impl FromStr for OperationStatus {
    type Err = OperationStatusParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use OperationStatus::*;
        Ok(match s {
            "applied" => Applied,
            "failed" => Failed,
            "backtracked" => Backtracked,
            "skipped" => Skipped,
            s => return Err(OperationStatusParseError(s.to_string())),
        })
    }
}

/// Query the chain to retrieve the confirmed state of the contract with the given [`ContractId`].
///
/// This function should query the state of the contract at the given confirmation depth -- that
/// is, the state of the the contract, but not accounting for the latest
/// `DEFAULT_CONFIRMATION_DEPTH` blocks.
pub fn get_contract_state(
    uri: Option<&http::Uri>,
    originator_key_pair: &TezosKeyMaterial,
    contract_id: &ContractId,
) -> impl Future<Output = Result<ContractState, GetContractStateError>> + Send + 'static {
    let uri = uri.map(|uri| uri.to_string());
    let customer_account_key = originator_key_pair.private_key().to_base58check();
    let contract_id = contract_id.clone().to_originated_address().to_base58check();

    async move {
        tokio::task::spawn_blocking(move || {
            PYTHON.run(python! {
                        out = contract_state(
                            'uri,
                            'customer_account_key,
                            'contract_id
                        )
            });

            let (status, delay_expiry, revocation_lock_bytes, customer_amount, merchant_amount) =
                PYTHON.get::<(i32, u32, Vec<u8>, u64, u64)>("out");

            let status = ContractStatus::try_from(status).unwrap();

            let timeout_expired = if delay_expiry == 0 {
                None
            } else {
                let delay_expiry =
                    SystemTime::UNIX_EPOCH + Duration::from_secs(delay_expiry.into());
                Some(delay_expiry < SystemTime::now())
            };

            let (revocation_lock, final_balances) = if status == ContractStatus::CustomerClose {
                let revocation_lock_bytes = <[u8; 32]>::try_from(revocation_lock_bytes).unwrap();
                let revocation_lock = RevocationLock::from_bytes(&revocation_lock_bytes).unwrap();

                let final_balances = FinalBalances {
                    merchant_balance: MerchantBalance::try_new(merchant_amount).unwrap(),
                    customer_balance: CustomerBalance::try_new(customer_amount).unwrap(),
                };

                (Some(revocation_lock), Some(final_balances))
            } else {
                (None, None)
            };

            ContractState {
                status,
                timeout_expired,
                revocation_lock,
                final_balances,
            }
        })
        .await
        .map_err(GetContractStateError)
    }
}

pub mod establish {
    use super::*;
    use crate::escrow::notify::Level;
    use zkabacus_crypto::{ChannelId, PublicKey};

    #[allow(unused)]
    pub struct CustomerFundingInformation {
        /// Initial balance for the customer in the channel.
        pub balance: CustomerBalance,

        /// Funding source which will support the balance. This address is the hash of
        /// the `public_key`.
        pub address: TezosFundingAddress,

        /// Public key associated with the funding address. The customer must have access to the
        /// corresponding [`tezedge::PrivateKey`].
        pub public_key: TezosPublicKey,
    }

    #[allow(unused)]
    pub struct MerchantFundingInformation {
        /// Initial balance for the merchant in the channel.
        pub balance: MerchantBalance,

        /// Funding source which will support the balance. This address is the hash of
        /// the `public_key`.
        pub address: TezosFundingAddress,

        /// Public key associated with the funding address. The merchant must have access to the
        /// corresponding [`tezedge::PrivateKey`].
        pub public_key: TezosPublicKey,
    }

    /// An error while attempting to originate the contract.
    #[derive(Debug, Clone, thiserror::Error)]
    #[error("Could not originate contract: {0}")]
    pub struct OriginateError(String);

    /// An error while attempting to fund the contract.
    #[derive(Debug, Clone, thiserror::Error)]
    #[error("Could not fund contract: {0}")]
    pub struct CustomerFundError(String);

    /// Originate a contract on chain.
    ///
    /// This call will wait until the contract is confirmed at depth. It returns the new
    /// [`ContractId`] and the [`Level`] of the block that contains the originated contract.
    ///
    /// The `originator_key_pair` should belong to whichever party originates the contract.
    /// Currently, this must be called by the customer. Its public key must be the same as the one
    /// in the provided [`CustomerFundingInformation`].
    ///
    /// By default, this uses the Tezos mainnet; however, another URI may be specified to point to a
    /// sandbox or testnet node.
    #[allow(clippy::too_many_arguments)]
    pub async fn originate(
        uri: Option<&http::Uri>,
        merchant_funding_info: &MerchantFundingInformation,
        customer_funding_info: &CustomerFundingInformation,
        merchant_public_key: &PublicKey,
        originator_key_pair: &TezosKeyMaterial,
        channel_id: &ChannelId,
        confirmation_depth: u64,
        self_delay: u64,
    ) -> Result<(ContractId, OperationStatus, Level), OriginateError> {
        let (g2, y2s, x2) =
            super::pointcheval_sanders_public_key_to_python_input(merchant_public_key);
        let merchant_funding = merchant_funding_info.balance.into_inner();
        let merchant_address = merchant_funding_info.address.to_base58check();
        let merchant_pubkey = merchant_funding_info.public_key.to_base58check();

        let customer_account_key = originator_key_pair.private_key().to_base58check();
        let customer_funding = customer_funding_info.balance.into_inner();
        let customer_address = customer_funding_info.address.to_base58check();
        let customer_pubkey = customer_funding_info.public_key.to_base58check();
        let channel_id = hex_string(channel_id.to_bytes().to_vec());
        let uri = uri.map(|uri| uri.to_string());

        PYTHON.run(python! {
            success = True
            try:
                out = originate(
                    'uri,
                    'customer_address, 'merchant_address,
                    'customer_account_key,
                    'customer_pubkey, 'merchant_pubkey,
                    'channel_id,
                    'g2, 'y2s, 'x2,
                    'customer_funding, 'merchant_funding,
                    'confirmation_depth,
                    'self_delay
                )
            except Exception as e:
                success = False
                error = repr(e)
        });

        if PYTHON.get("success") {
            let (contract_id, status, level) = PYTHON.get::<(String, String, u32)>("out");
            let contract_id = ContractId::new(
                OriginatedAddress::from_base58check(&contract_id)
                    .expect("Contract id returned from pytezos must be valid base58"),
            );
            Ok((contract_id, status.parse().unwrap(), level.into()))
        } else {
            let error = PYTHON.get::<String>("error");
            Err(OriginateError(error))
        }
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
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        customer_funding_info: &CustomerFundingInformation,
        customer_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> Result<(OperationStatus, Level), CustomerFundError> {
        let customer_funding = customer_funding_info.balance.into_inner();
        let customer_private_key = customer_key_pair.private_key().to_base58check();
        let customer_pubkey = customer_funding_info.public_key.to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let contract_id = &contract_id;
        let uri = uri.map(|uri| uri.to_string());

        PYTHON.run(python! {
            success = True
            try:
                out = add_customer_funding(
                    'uri,
                    'customer_private_key,
                    'contract_id,
                    'customer_funding,
                    'confirmation_depth
                )
            except Exception as e:
                success = False
                error = repr(e)
        });

        if PYTHON.get("success") {
            let (status, level) = PYTHON.get::<(String, u32)>("out");
            let contract_id = ContractId::new(
                OriginatedAddress::from_base58check(contract_id)
                    .expect("Contract id returned from pytezos must be valid base58"),
            );
            Ok((status.parse().unwrap(), level.into()))
        } else {
            let error = PYTHON.get::<String>("error");
            Err(CustomerFundError(error))
        }
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

    /// Verify that the customer has successfully funded the contract via the `addFunding`
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
        merchant_key_pair: &TezosKeyMaterial,
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
        customer_key_pair: &TezosKeyMaterial,
    ) -> Result<(), Error> {
        todo!()
    }
}

pub mod close {
    use zkabacus_crypto::ChannelId;

    use super::FinalBalances;
    use crate::escrow::types::*;

    use {
        serde::{Deserialize, Serialize},
        tezedge::signer::OperationSignatureInfo,
        zkabacus_crypto::{
            customer::ClosingMessage, revlock::RevocationSecret, CloseState, CustomerBalance,
            MerchantBalance,
        },
    };

    /// Merchant authorization signature for a mutual close operation.
    ///
    /// The internals of this type are a dupe for the tezedge [`OperationSigantureInfo`] type.
    /// We're not reusing that type because it isn't serializable, and because we may want to
    /// change the internal storage here.
    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct MutualCloseAuthorizationSignature {
        /// base58check with prefix (`Prefix::operation`) encoded operation hash.
        operation_hash: String,
        /// forged operation (hex) concatenated with signature('hex').
        operation_with_signature: String,
        /// operation signature encoded with base58check with prefix (`Prefix::edsig`).
        signature: String,
    }

    impl MutualCloseAuthorizationSignature {
        pub fn from_operation_signature_info(info: OperationSignatureInfo) -> Self {
            let OperationSignatureInfo {
                operation_hash,
                operation_with_signature,
                signature,
            } = info;
            Self {
                operation_hash,
                operation_with_signature,
                signature,
            }
        }

        /// Get the operation hash.
        pub fn operation_hash(&self) -> &String {
            &self.operation_hash
        }

        /// Get the forged operation hash concatenated with the signature.
        pub fn operation_with_signature(&self) -> &String {
            &self.operation_with_signature
        }

        /// Get the signature by itself.
        pub fn signature(&self) -> &String {
            &self.signature
        }
    }

    /// Initiate expiry close flow via the `expiry` entrypoint on the given [`ContractId`].
    ///
    /// This function will wait until the expiry operation is confirmed at depth and is called
    /// by the merchant.
    ///
    /// This operation is invalid if:
    /// - the contract status is not OPEN
    /// - the [`TezosFundingAddress`] specified does not match the `merch_addr` field in the
    ///   the specified contract
    #[allow(unused)]
    pub async fn expiry(
        contract_id: &ContractId,
        merchant_key_pair: &TezosKeyMaterial,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Complete expiry close flow by claiming the entire channel balance on the [`ContractId`]
    /// via the `merchClaim` entrypoint.
    ///
    /// This function will wait until the self-delay period on the `expiry` entrypoint has passed.
    /// After posting the `merchClaim` operation, it will wait until it has been confirmed at
    /// depth. It is called by the merchant.
    ///
    /// This operation is invalid if:
    /// - the contract status is not EXPIRY
    /// - the [`TezosKeyPair`] does not match the `merch_addr` field in the specified
    ///   contract
    #[allow(unused)]
    pub async fn merch_claim(
        contract_id: &ContractId,
        merchant_key_pair: &TezosKeyMaterial,
    ) -> Result<FinalBalances, Error> {
        todo!()
    }

    /// Initiate unilateral customer close flow or correct balances from the expiry flow by
    /// posting the correct channel balances for the [`ContractId`] via the `custClose` entrypoint.
    ///
    /// This function will wait until it is confirmed at depth. It is called by the customer. If
    /// it is called in response to an `expiry` operation, it will be called by the customer's
    /// notification service.
    ///
    /// This operation is invalid if:
    /// - the contract status is neither OPEN nor EXPIRY
    /// - the [`TezosKeyPair`] does not match the `cust_addr` field in the specified contract
    /// - the signature in the [`ClosingMessage`] is not a well-formed signature
    /// - the signature in the [`ClosingMessage`] is not a valid signature under the merchant
    ///   public key on the expected tuple
    #[allow(unused)]
    pub async fn cust_close(
        contract_id: &ContractId,
        close_message: &ClosingMessage,
        customer_key_pair: &TezosKeyMaterial,
    ) -> Result<(), Error> {
        // This function should:
        // - Generate customer authorization EdDSA signature on the operation with the customer's
        //   Tezos public key.
        // - Send custClose entrypoint calling operation to blockchain. This operation results in a
        //   timelock on the customer's balance and an immediate payout of the merchant balance
        todo!()
    }

    /// Dispute balances posted by a customer (via [`cust_close()`]) by posting a revocation
    /// secret that matches the posted revocation lock. On successful completion, this call
    /// will transfer the posted customer balance to the merchant.
    ///
    /// This function will wait until it is confirmed at depth. It is called by the merchant.
    ///
    /// This operation is invalid if:
    /// - the contract status is not CUST_CLOSE
    /// - the [`TezosKeyPair`] does not match the `merch_addr` field in the specified contract
    /// - the [`RevocationSecret`] does not hash to the `rev_lock` field in the specified contract
    #[allow(unused)]
    pub async fn merch_dispute(
        contract_id: &ContractId,
        revocation_secret: &RevocationSecret,
        merchant_key_pair: &TezosKeyMaterial,
    ) -> Result<(FinalBalances), Error> {
        todo!()
    }

    /// Claim customer funds (posted via [`cust_close()`]) after the timeout period has elapsed
    /// via the `custClaim` entrypoint.
    ///
    /// This function will wait until the timeout period from the `custClose` entrypoint call has
    /// elapsed, and until the `custClaim` operation is confirmed at depth. It is called by the
    /// customer.
    ///
    /// This operation is invalid if:
    /// - the contract status is not CUST_CLOSE
    /// - the [`TezosKeyPair`] does not match the `cust_addr` field in the specified contract
    #[allow(unused)]
    pub async fn cust_claim(
        contract_id: &ContractId,
        customer_key_pair: &TezosKeyMaterial,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Authorize the close state provided in the mutual close flow by producing a valid EdDSA
    /// signature over the tuple
    /// `(contract id, "zkChannels mutual close", channel id, customer balance, merchant balance)`
    ///
    /// This is called by the merchant.
    #[allow(unused)]
    pub async fn authorize_mutual_close(
        contract_id: &ContractId,
        close_state: &CloseState,
        merchant_key_pair: &TezosKeyMaterial,
    ) -> Result<MutualCloseAuthorizationSignature, Error> {
        todo!()
    }

    /// Execute the mutual close flow via the `mutualClose` entrypoint paying out the specified
    /// channel balances to both parties.
    ///
    /// This function will wait until the operation is confirmed at depth. It is called by the
    /// customer.
    ///
    /// This operation is invalid if:
    /// - the contract status is not OPEN
    /// - the [`TezosKeyPair`] does not match the `cust_addr` field in the specified contract
    /// - the `authorization_signature` is not a valid signature under the merchant public key
    ///   on the expected tuple
    #[allow(unused)]
    pub async fn mutual_close(
        contract_id: &ContractId,
        channel_id: &ChannelId,
        customer_balance: &CustomerBalance,
        merchant_balance: &MerchantBalance,
        authorization_signature: MutualCloseAuthorizationSignature,
        customer_key_pair: &TezosKeyMaterial,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Verify that the specified contract is closed.
    ///
    /// This function will wait until the contract status is CLOSED at the expected confirmation
    /// depth and is called by the merchant.
    #[allow(unused)]
    pub async fn verify_contract_closed(contract_id: &ContractId) -> Result<(), Error> {
        todo!()
    }
}
