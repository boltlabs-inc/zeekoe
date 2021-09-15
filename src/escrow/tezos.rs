use std::str::FromStr;

use crate::escrow::types::*;
use inline_python::python;
use tezedge::{OriginatedAddress, ToBase58Check};
use zkabacus_crypto::RevocationLock;

use self::close::FinalBalances;

/// The Michelson contract code for the ZkChannels contract.
static CONTRACT_CODE: &str = include_str!("zkchannel_contract.tz");

/// The default confirmation depth to consider chain operations to be final.
pub const DEFAULT_CONFIRMATION_DEPTH: u64 = 1; // FIXME: put this back to 20 after testing

/// The default `self_delay` parameter: 2 days, in seconds.
pub const DEFAULT_SELF_DELAY: u64 = 2 * 24 * 60 * 60;

/// Create a fresh python execution context to be used for a single python operation, then thrown
/// away. This ensures we don't carry over global state, and we can concurrently use python-based
/// functions without the Global Interpreter Lock.
fn python_context() -> inline_python::Context {
    python! {
        from pytezos import pytezos, Contract, ContractInterface
        import json

        main_code = ContractInterface.from_michelson('CONTRACT_CODE)

        // Originate a contract on chain
        def originate(
            uri,
            cust_addr, merch_addr,
            cust_acc,
            merch_pubkey,
            channel_id,
            merch_g2, merch_y2s, merch_x2,
            cust_funding, merch_funding,
            min_confirmations,
            self_delay
        ):
            // Customer pytezos interface
            cust_py = pytezos.using(key=cust_acc, shell=uri)

            initial_storage = {"cid": channel_id,
            "customer_address": cust_addr,
            "customer_balance": cust_funding,
            "delay_expiry": "1970-01-01T00:00:00Z",
            "merchant_address": merch_addr,
            "merchant_balance": merch_funding,
            "merchant_public_key": merch_pubkey,
            "g2": merch_g2,
            "y2s_0": merch_y2s[0],
            "y2s_1": merch_y2s[1],
            "y2s_2": merch_y2s[2],
            "y2s_3": merch_y2s[3],
            "y2s_4": merch_y2s[4],
            "x2": merch_x2,
            "revocation_lock": "0x00",
            "self_delay": self_delay,
            "status": 0}

            // Originate main zkchannel contract
            out = cust_py.origination(script=main_code.script(initial_storage=initial_storage)).autofill().sign().send(min_confirmations=min_confirmations)

            // Get address, status, and level of main zkchannel contract
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            contract_id = contents["metadata"]["operation_result"]["originated_contracts"][0]
            status = contents["metadata"]["operation_result"]["status"]
            block = op_info["branch"]
            level = 1 // TODO: get the level where the operation was confirmed

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
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            block = op_info["branch"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)

        // Call the `addMerchFunding` endpoint of an extant contract
        def add_merchant_funding(
            uri,
            merch_acc,
            contract_id,
            merch_funding,
            min_confirmations
        ):
            // Merchant pytezos interface
            merch_py = pytezos.using(key=merch_acc, shell=uri)

            // Merchant zkchannel contract interface
            merch_ci = merch_py.contract(contract_id)

            // Call the addMerchFunding entrypoint
            out = merch_ci.addMerchFunding().with_amount(merch_funding).send(min_confirmations=min_confirmations)

            // Get status and level of the addMerchFunding operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)

        def cust_close(
            uri,
            cust_acc,
            contract_id,
            customer_balance, merchant_balance,
            sigma1, sigma2,
            revocation_lock,
            min_confirmations,
        ):
            // Customer pytezos interface
            cust_py = pytezos.using(key=cust_acc, shell=uri)

            // Customer zkchannel contract interface
            cust_ci = cust_py.contract(contract_id)

            // Set the storage for the operation
            close_storage = {
                "customer_balance": int(customer_balance),
                "merchant_balance": int(merchant_balance),
                "revocation_lock": revocation_lock,
                "sigma1": sigma1,
                "sigma2": sigma2
            }

            // Call the custClose entrypoint
            out = cust_ci.custClose(close_storage).send(min_confirmations=min_confirmations)

            // Get status and level of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)

        def cust_claim(
            uri,
            cust_acc,
            contract_id,
            min_confirmations,
        ):
            // Customer pytezos interface
            cust_py = pytezos.using(key=cust_acc, shell=uri)

            // Customer zkchannel contract interface
            cust_ci = cust_py.contract(contract_id)

            // Call the custClaim entrypoint
            out = cust_ci.custClaim().send(min_confirmations=min_confirmations)

            // Get status and level of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)

        def reclaim_funding(
            uri,
            cust_acc,
            contract_id,
            min_confirmations,
        ):
            // Customer pytezos interface
            cust_py = pytezos.using(key=cust_acc, shell=uri)

            // Customer zkchannel contract interface
            cust_ci = cust_py.contract(contract_id)

            // Call the reclaimFunding entrypoint
            out = cust_ci.reclaimFunding().send(min_confirmations=min_confirmations)

            // Get status and level of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)

        def expiry(
            uri,
            merch_acc,
            contract_id,
            min_confirmations,
        ):
            // Merchant pytezos interface
            merch_py = pytezos.using(key=merch_acc, shell=uri)

            // Merchant zkchannel contract interface
            merch_ci = merch_py.contract(contract_id)

            // Call the expiry entrypoint
            out = merch_ci.expiry().send(min_confirmations=min_confirmations)

            // Get status and level of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)

        def merch_claim(
            uri,
            merch_acc,
            contract_id,
            min_confirmations,
        ):
            // Merchant pytezos interface
            merch_py = pytezos.using(key=merch_acc, shell=uri)

            // Merchant zkchannel contract interface
            merch_ci = merch_py.contract(contract_id)

            // Call the merchClaim entrypoint
            out = merch_ci.merchClaim().send(min_confirmations=min_confirmations)

            // Get status and level of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)

        def merch_dispute(
            uri,
            merch_acc,
            contract_id,
            revocation_secret,
            min_confirmations,
        ):
            // Merchant pytezos interface
            merch_py = pytezos.using(key=merch_acc, shell=uri)

            // Merchant zkchannel contract interface
            merch_ci = merch_py.contract(contract_id)

            // Call the merchDispute entrypoint
            out = merch_ci.merchDispute(revocation_secret).send(min_confirmations=min_confirmations)

            // Get status and level of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)

        def mutual_close(
            uri,
            cust_acc,
            contract_id,
            customer_balance, merchant_balance,
            mutual_close_signature,
            min_confirmations,
        ):
            // Customer pytezos interface
            cust_py = pytezos.using(key=cust_acc, shell=uri)

            // Customer zkchannel contract interface
            cust_ci = cust_py.contract(contract_id)

            // Set the storage for the operation
            mutual_close_storage = {
                "customer_balance": int(customer_balance),
                "merchant_balance": int(merchant_balance),
                "merchSig": mutual_close_signature
            }

            // Call the mutualClose entrypoint
            out = cust_ci.mutualClose(mutual_close_storage).send(min_confirmations=min_confirmations)

            // Get status and level of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            level = 1 // TODO: get the level where the operation was confirmed

            return (status, level)
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
pub async fn get_contract_state(_contract_id: &ContractId) -> Result<ContractState, Error> {
    todo!()
}

pub mod establish {
    use futures::Future;
    use tokio::task::JoinError;

    use super::*;
    use crate::escrow::notify::Level;
    use zkabacus_crypto::{ChannelId, CustomerBalance, MerchantBalance, PublicKey};

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
    #[derive(Debug, thiserror::Error)]
    #[error("Could not originate contract: {0}")]
    pub struct OriginateError(#[from] JoinError);

    /// An error while attempting to fund the contract.
    #[derive(Debug, thiserror::Error)]
    #[error("Could not fund contract: {0}")]
    pub struct CustomerFundError(#[from] JoinError);

    /// An error while attempting to fund the contract.
    #[derive(Debug, thiserror::Error)]
    #[error("Could not reclaim funding from contract: {0}")]
    pub struct ReclaimFundingError(#[from] JoinError);

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
    pub fn originate(
        uri: Option<&http::Uri>,
        merchant_funding_info: &MerchantFundingInformation,
        customer_funding_info: &CustomerFundingInformation,
        merchant_public_key: &PublicKey,
        originator_key_pair: &TezosKeyMaterial,
        channel_id: &ChannelId,
        confirmation_depth: u64,
        self_delay: u64,
    ) -> impl Future<Output = Result<(ContractId, OperationStatus, Level), OriginateError>>
           + Send
           + 'static {
        let (g2, y2s, x2) =
            super::pointcheval_sanders_public_key_to_python_input(merchant_public_key);
        let merchant_funding = merchant_funding_info.balance.into_inner();
        let merchant_address = merchant_funding_info.address.to_base58check();
        let merchant_pubkey = merchant_funding_info.public_key.to_base58check();

        let customer_account_key = originator_key_pair.private_key().to_base58check();
        let customer_funding = customer_funding_info.balance.into_inner();
        let customer_address = customer_funding_info.address.to_base58check();
        let channel_id = hex_string(channel_id.to_bytes().to_vec());
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = originate(
                        'uri,
                        'customer_address, 'merchant_address,
                        'customer_account_key,
                        'merchant_pubkey,
                        'channel_id,
                        'g2, 'y2s, 'x2,
                        'customer_funding, 'merchant_funding,
                        'confirmation_depth,
                        'self_delay
                    )
                });

                let (contract_id, status, level) = context.get::<(String, String, u32)>("out");
                let contract_id = ContractId::new(
                    OriginatedAddress::from_base58check(&contract_id)
                        .expect("Contract id returned from pytezos must be valid base58"),
                );
                (contract_id, status.parse().unwrap(), level.into())
            })
            .await
            .map_err(OriginateError)
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
    pub fn add_customer_funding(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        customer_funding_info: &CustomerFundingInformation,
        customer_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), CustomerFundError>> + Send + 'static
    {
        let customer_funding = customer_funding_info.balance.into_inner();
        let customer_private_key = customer_key_pair.private_key().to_base58check();
        let customer_pubkey = customer_funding_info.public_key.to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = add_customer_funding(
                        'uri,
                        'customer_private_key,
                        'contract_id,
                        'customer_funding,
                        'confirmation_depth
                    )
                });

                let (status, level) = context.get::<(String, u32)>("out");
                (status.parse().unwrap(), level.into())
            })
            .await
            .map_err(CustomerFundError)
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
    pub fn verify_origination(
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
    pub fn verify_customer_funding(
        _contract_id: &ContractId,
        _customer_funding_info: &CustomerFundingInformation,
    ) -> Result<(), Error> {
        todo!()
    }

    pub fn verify_merchant_funding(
        _contract_id: &ContractId,
        _customer_funding_info: &CustomerFundingInformation,
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
    pub fn add_merchant_funding(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        merchant_funding_info: &MerchantFundingInformation,
        merchant_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), CustomerFundError>> + Send + 'static
    {
        let merchant_funding = merchant_funding_info.balance.into_inner();
        let merchant_private_key = merchant_key_pair.private_key().to_base58check();
        let merchant_pubkey = merchant_funding_info.public_key.to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = add_merchant_funding(
                        'uri,
                        'merchant_private_key,
                        'contract_id,
                        'merchant_funding,
                        'confirmation_depth
                    )
                });

                let (status, level) = context.get::<(String, u32)>("out");
                (status.parse().unwrap(), level.into())
            })
            .await
            .map_err(CustomerFundError)
        }
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
    pub fn reclaim_customer_funding(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        customer_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), ReclaimFundingError>> + Send + 'static
    {
        let customer_private_key = customer_key_pair.private_key().to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = reclaim_funding(
                        'uri,
                        'customer_private_key,
                        'contract_id,
                        'confirmation_depth
                    )
                });

                let (status, level) = context.get::<(String, u32)>("out");
                (status.parse().unwrap(), level.into())
            })
            .await
            .map_err(ReclaimFundingError)
        }
    }
}

pub mod close {
    use super::python_context;
    use crate::escrow::{
        notify::Level,
        tezos::{hex_string, OperationStatus},
        types::*,
    };
    use futures::Future;
    use inline_python::python;
    use tokio::task::JoinError;
    use zkabacus_crypto::ChannelId;
    use {
        serde::{Deserialize, Serialize},
        tezedge::signer::OperationSignatureInfo,
        tezedge::ToBase58Check,
        zkabacus_crypto::{
            customer::ClosingMessage, revlock::RevocationSecret, CloseState, CustomerBalance,
            MerchantBalance,
        },
    };

    #[derive(Debug, thiserror::Error)]
    #[error("Could not issue expiry: {0}")]
    pub struct ExpiryError(#[from] JoinError);

    #[derive(Debug, thiserror::Error)]
    #[error("Could not issue merchant claim: {0}")]
    pub struct MerchantClaimError(#[from] JoinError);

    #[derive(Debug, thiserror::Error)]
    #[error("Could not issue customer close: {0}")]
    pub struct CustomerCloseError(#[from] JoinError);

    #[derive(Debug, thiserror::Error)]
    #[error("Could not issue merchant dispute: {0}")]
    pub struct MerchantDisputeError(#[from] JoinError);

    #[derive(Debug, thiserror::Error)]
    #[error("Could not issue customer claim: {0}")]
    pub struct CustomerClaimError(#[from] JoinError);
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

    impl From<OperationSignatureInfo> for MutualCloseAuthorizationSignature {
        fn from(info: OperationSignatureInfo) -> Self {
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
    pub fn expiry(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        merchant_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), ExpiryError>> + Send + 'static {
        let merchant_private_key = merchant_key_pair.private_key().to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = expiry('uri, 'merchant_private_key, 'contract_id, 'confirmation_depth)
                });

                let (status, level) = context.get::<(String, u32)>("out");
                (status.parse().unwrap(), level.into())
            })
            .await
            .map_err(ExpiryError)
        }
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
    pub fn merch_claim(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        merchant_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), MerchantClaimError>> + Send + 'static
    {
        let merchant_private_key = merchant_key_pair.private_key().to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = merch_claim(
                        'uri,
                        'merchant_private_key,
                        'contract_id,
                        'confirmation_depth
                    )
                });

                let (status, level) = context.get::<(String, u32)>("out");
                (status.parse().unwrap(), level.into())
            })
            .await
            .map_err(MerchantClaimError)
        }
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
    pub fn cust_close(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        close_message: &ClosingMessage,
        customer_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), CustomerCloseError>> + Send + 'static
    {
        let customer_balance = close_message.customer_balance().into_inner();
        let merchant_balance = close_message.merchant_balance().into_inner();
        let revocation_lock = hex_string(close_message.revocation_lock().as_bytes().to_vec());
        let customer_private_key = customer_key_pair.private_key().to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let (sigma1, sigma2) = close_message.closing_signature().clone().as_bytes();
        let sigma1 = sigma1.to_vec();
        let sigma2 = sigma2.to_vec();
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = cust_close(
                        'uri,
                        'customer_private_key,
                        'contract_id,
                        'customer_balance,
                        'merchant_balance,
                        'sigma1, 'sigma2,
                        'revocation_lock,
                        'confirmation_depth
                    )
                });

                let (status, level) = context.get::<(String, u32)>("out");
                (status.parse().unwrap(), level.into())
            })
            .await
            .map_err(CustomerCloseError)
        }
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
    pub fn merch_dispute(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        revocation_secret: &RevocationSecret,
        merchant_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), MerchantDisputeError>> + Send + 'static
    {
        let merchant_private_key = merchant_key_pair.private_key().to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let revocation_secret = hex_string(revocation_secret.as_bytes().to_vec());
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = merch_dispute(
                        'uri,
                        'merchant_private_key,
                        'contract_id,
                        'revocation_secret,
                        'confirmation_depth
                    )
                });

                let (status, level) = context.get::<(String, u32)>("out");
                (status.parse().unwrap(), level.into())
            })
            .await
            .map_err(MerchantDisputeError)
        }
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
    pub fn cust_claim(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        customer_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), CustomerClaimError>> + Send + 'static
    {
        let customer_private_key = customer_key_pair.private_key().to_base58check();
        let contract_id = contract_id.clone().to_originated_address().to_base58check();
        let uri = uri.map(|uri| uri.to_string());

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = cust_claim(
                        'uri,
                        'customer_private_key,
                        'contract_id,
                        'confirmation_depth
                    )
                });

                let (status, level) = context.get::<(String, u32)>("out");
                (status.parse().unwrap(), level.into())
            })
            .await
            .map_err(CustomerClaimError)
        }
    }

    /// Authorize the close state provided in the mutual close flow by producing a valid EdDSA
    /// signature over the tuple
    /// `(contract id, "zkChannels mutual close", channel id, customer balance, merchant balance)`
    ///
    /// This is called by the merchant.
    #[allow(unused)]
    pub fn authorize_mutual_close(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        close_state: &CloseState,
        merchant_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> Result<OperationSignatureInfo, Error> {
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
    #[allow(clippy::too_many_arguments)]
    pub fn mutual_close(
        uri: Option<&http::Uri>,
        contract_id: &ContractId,
        channel_id: &ChannelId,
        customer_balance: &CustomerBalance,
        merchant_balance: &MerchantBalance,
        authorization_signature: &MutualCloseAuthorizationSignature,
        merchant_key_pair: &TezosKeyMaterial,
        confirmation_depth: u64,
    ) -> impl Future<Output = Result<(OperationStatus, Level), Error>> + Send + 'static {
        async move { todo!() }
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
