use {
    crate::escrow::types::*,
    canonicalize_json_micheline::{canonicalize_json_micheline, CanonicalizeError},
    futures::Future,
    inline_python::{pyo3, pyo3::conversion::FromPyObject, python},
    serde::{Deserialize, Serialize},
    std::{
        convert::{TryFrom, TryInto},
        str::FromStr,
        time::{Duration, SystemTime},
    },
    tezedge::{OriginatedAddress, ToBase58Check},
    tokio::task::JoinError,
    zkabacus_crypto::{
        customer::ClosingMessage, revlock::RevocationSecret, ChannelId, CloseState,
        CustomerBalance, MerchantBalance, PublicKey, RevocationLock,
    },
};

/// The Micheline JSON for the ZkChannels contract.
static CONTRACT_CODE: &str = include_str!("zkchannels_contract_canonical.json");

lazy_static::lazy_static! {
    static ref CONTRACT_CODE_HASH: ContractHash = ContractHash::new(&*CONTRACT_CODE);
}

/// The default `revocation_lock`: a hex-encoded string which pytezos reads as a scalar 0.
const DEFAULT_REVOCATION_LOCK: &str = "0x00";

/// Create a fresh python execution context to be used for a single python operation, then thrown
/// away. This ensures we don't carry over global state, and we can concurrently use python-based
/// functions without the Global Interpreter Lock.
fn python_context() -> inline_python::Context {
    let context = python! {
        import json
        from pytezos import pytezos, Contract, ContractInterface
        from pytezos.michelson.types import MichelsonType
        from pytezos.michelson.parse import michelson_to_micheline

        main_code = ContractInterface.from_micheline(json.loads('CONTRACT_CODE))

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
            "revocation_lock": 'DEFAULT_REVOCATION_LOCK,
            "self_delay": self_delay,
            "status": 0}

            // Originate main zkchannel contract
            out = cust_py.origination(script=main_code.script(initial_storage=initial_storage)).autofill().sign().send(min_confirmations=min_confirmations)

            // Get address, status of main zkchannel contract
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            contract_id = contents["metadata"]["operation_result"]["originated_contracts"][0]
            status = contents["metadata"]["operation_result"]["status"]
            block = op_info["branch"]

            return (contract_id, status)

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

            // Get status of the addCustFunding operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]
            block = op_info["branch"]

            return status

        // Get the state of a contract.
        def contract_state(
            uri,
            cust_acc,
            contract_id,
            min_confirmations
        ):
            cust_py = pytezos.using(key=cust_acc, shell=uri)
            cust_ci = cust_py.contract(contract_id)

            if min_confirmations > 1:
                block_id = "head~{}".format(min_confirmations-1)
                contract = cust_ci.using(block_id = block_id)
            else:
                contract = cust_ci

            storage = contract.storage()
            storage["revocation_lock"] = storage["revocation_lock"].to_bytes(32, byteorder="little")

            contract_code = json.dumps(contract.to_micheline(), sort_keys = True)
            return (storage, contract_code)

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

            // Get status of the addMerchFunding operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]

            return status

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

            // Get status of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]

            return status

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

            // Get status of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]

            return status

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

            // Get status of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]

            return status

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

            // Get status of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]

            return status

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

            // Get status of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]

            return status

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

            // Get status of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]

            return status

        def sign_mutual_close(
            uri,
            merch_acc,
            channel_id,
            contract_id,
            customer_balance, merchant_balance
        ):
            // Merchant pytezos interface
            merch_py = pytezos.using(key=merch_acc, shell=uri)
            // Specify the structure and types of the fields going into the mutual close state.
            ty = MichelsonType.match(michelson_to_micheline("pair (pair bls12_381_fr string) (pair address (pair mutez mutez))"))
            // create the packed (serialized) version of the mutual close state, corresponding to the types above.
            // legacy=True ensures pytezos will always serialize the data as in michelson rather than micheline.
            packed = ty.from_python_object((channel_id, "zkChannels mutual close", contract_id, customer_balance, merchant_balance)).pack(legacy=True).hex()
            mutual_close_signature = merch_py.key.sign(packed)

            return mutual_close_signature

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

            // Get status of the operation
            search_depth = 2 * min_confirmations
            op_info = pytezos.using(shell=uri).shell.blocks[-search_depth:].find_operation(out.hash())
            contents = op_info["contents"][0]
            status = contents["metadata"]["operation_result"]["status"]

            return status
    };
    context
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

fn vec_equals<T: PartialEq>(a: &[T], b: &[T]) -> bool {
    let matching = a.iter().zip(b.iter()).filter(|&(a, b)| a == b).count();
    matching == a.len() && matching == b.len()
}

/// Convert a byte vector into a string like "0xABC123".
fn hex_string(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

/// Convert a Pointcheval-Sanders public key to its three components in string-encoded form suitable
/// for pytezos.
fn pointcheval_sanders_public_key_to_python_input(
    public_key: &zkabacus_crypto::PublicKey,
) -> (String, Vec<String>, String) {
    let zkabacus_crypto::PublicKey { g2, y2s, x2, .. } = public_key;
    let g2 = hex_string(&g2.to_uncompressed());
    let y2s = y2s
        .iter()
        .map(|y2| hex_string(&y2.to_uncompressed()))
        .collect::<Vec<_>>();
    let x2 = hex_string(&x2.to_uncompressed());

    (g2, y2s, x2)
}

#[derive(Debug, thiserror::Error)]
pub enum VerificationError {
    #[error("Expected contract to be {expected:?}, but was {actual:?}")]
    UnexpectedContractStatus {
        expected: ContractStatus,
        actual: ContractStatus,
    },
    #[error(transparent)]
    ContractState(#[from] ContractStateError),
    #[error("Contract hash did not match expected hash")]
    UnexpectedContractHash,
    #[error("Expected customer contract's self_delay to be {expected:?}, but was {actual:?}")]
    UnexpectedSelfDelay { expected: u64, actual: u64 },
    #[error("Expected customer contract's delay_expiry to be to 0, but was {actual:?}")]
    UnexpectedDelayExpiry { actual: u32 },
    #[error("Expected contract's merchant_balance to be {expected:?}, but was {actual:?}")]
    UnexpectedMerchantBalance {
        expected: MerchantBalance,
        actual: MerchantBalance,
    },
    #[error("Expected contract's customer_balance to be {expected:?}, but was {actual:?}")]
    UnexpectedCustomerBalance {
        expected: CustomerBalance,
        actual: CustomerBalance,
    },
    #[error("Expected customer contract's revocation_lock_bytes to be 0, but was {actual:?}")]
    UnexpectedRevocationLock { actual: Vec<u8> },
    #[error(transparent)]
    ZkAbacus(#[from] zkabacus_crypto::Error),
    #[error("Contract's MerchantPublicKey did not match the merchant's public key")]
    UnexpectedMerchantKey,
}

#[derive(Debug, thiserror::Error)]
pub enum ContractStateError {
    #[error(transparent)]
    PythonError(#[from] JoinError),
    #[error(transparent)]
    ParseContractStatus(#[from] ParseContractStatusError),
    #[error(transparent)]
    ZkAbacusError(#[from] zkabacus_crypto::Error),
    #[error("Could not parse bytes as `RevocationLock`: {0:?}")]
    ParseRevocationLock(Vec<u8>),
    #[error("Error canonicalizing contract: {0:?}")]
    Canonicalize(#[from] CanonicalizeError),
}

/// State of a zkChannels contract at a point in time.
#[derive(Debug)]
pub struct ContractState {
    merchant_address_base58: String,
    merchant_tezos_public_key_base58: String,
    customer_amount: u64,
    merchant_amount: u64,
    status: i32,
    revocation_lock_bytes: Vec<u8>,
    self_delay: u64,
    delay_expiry: u32,
    merchant_public_key: (Vec<u8>, [Vec<u8>; 5], Vec<u8>),
    contract_code: String,
}

impl ContractState {
    /// Get the current status of the contract.
    pub fn status(&self) -> Result<ContractStatus, ContractStateError> {
        Ok(ContractStatus::try_from(self.status)?)
    }

    /// Get the indicator to whether the timeout was set and, if so, whether it has expired.
    pub fn timeout_expired(&self) -> Option<bool> {
        match self.delay_expiry {
            0 => None,
            n => {
                let timeout_expired =
                    SystemTime::UNIX_EPOCH + Duration::from_secs(n.into()) < SystemTime::now();
                Some(timeout_expired)
            }
        }
    }

    pub fn customer_balance(&self) -> Result<CustomerBalance, zkabacus_crypto::Error> {
        CustomerBalance::try_new(self.customer_amount)
    }

    pub fn merchant_balance(&self) -> Result<MerchantBalance, zkabacus_crypto::Error> {
        MerchantBalance::try_new(self.merchant_amount)
    }

    /// Get the final balances on the contract if they are determined.
    pub fn final_balances(&self) -> Result<Option<FinalBalances>, ContractStateError> {
        Ok(match self.status()? {
            ContractStatus::CustomerClose => Some(FinalBalances {
                merchant_balance: self.merchant_balance()?,
                customer_balance: self.customer_balance()?,
            }),
            _ => None,
        })
    }

    // Get the revocation lock from the contract, if it has been set.
    pub fn revocation_lock(&self) -> Result<Option<RevocationLock>, ContractStateError> {
        Ok(match self.status()? {
            ContractStatus::CustomerClose => {
                let revocation_lock_bytes: &[u8; 32] =
                    &self.revocation_lock_bytes.clone().try_into().map_err(|_| {
                        ContractStateError::ParseRevocationLock(self.revocation_lock_bytes.clone())
                    })?;
                let revocation_lock = RevocationLock::from_bytes(revocation_lock_bytes)
                    .ok_or_else(|| {
                        ContractStateError::ParseRevocationLock(self.revocation_lock_bytes.clone())
                    })?;
                Some(revocation_lock)
            }
            _ => None,
        })
    }

    /// The merchant's Pointcheval Sanders public key: (g2, y2s, x2)
    pub fn merchant_public_key(&self) -> &(Vec<u8>, [Vec<u8>; 5], Vec<u8>) {
        &self.merchant_public_key
    }

    /// A SHA3-256 hash of the contract's Micheline JSON encoding.
    pub fn has_correct_hash(&self) -> Result<bool, ContractStateError> {
        let canonicalized_contract_code = canonicalize_json_micheline(&self.contract_code)?;
        Ok(ContractHash::new(&canonicalized_contract_code) == *CONTRACT_CODE_HASH)
    }

    pub fn self_delay(&self) -> u64 {
        self.self_delay
    }
}

impl<'source> FromPyObject<'source> for ContractState {
    // This expects a tuple of the shape:
    //
    // (storage, micheline_json)
    //
    // Where storage is a hash of the storage of a contract, and `micheline_json` is the serialized
    // Micheline JSON representation of the contract.
    fn extract(obj: &'source pyo3::PyAny) -> pyo3::PyResult<Self> {
        let storage = obj.get_item(0)?;
        let contract_code = obj.get_item(1)?.extract()?;

        Ok(ContractState {
            merchant_address_base58: storage.get_item("merchant_address")?.extract()?,
            merchant_tezos_public_key_base58: storage.get_item("merchant_public_key")?.extract()?,
            customer_amount: storage.get_item("customer_balance")?.extract()?,
            merchant_amount: storage.get_item("merchant_balance")?.extract()?,
            status: storage.get_item("status")?.extract()?,
            revocation_lock_bytes: storage.get_item("revocation_lock")?.extract()?,
            self_delay: storage.get_item("self_delay")?.extract()?,
            delay_expiry: storage.get_item("delay_expiry")?.extract()?,
            merchant_public_key: (
                storage.get_item("g2")?.extract()?,
                [
                    storage.get_item("y2s_0")?.extract()?,
                    storage.get_item("y2s_1")?.extract()?,
                    storage.get_item("y2s_2")?.extract()?,
                    storage.get_item("y2s_3")?.extract()?,
                    storage.get_item("y2s_4")?.extract()?,
                ],
                storage.get_item("x2")?.extract()?,
            ),
            contract_code,
        })
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
#[error("Could not issue mutual close: {0}")]
pub struct MutualCloseError(#[from] JoinError);

#[derive(Debug, thiserror::Error)]
#[error("Could not issue authorization signature for mutual close: {0}")]
pub struct AuthorizeMutualCloseError(#[from] JoinError);

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
    /// operation signature encoded with base58check with prefix (`Prefix::edsig`).
    signature: String,
}

impl MutualCloseAuthorizationSignature {
    /// Get the signature by itself.
    pub fn signature(&self) -> &String {
        &self.signature
    }
}

/// Originate a contract on chain.
///
/// This call will wait until the contract is confirmed at depth. It returns the new
/// [`ContractId`].
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
) -> impl Future<Output = Result<(ContractId, OperationStatus), OriginateError>> + Send + 'static {
    let (g2, y2s, x2) = pointcheval_sanders_public_key_to_python_input(merchant_public_key);
    let merchant_funding = merchant_funding_info.balance.into_inner();
    let merchant_address = merchant_funding_info.address.to_base58check();
    let merchant_pubkey = merchant_funding_info.public_key.to_base58check();

    let customer_account_key = originator_key_pair.private_key().to_base58check();
    let customer_funding = customer_funding_info.balance.into_inner();
    let customer_address = customer_funding_info.address.to_base58check();
    let channel_id = hex_string(&channel_id.to_bytes());
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

            let (contract_id, status) = context.get::<(String, String)>("out");
            let contract_id = ContractId::new(
                OriginatedAddress::from_base58check(&contract_id)
                    .expect("Contract id returned from pytezos must be valid base58"),
            );
            (contract_id, status.parse().unwrap())
        })
        .await
        .map_err(OriginateError)
    }
}

/// Information used by a Tezos node to post an operation on chain.
pub struct TezosClient {
    /// Link to the Tezos network.
    pub uri: Option<http::Uri>,
    /// ID of the contract for which the client will post an operation.
    pub contract_id: ContractId,
    /// Key material for the client.
    pub client_key_pair: TezosKeyMaterial,
    /// Block depth for which the client will wait for their operation to reach.
    pub confirmation_depth: u64,
    /// Mutually-agreed delay period for which a client must wait before claiming funds.
    pub self_delay: u64,
}

impl TezosClient {
    /// Transform the Tezos client fields into the correct Python representations, for use in
    /// inline-python calls to the PyTezos API.
    ///
    /// Returns tuple of `(URI, private key, contract_id)`
    fn as_python_types(&self) -> (Option<String>, String, String) {
        let private_key = self.client_key_pair.private_key().to_base58check();
        let contract_id = self
            .contract_id
            .clone()
            .to_originated_address()
            .to_base58check();
        let uri = self.uri.as_ref().map(|uri| uri.to_string());

        (uri, private_key, contract_id)
    }

    /// Query the chain to retrieve the confirmed state of the contract with the given [`ContractId`].
    ///
    /// This function should query the state of the contract at the confirmation depth described in
    /// the `TezosClient`, which may not be the default or "fully confirmed" depth.
    pub fn get_contract_state(
        &self,
    ) -> impl Future<Output = Result<ContractState, ContractStateError>> + Send + 'static {
        let (uri, client_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = contract_state(
                        'uri,
                        'client_private_key,
                        'contract_id,
                        'confirmation_depth
                    )
                });

                context.get::<ContractState>("out")
            })
            .await
            .map_err(ContractStateError::PythonError)
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
    pub fn add_customer_funding(
        &self,
        customer_funding_info: &CustomerFundingInformation,
    ) -> impl Future<Output = Result<OperationStatus, CustomerFundError>> + Send + 'static {
        let customer_funding = customer_funding_info.balance.into_inner();
        let (uri, customer_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

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

                let status = context.get::<String>("out");
                status.parse().unwrap()
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
    /// This function will return [`VerificationError`] if the contract is not a valid
    /// zkChannels contract or it does not have the expected storage.
    pub async fn verify_origination(
        &self,
        expected_merchant_balance: MerchantBalance,
        expected_customer_balance: CustomerBalance,
        merchant_public_key: &PublicKey,
    ) -> Result<(), VerificationError> {
        let contract_state = self.get_contract_state().await?;

        if contract_state.delay_expiry != 0 {
            return Err(VerificationError::UnexpectedDelayExpiry {
                actual: contract_state.delay_expiry,
            });
        }

        if !is_zero(&contract_state.revocation_lock_bytes) {
            return Err(VerificationError::UnexpectedRevocationLock {
                actual: contract_state.revocation_lock_bytes,
            });
        }

        match contract_state.status()? {
            ContractStatus::AwaitingCustomerFunding => {}
            actual => {
                return Err(VerificationError::UnexpectedContractStatus {
                    expected: ContractStatus::AwaitingCustomerFunding,
                    actual,
                })
            }
        };

        if contract_state.self_delay() != self.self_delay {
            return Err(VerificationError::UnexpectedSelfDelay {
                expected: self.self_delay,
                actual: contract_state.self_delay(),
            });
        }

        let merchant_balance = contract_state.merchant_balance()?;
        let customer_balance = contract_state.customer_balance()?;

        if merchant_balance.into_inner() != expected_merchant_balance.into_inner() {
            return Err(VerificationError::UnexpectedMerchantBalance {
                expected: expected_merchant_balance,
                actual: merchant_balance,
            });
        }

        if customer_balance.into_inner() != expected_customer_balance.into_inner() {
            return Err(VerificationError::UnexpectedCustomerBalance {
                expected: expected_customer_balance,
                actual: customer_balance,
            });
        }

        let (expected_g2, expected_y2s, expected_x2) =
            pointcheval_sanders_public_key_to_python_input(merchant_public_key);
        let (g2, y2s, x2) = contract_state.merchant_public_key();
        let (g2, y2s, x2) = (
            hex_string(g2),
            y2s.iter().map(|y2| hex_string(y2)).collect::<Vec<String>>(),
            hex_string(x2),
        );

        if g2 != expected_g2 || !vec_equals(&y2s, &expected_y2s) || x2 != expected_x2 {
            return Err(VerificationError::UnexpectedMerchantKey);
        }

        if !contract_state.has_correct_hash()? {
            return Err(VerificationError::UnexpectedContractHash);
        }

        Ok(())
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
    pub async fn verify_customer_funding(
        &self,
        merchant_balance: &MerchantBalance,
    ) -> Result<(), VerificationError> {
        let expected = if merchant_balance.into_inner() > 0 {
            ContractStatus::AwaitingMerchantFunding
        } else {
            ContractStatus::Open
        };

        let contract_state = self.get_contract_state().await?;
        let actual = contract_state.status()?;

        if expected == actual {
            Ok(())
        } else {
            Err(VerificationError::UnexpectedContractStatus { expected, actual })
        }
    }

    /// Verify that the contract storage status has been open for ``required_confirmations`.
    pub async fn verify_merchant_funding(&self) -> Result<(), VerificationError> {
        let contract_state = self.get_contract_state().await?;

        match contract_state.status()? {
            ContractStatus::Open => Ok(()),
            actual => Err(VerificationError::UnexpectedContractStatus {
                expected: ContractStatus::Open,
                actual,
            }),
        }
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
    pub fn add_merchant_funding(
        &self,
        merchant_funding_info: &MerchantFundingInformation,
    ) -> impl Future<Output = Result<OperationStatus, CustomerFundError>> + Send + 'static {
        let merchant_funding = merchant_funding_info.balance.into_inner();
        let (uri, merchant_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

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

                let status = context.get::<String>("out");
                status.parse().unwrap()
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
        &self,
    ) -> impl Future<Output = Result<OperationStatus, ReclaimFundingError>> + Send + 'static {
        let (uri, customer_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

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

                let status = context.get::<String>("out");
                status.parse().unwrap()
            })
            .await
            .map_err(ReclaimFundingError)
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
    pub fn expiry(
        &self,
    ) -> impl Future<Output = Result<OperationStatus, ExpiryError>> + Send + 'static {
        let (uri, merchant_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = expiry('uri, 'merchant_private_key, 'contract_id, 'confirmation_depth)
                });

                let status = context.get::<String>("out");
                status.parse().unwrap()
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
    pub fn merch_claim(
        &self,
    ) -> impl Future<Output = Result<OperationStatus, MerchantClaimError>> + Send + 'static {
        let (uri, merchant_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

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

                let status = context.get::<String>("out");
                status.parse().unwrap()
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
    pub fn cust_close(
        &self,
        close_message: &ClosingMessage,
    ) -> impl Future<Output = Result<OperationStatus, CustomerCloseError>> + Send + 'static {
        let (uri, customer_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

        let customer_balance = close_message.customer_balance().into_inner();
        let merchant_balance = close_message.merchant_balance().into_inner();
        let revocation_lock = hex_string(&close_message.revocation_lock().as_bytes());
        let (sigma1, sigma2) = close_message.closing_signature().clone().as_bytes();
        let sigma1 = hex_string(&sigma1);
        let sigma2 = hex_string(&sigma2);

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

                let status = context.get::<String>("out");
                status.parse().unwrap()
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
    pub fn merch_dispute(
        &self,
        revocation_secret: &RevocationSecret,
    ) -> impl Future<Output = Result<OperationStatus, MerchantDisputeError>> + Send + 'static {
        let (uri, merchant_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

        let revocation_secret = hex_string(&revocation_secret.as_bytes());

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

                let status = context.get::<String>("out");
                status.parse().unwrap()
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
    pub fn cust_claim(
        &self,
    ) -> impl Future<Output = Result<OperationStatus, CustomerClaimError>> + Send + 'static {
        let (uri, customer_private_key, contract_id) = self.as_python_types();
        let confirmation_depth = self.confirmation_depth;

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

                let status = context.get::<String>("out");
                status.parse().unwrap()
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
        &self,
        close_state: &CloseState,
    ) -> impl Future<Output = Result<MutualCloseAuthorizationSignature, AuthorizeMutualCloseError>>
           + Send
           + 'static {
        let (uri, merchant_private_key, contract_id) = self.as_python_types();
        let channel_id = close_state.channel_id();
        let channel_id = hex_string(&channel_id.to_bytes());
        let customer_balance = close_state.customer_balance().into_inner();
        let merchant_balance = close_state.merchant_balance().into_inner();

        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = sign_mutual_close(
                        'uri,
                        'merchant_private_key,
                        'channel_id,
                        'contract_id,
                        'customer_balance,
                        'merchant_balance
                    )
                });

                MutualCloseAuthorizationSignature {
                    signature: context.get::<String>("out"),
                }
            })
            .await
            .map_err(AuthorizeMutualCloseError)
        }
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
    pub fn mutual_close(
        &self,
        channel_id: &ChannelId,
        customer_balance: &CustomerBalance,
        merchant_balance: &MerchantBalance,
        authorization_signature: &MutualCloseAuthorizationSignature,
    ) -> impl Future<Output = Result<OperationStatus, MutualCloseError>> + Send + 'static {
        let (uri, customer_private_key, contract_id) = self.as_python_types();
        let customer_balance = customer_balance.into_inner();
        let merchant_balance = merchant_balance.into_inner();
        let confirmation_depth = self.confirmation_depth;
        let mutual_close_signature = authorization_signature.signature.clone();
        async move {
            tokio::task::spawn_blocking(move || {
                let context = python_context();
                context.run(python! {
                    out = mutual_close(
                        'uri,
                        'customer_private_key,
                        'contract_id,
                        'customer_balance,
                        'merchant_balance,
                        'mutual_close_signature,
                        'confirmation_depth
                    )
                });

                let status = context.get::<String>("out");
                status.parse().unwrap()
            })
            .await
            .map_err(MutualCloseError)
        }
    }

    /// Verify that the specified contract is closed.
    ///
    /// This function will wait until the contract status is CLOSED at the expected confirmation
    /// depth and is called by the merchant.
    #[allow(unused)]
    pub async fn verify_contract_closed(&self, contract_id: &ContractId) -> Result<(), Error> {
        todo!()
    }
}

fn is_zero(buf: &[u8]) -> bool {
    let (prefix, aligned, suffix) = unsafe { buf.align_to::<u128>() };

    prefix.iter().all(|&x| x == 0)
        && suffix.iter().all(|&x| x == 0)
        && aligned.iter().all(|&x| x == 0)
}
