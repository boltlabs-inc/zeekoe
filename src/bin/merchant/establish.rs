use {anyhow::Context, async_trait::async_trait, http::Uri, rand::rngs::StdRng};

use zkabacus_crypto::{
    merchant::Config as ZkAbacusConfig, ChannelId, Context as ProofContext, CustomerBalance,
    CustomerRandomness, MerchantBalance, MerchantRandomness, VerifiedBlindedState,
};

use zeekoe::{
    abort,
    escrow::{
        tezos,
        types::{KeyHash, TezosKeyMaterial, TezosPublicKey},
    },
    merchant::{config::Service, database::QueryMerchant, server::SessionKey, Chan},
    offer_abort, proceed,
    protocol::{self, establish, ChannelStatus, Party::Merchant},
};

use tezedge::crypto::Prefix;

use super::{approve, Method};

pub struct Establish;

#[async_trait]
impl Method for Establish {
    type Protocol = protocol::Establish;

    async fn run(
        &self,
        mut rng: StdRng,
        client: &reqwest::Client,
        tezos_key_material: TezosKeyMaterial,
        tezos_uri: Uri,
        service: &Service,
        zkabacus_merchant_config: &ZkAbacusConfig,
        database: &dyn QueryMerchant,
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error> {
        // Receive the customer's random contribution to the channel ID
        let (customer_randomness, chan) = chan
            .recv()
            .await
            .context("Failed to receive customer randomness")?;

        // Receive the customer's desired deposit into the channel
        let (customer_deposit, chan) = chan
            .recv()
            .await
            .context("Failed to receive customer balance")?;

        // Receive the customer's desired merchant contribution to the channel
        let (merchant_deposit, chan) = chan
            .recv()
            .await
            .context("Failed to receive merchant balance")?;

        // Receive the channel establishment justification note from the customer
        let (note, chan) = chan
            .recv()
            .await
            .context("Failed to receive establish note")?;

        // Receive the customer's Tezos public key (EdDSA public key)
        let (customer_tezos_public_key, chan) = chan
            .recv()
            .await
            .context("Failed to receive customer Tezos public key")?;

        // Receive the customer's Tezos account (tz1) address corresponding to that public key
        let (customer_funding_address, chan) = chan
            .recv()
            .await
            .context("Failed to receive customer Tezos funding address")?;

        // Recieve the key hash, computed over the merchant's public keys
        let (key_hash, chan) = chan.recv().await.context("Failed to receive key hash")?;

        // TODO: ensure that:
        // - customer's tezos public key is valid

        // Check that the customer's Tezos public key corresponds to their Tezos account
        let customer_keys_match = customer_tezos_public_key.hash() == customer_funding_address;

        // Check that the customer's account is actually a tz1 address
        let funding_address_is_tz1 = matches!(customer_funding_address.get_prefix(), Prefix::tz1);

        // Check that the key hash matches the merchant's expected key hash
        let merchant_keys_match = key_hash
            == KeyHash::new(
                zkabacus_merchant_config.signing_keypair().public_key(),
                tezos_key_material.funding_address(),
                tezos_key_material.public_key(),
            );

        // TODO: Add "valid tezos public key" check to this
        if !(customer_keys_match && funding_address_is_tz1 && merchant_keys_match) {
            abort!(in chan return establish::Error::Rejected("invalid inputs".into()))
        }

        // Request approval from the approval service
        let response_url = match approve::establish(
            client,
            &service.approve,
            &customer_deposit,
            &merchant_deposit,
            note,
        )
        .await
        {
            Ok(confirm_url) => confirm_url,
            Err(approval_error) => {
                let error = establish::Error::Rejected(
                    approval_error.unwrap_or_else(|| "internal error".into()),
                );
                abort!(in chan return error);
            }
        };

        // Finish the full Establish protocol
        let establish_result = approve_and_establish(
            &mut rng,
            database,
            zkabacus_merchant_config,
            customer_randomness,
            session_key,
            merchant_deposit,
            customer_deposit,
            &customer_tezos_public_key,
            &tezos_key_material,
            &tezos_uri,
            chan,
        )
        .await;

        // Report the result of the channel establishment to the approver
        match establish_result {
            Ok(()) => approve::establish_success(client, response_url).await,
            Err(_) => approve::failure(client, response_url).await,
        }

        // Return the result
        establish_result
    }
}

/// Signal to the customer that the channel has been approved to be established, and continue to the
/// end of the channel establishment protocol.
#[allow(clippy::too_many_arguments)]
#[allow(unused)]
#[allow(clippy::unreachable)]
#[allow(clippy::diverging_sub_expression)]
async fn approve_and_establish(
    rng: &mut StdRng,
    database: &dyn QueryMerchant,
    zkabacus_merchant_config: &zkabacus_crypto::merchant::Config,
    customer_randomness: CustomerRandomness,
    session_key: SessionKey,
    merchant_deposit: MerchantBalance,
    customer_deposit: CustomerBalance,
    customer_tezos_public_key: &TezosPublicKey,
    merchant_key_material: &TezosKeyMaterial,
    tezos_uri: &Uri,
    chan: Chan<establish::MerchantApproveEstablish>,
) -> Result<(), anyhow::Error> {
    // The approval service has approved
    proceed!(in chan);

    // Generate the merchant's random contribution to the channel ID
    let merchant_randomness = MerchantRandomness::new(rng);

    // Send the merchant's randomness to the customer
    let chan = chan
        .send(merchant_randomness)
        .await
        .context("Failed to send merchant randomness for channel ID")?;

    // Generate channel ID (customer will share this same value since they use the same inputs)
    let channel_id = ChannelId::new(
        merchant_randomness,
        customer_randomness,
        // Merchant's Pointcheval-Sanders public key:
        zkabacus_merchant_config.signing_keypair().public_key(),
        merchant_key_material.public_key().as_ref(),
        customer_tezos_public_key.as_ref(),
    );

    // Generate the proof context for the establish proof
    // TODO: the context should actually be formed from a session transcript up to this point
    let context = ProofContext::new(&session_key.to_bytes());

    // Receive the establish proof from the customer and validate it
    let (blinded_state, chan) = zkabacus_initialize(
        rng,
        zkabacus_merchant_config,
        context,
        &channel_id,
        merchant_deposit,
        customer_deposit,
        chan,
    )
    .await
    .context("Failed to initialize channel")?;

    // Receive contract id from customer (possibly also send block height, check spec)
    let (contract_id, chan) = chan
        .recv()
        .await
        .context("Failed to receive contract ID from customer")?;
    let (origination_level, chan) = chan
        .recv()
        .await
        .context("Failed to receive contract origination level from the customer")?;

    // TODO: check (waiting, if necessary, until a certain configurable timeout) that the
    // contract has been originated on chain and confirmed to desired block depth, and:
    // - the originated contract contains the expected zkChannels contract
    // - the originated contract's on-chain storage is as expected for the zkAbacus channel ID:
    //   * the contract storage contains the merchant's zkAbacus Pointcheval Sanders public key
    //   * the merchant's tezos tz1 address and eddsa public key match the fields merch_addr and
    //     merch_pk, respectively
    //   * the self_delay field in the contract matches the global default
    //   * the close field matches the merchant's close flag (constant close curve point)
    //   * customer deposit and merchant deposit match the initial balances in the contract
    //     storage bal_cust_0 and bal_merch_0, respectively

    // TODO: otherwise, if any of these checks fail, invoke `abort!`

    // Store the channel information in the database
    database
        .new_channel(
            &channel_id,
            &contract_id,
            &origination_level,
            &merchant_deposit,
            &customer_deposit,
        )
        .await
        .context("Failed to insert new channel_id, contract_id in database")?;

    tezos::establish::verify_customer_funding(
        &merchant_deposit,
        Some(tezos_uri),
        merchant_key_material,
        &contract_id,
        tezos::DEFAULT_CONFIRMATION_DEPTH,
    )
    .await
    .unwrap_or_else(|err| eprintln!("Could not verify customer funding: {}", err));

    // TODO: otherwise, if any of these checks fail, invoke `abort!`

    // Transition the contract state in the database from originated to customer-funded
    database
        .compare_and_swap_channel_status(
            &channel_id,
            &ChannelStatus::Originated,
            &ChannelStatus::CustomerFunded,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to update channel to CustomerFunded status (id: {})",
                &channel_id
            )
        })?;

    // If the merchant contribution was greater than zero, fund the channel on chain, and await
    // confirmation that the funding has gone through to the required confirmation depth
    if merchant_deposit.into_inner() > 0 {
        match tezos::establish::add_merchant_funding(
            Some(tezos_uri),
            &contract_id,
            &tezos::establish::MerchantFundingInformation {
                balance: merchant_deposit,
                public_key: merchant_key_material.public_key().clone(),
                address: merchant_key_material.funding_address(),
            },
            merchant_key_material,
            tezos::DEFAULT_CONFIRMATION_DEPTH,
        )
        .await
        {
            Ok((tezos::OperationStatus::Applied, _)) => {}
            _ => abort!(in chan return establish::Error::FailedMerchantFunding),
        }
    }

    // Transition the contract state in the database from customer-funded to merchant-funded
    // (where merchant-funded means that the contract storage status is OPEN)
    database
        .compare_and_swap_channel_status(
            &channel_id,
            &ChannelStatus::CustomerFunded,
            &ChannelStatus::MerchantFunded,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to update channel to MerchantFunded status (id: {})",
                &channel_id
            )
        })?;

    // Move forward in the protocol
    proceed!(in chan);

    // The customer verifies on-chain that we've funded within their desired timeout period and
    // has the chance to abort
    offer_abort!(in chan as Merchant);

    // Attempt to activate the off-chain zkChannel, setting the state in the database to the
    // active state if successful, and forwarding the pay token to the customer
    zkabacus_activate(
        rng,
        database,
        zkabacus_merchant_config,
        &channel_id,
        blinded_state,
        chan,
    )
    .await
    .context("Failed to activate channel")?;

    Ok(())
}

/// The core zkAbacus.Initialize protocol.
async fn zkabacus_initialize(
    rng: &mut StdRng,
    config: &ZkAbacusConfig,
    context: ProofContext,
    channel_id: &ChannelId,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
    chan: Chan<establish::Initialize>,
) -> Result<
    (
        VerifiedBlindedState,
        Chan<establish::CustomerSupplyContractInfo>,
    ),
    anyhow::Error,
> {
    // Receive the establish proof from the customer
    let (proof, chan) = chan
        .recv()
        .await
        .context("Failed to receive establish proof")?;

    // Attempt to initialize the channel to produce a closing signature and state commitment
    if let Some((closing_signature, blinded_state)) = config.initialize(
        rng,
        channel_id,
        customer_balance,
        merchant_balance,
        proof,
        &context,
    ) {
        // Continue, because the proof validated
        proceed!(in chan);

        // Send the closing signature to the customer
        let chan = chan
            .send(closing_signature)
            .await
            .context("Failed to send initial closing signature")?;

        // Allow customer to reject signature if it is invalid
        offer_abort!(in chan as Merchant);

        Ok((blinded_state, chan))
    } else {
        abort!(in chan return establish::Error::InvalidEstablishProof);
    }
}

/// The core zkAbacus.Activate protocol.
async fn zkabacus_activate(
    rng: &mut StdRng,
    database: &dyn QueryMerchant,
    config: &ZkAbacusConfig,
    channel_id: &ChannelId,
    blinded_state: VerifiedBlindedState,
    chan: Chan<establish::Activate>,
) -> Result<(), anyhow::Error> {
    // Generate the pay token to send to the customer
    let pay_token = config.activate(rng, blinded_state);

    // Send the pay token to the customer
    let chan = chan
        .send(pay_token)
        .await
        .context("Failed to send pay token")?;

    // Transition the channel state to active
    database
        .compare_and_swap_channel_status(
            channel_id,
            &ChannelStatus::MerchantFunded,
            &ChannelStatus::Active,
        )
        .await
        .with_context(|| {
            format!(
                "Failed to update channel to Active status (id: {})",
                &channel_id
            )
        })?;

    // Close communication with the customer
    chan.close();

    Ok(())
}
