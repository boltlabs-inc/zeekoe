use crate::escrow::{notify::Level, types::*};
use inline_python::python;
use tezedge::{OriginatedAddress, ToBase58Check};
use zkabacus_crypto::{ChannelId, CustomerBalance, MerchantBalance, PublicKey};

/// The Michelson contract code for the ZkChannels contract.
static CONTRACT_CODE: &str = include_str!("zkchannel_contract.tz");

/// The default confirmation depth to consider chain operations to be final.
pub const DEFAULT_CONFIRMATION_DEPTH: u64 = 20;

lazy_static::lazy_static! {
    /// The ZkChannels close scalar as bytes
    static ref CLOSE_SCALAR_BYTES: [u8; 32] = zkabacus_crypto::CLOSE_SCALAR.to_bytes();

    /// The python execution context used for all pytezos operations.
    static ref PYTHON: inline_python::Context = {
        let close_scalar = CLOSE_SCALAR_BYTES.to_vec();

        python! {
            from pytezos import pytezos, Contract, ContractInterface
            import json

            main_code = ContractInterface.from_michelson('CONTRACT_CODE)

            close_scalar_bytes = 'close_scalar

            def originate(
                uri,
                cust_acc,
                cust_pubkey, merch_pubkey,
                channel_id,
                merch_g2, merch_y2s, merch_x2,
                cust_funding, merch_funding,
                min_confirmations
            ):
                // Customer pytezos interface
                cust_py = pytezos.using(key=cust_acc, shell=uri)

                initial_storage = {
                    "cid": channel_id,
                    "close_flag": close_scalar_bytes,
                    "context_string": "zkChannels mutual close",
                    "custAddr": cust_addr,
                    "custBal": 0,
                    "custFunding": cust_funding,
                    "custPk": cust_pubkey,
                    "delayExpiry": "1970-01-01T00:00:00Z",
                    "g2": merch_g2,
                    "merchAddr": merch_addr,
                    "merchBal": 0,
                    "merchFunding": merch_funding,
                    "merchPk": merch_pubkey,
                    "merchPk0": merch_y2s[0],
                    "merchPk1": merch_y2s[1],
                    "merchPk2": merch_y2s[2],
                    "merchPk3": merch_y2s[3],
                    "merchPk4": merch_y2s[4],
                    "merchPk5": merch_x2,
                    "revLock": "0x00",
                    "selfDelay": 3,
                    "status": 0
                }

                // Originate main zkchannel contract
                out = cust_py.origination(script=main_code.script(initial_storage=initial_storage)).autofill().sign().send(min_confirmations=min_confirmations)

                // Get address of main zkchannel contract
                opg = pytezos.shell.blocks[-20:].find_operation(out.hash())
                contents = opg["contents"][0]
                level = contents["level"]
                main_id = contents["metadata"]["operation_result"]["originated_contracts"][0]

                return (main_id, level)
        }
    };
}

fn merchant_public_key_to_python_input(
    public_key: &zkabacus_crypto::PublicKey,
) -> (Vec<u8>, Vec<Vec<u8>>, Vec<u8>) {
    let zkabacus_crypto::PublicKey { g2, y2s, x2, .. } = public_key;
    let g2 = g2.to_compressed().to_vec();
    let y2s = y2s
        .iter()
        .map(|y2| y2.to_compressed().to_vec())
        .collect::<Vec<_>>();
    let x2 = x2.to_compressed().to_vec();

    (g2, y2s, x2)
}

pub mod establish {
    use super::*;

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
    pub async fn originate(
        uri: Option<&http::Uri>,
        merchant_funding_info: &MerchantFundingInformation,
        customer_funding_info: &CustomerFundingInformation,
        merchant_public_key: &PublicKey,
        originator_key_pair: &TezosKeyMaterial,
        channel_id: &ChannelId,
        confirmation_depth: u64,
    ) -> Result<(ContractId, Level), OriginateError> {
        let (g2, y2s, x2) = super::merchant_public_key_to_python_input(merchant_public_key);
        let merchant_funding = merchant_funding_info.balance.into_inner();
        let merchant_pubkey = merchant_funding_info.public_key.to_base58check();

        let customer_account_details = originator_key_pair.file_contents();
        let customer_funding = customer_funding_info.balance.into_inner();
        let customer_pubkey = customer_funding_info.public_key.to_base58check();
        let channel_id = channel_id.to_bytes().to_vec();
        let uri = uri.map(|uri| uri.to_string());

        PYTHON.run(python! {
            success = true
            try:
                out = originate(
                    'uri,
                    'customer_account_details, 'channel_id,
                    'customer_pubkey, 'merchant_pubkey,
                    'g2, 'y2s, 'x2,
                    'customer_funding, 'merchant_funding,
                    'confirmation_depth
                )
            except Exception as e:
                success = false
                error = str(e)
        });

        if PYTHON.get("success") {
            let (contract_id, level) = PYTHON.get::<(String, u32)>("out");
            let contract_id = ContractId::new(
                OriginatedAddress::from_base58check(&contract_id)
                    .expect("Contract id returned from pytezos must be valid base58"),
            );
            Ok((contract_id, level.into()))
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
        contract_id: &ContractId,
        customer_funding_info: &CustomerFundingInformation,
        customer_key_pair: &TezosKeyMaterial,
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

mod close {
    use crate::escrow::types::*;

    use {
        tezedge::signer::OperationSignatureInfo,
        zkabacus_crypto::{
            customer::ClosingMessage, revlock::RevocationSecret, CloseState, CustomerBalance,
            MerchantBalance,
        },
    };

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
    ) -> Result<(), Error> {
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
    ) -> Result<(), Error> {
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
    ) -> Result<OperationSignatureInfo, Error> {
        todo!()
    }

    /// Execute the mutual close flow via the `mutualClose` entrypoint by paying out the specified
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
        customer_balance: &CustomerBalance,
        merchant_balance: &MerchantBalance,
        authorization_signature: &OperationSignatureInfo,
        merchant_key_pair: &TezosKeyMaterial,
    ) -> Result<(), Error> {
        todo!()
    }
}
