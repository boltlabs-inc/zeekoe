use {anyhow::Context, rand::rngs::StdRng};

use zkabacus_crypto::{
    merchant::Config as ZkAbacusConfig, ChannelId, Context as ProofContext, CustomerBalance,
    CustomerRandomness, MerchantBalance, MerchantRandomness, VerifiedBlindedState,
};

use zeekoe::{
    abort,
    escrow::{
        tezos::{self, TezosClient},
        types::{KeyHash, TezosKeyMaterial, TezosPublicKey},
    },
    merchant::{config::Service, database::QueryMerchant, server::SessionKey, Chan, Config},
    offer_abort, proceed,
    protocol::{self, establish, ChannelStatus, Party::Merchant},
    timeout::WithTimeout,
};

use tezedge::crypto::Prefix;

use super::{approve, database, load_tezos_client};

pub struct Establish;

impl Establish {
    #[allow(clippy::too_many_arguments)]
    pub async fn run(
        &self,
        mut rng: StdRng,
        client: &reqwest::Client,
        config: &Config,
        service: &Service,
        zkabacus_merchant_config: &ZkAbacusConfig,
        session_key: SessionKey,
        chan: Chan<protocol::Establish>,
    ) -> Result<(), anyhow::Error> {
        let (customer_deposit, merchant_deposit, note, channel_id_contribution, chan) =
            receive_channel_request(chan, config, zkabacus_merchant_config)
                .with_timeout(6 * service.message_timeout)
                .await
                .context("Establish timed out while receiving channel request")?
                .context("Failed to receive valid channel request")?;

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
        // The approval service has approved
        proceed!(in chan);

        let establish_result = establish_channel(
            &mut rng,
            channel_id_contribution,
            zkabacus_merchant_config,
            session_key,
            config,
            service,
            merchant_deposit,
            customer_deposit,
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

/// Establish a channel.
/// This large function exists so that the approver service can catch any errors arise during establishment.
#[allow(clippy::too_many_arguments)]
async fn establish_channel(
    mut rng: &mut StdRng,
    channel_id_contribution: CustomerChannelIdContribution,
    zkabacus_merchant_config: &ZkAbacusConfig,
    session_key: SessionKey,
    config: &Config,
    service: &Service,
    merchant_deposit: MerchantBalance,
    customer_deposit: CustomerBalance,
    chan: Chan<establish::MerchantSupplyInfo>,
) -> Result<(), anyhow::Error> {
    let database = database(config).await?;
    let tezos_key_material = config.load_tezos_key_material()?;

    // Form channel ID, incorporating randomness and key material from both parties.
    let (channel_id, chan) = form_channel_id(
        chan,
        &mut rng,
        zkabacus_merchant_config,
        &tezos_key_material,
        channel_id_contribution,
    )
    .await?;

    // Generate the proof context for the establish proof
    // TODO: the context should actually be formed from a session transcript up to this point
    let context = ProofContext::new(&session_key.to_bytes());

    // Receive the establish proof from the customer and validate it
    let (blinded_state, chan) = zkabacus_initialize(
        &mut rng,
        zkabacus_merchant_config,
        context,
        channel_id,
        merchant_deposit,
        customer_deposit,
        chan,
    )
    .with_timeout(4 * service.message_timeout)
    .await
    .context("Establish timed out while initializing channel")?
    .context("Failed to initialize channel")?;

    // Verify that the customer originated and funded the channel correctly
    // Timeout accounts for posting and verification of two Tezos operations
    let chan = verify_contract(
        chan,
        config,
        database.as_ref(),
        merchant_deposit,
        customer_deposit,
        channel_id,
        zkabacus_merchant_config,
    )
    .with_timeout(2 * (service.transaction_timeout + service.verification_timeout))
    .await
    .context("Establish timed out while verifying on-chain contract state")?
    .context("Failed to verify on-chain contract state")?;

    fund_contract(database.as_ref(), config, channel_id, merchant_deposit).await?;

    let chan = notify_customer_of_funding(chan)
        .with_timeout(service.message_timeout + service.verification_timeout)
        .await
        .context("Establish timed out while waiting for customer to verify funding")?
        .context("Failed to get funding verification from customer")?;

    // Attempt to activate the off-chain zkChannel, setting the state in the database to the
    // active state if successful, and forwarding the pay token to the customer
    zkabacus_activate(
        &mut rng,
        database.as_ref(),
        zkabacus_merchant_config,
        channel_id,
        blinded_state,
        chan,
    )
    .await
    .context("Failed to activate channel")?;

    Ok(())
}

/// Receive and validate funding request from the customer.
async fn receive_channel_request(
    chan: Chan<establish::Establish>,
    config: &Config,
    zkabacus_merchant_config: &ZkAbacusConfig,
) -> Result<
    (
        CustomerBalance,
        MerchantBalance,
        String,
        CustomerChannelIdContribution,
        Chan<establish::MerchantApproveEstablish>,
    ),
    anyhow::Error,
> {
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
    let tezos_key_material = config.load_tezos_key_material()?;
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

    Ok((
        customer_deposit,
        merchant_deposit,
        note,
        CustomerChannelIdContribution {
            customer_randomness,
            customer_tezos_public_key,
        },
        chan,
    ))
}

struct CustomerChannelIdContribution {
    customer_randomness: CustomerRandomness,
    customer_tezos_public_key: TezosPublicKey,
}

/// Generate random input and form a channel ID based on the inputs from both parties.
async fn form_channel_id(
    chan: Chan<establish::MerchantSupplyInfo>,
    rng: &mut StdRng,
    zkabacus_merchant_config: &ZkAbacusConfig,
    tezos_key_material: &TezosKeyMaterial,
    channel_id_contribution: CustomerChannelIdContribution,
) -> Result<(ChannelId, Chan<establish::Initialize>), anyhow::Error> {
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
        channel_id_contribution.customer_randomness,
        // Merchant's Pointcheval-Sanders public key:
        zkabacus_merchant_config.signing_keypair().public_key(),
        tezos_key_material.public_key().as_ref(),
        channel_id_contribution.customer_tezos_public_key.as_ref(),
    );

    Ok((channel_id, chan))
}

/// Verify that the customer has correctly originated and funded the contract.
async fn verify_contract(
    chan: Chan<establish::CustomerSupplyContractInfo>,
    config: &Config,
    database: &dyn QueryMerchant,
    merchant_deposit: MerchantBalance,
    customer_deposit: CustomerBalance,
    channel_id: ChannelId,
    zkabacus_merchant_config: &ZkAbacusConfig,
) -> Result<Chan<establish::CustomerVerifyMerchantFunding>, anyhow::Error> {
    // Receive contract id from customer (possibly also send block height, check spec)
    let (contract_id, chan) = chan
        .recv()
        .await
        .context("Failed to receive contract ID from customer")?;

    let proposed_tezos_client = TezosClient {
        uri: Some(config.tezos_uri.clone()),
        contract_id: contract_id.clone(),
        client_key_pair: config.load_tezos_key_material()?,
        confirmation_depth: config.confirmation_depth,
        self_delay: config.self_delay,
    };
    match proposed_tezos_client
        .verify_origination(
            merchant_deposit,
            customer_deposit,
            zkabacus_merchant_config.signing_keypair().public_key(),
        )
        .await
    {
        Ok(()) => {}
        Err(err) => {
            eprintln!("Warning: {}", err);
            abort!(in chan return establish::Error::FailedVerifyOrigination);
        }
    };

    // Store the channel information in the database
    database
        .new_channel(
            &channel_id,
            &contract_id,
            &merchant_deposit,
            &customer_deposit,
        )
        .await
        .context("Failed to insert new channel_id, contract_id in database")?;

    // Move forward in the protocol
    proceed!(in chan);

    let (_contract_funded, chan) = chan
        .recv()
        .await
        .context("Failed to receive notification that the customer funded the contract")?;

    let tezos_client = load_tezos_client(config, &channel_id, database).await?;
    match tezos_client
        .verify_customer_funding(&merchant_deposit)
        .await
    {
        Ok(()) => {}
        Err(err) => {
            eprintln!("Warning: {}", err);
            abort!(in chan return establish::Error::FailedVerifyCustomerFunding);
        }
    };

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

    // Move forward in the protocol
    proceed!(in chan);

    Ok(chan)
}

/// Add merchant funding, if any, to the contract.
async fn fund_contract(
    database: &dyn QueryMerchant,
    config: &Config,
    channel_id: ChannelId,
    merchant_deposit: MerchantBalance,
) -> Result<(), anyhow::Error> {
    let tezos_client = load_tezos_client(config, &channel_id, database).await?;

    // If the merchant contribution was greater than zero, fund the channel on chain, and await
    // confirmation that the funding has gone through to the required confirmation depth
    if merchant_deposit.into_inner() > 0 {
        match tezos_client
            .add_merchant_funding(&tezos::MerchantFundingInformation {
                balance: merchant_deposit,
                public_key: tezos_client.client_key_pair.public_key().clone(),
                address: tezos_client.client_key_pair.funding_address(),
            })
            .await
        {
            Ok(tezos::OperationStatus::Applied) => {}
            _ => return Err(establish::Error::FailedMerchantFunding.into()),
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

    Ok(())
}

// Notify the customer that the channel is fully funded and wait for them to verify.
async fn notify_customer_of_funding(
    chan: Chan<establish::CustomerVerifyMerchantFunding>,
) -> Result<Chan<establish::Activate>, anyhow::Error> {
    let chan = chan
        .send(establish::ContractFunded)
        .await
        .context("Failed to notify customer contract was funded")?;
    offer_abort!(in chan as Merchant);

    Ok(chan)
}

/// The core zkAbacus.Initialize protocol.
async fn zkabacus_initialize(
    rng: &mut StdRng,
    config: &ZkAbacusConfig,
    context: ProofContext,
    channel_id: ChannelId,
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
        &channel_id,
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
    channel_id: ChannelId,
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
            &channel_id,
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
