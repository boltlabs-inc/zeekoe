use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng};

use zkabacus_crypto::{
    merchant::Config as ZkAbacusConfig, ChannelId, Context as ProofContext, CustomerBalance,
    CustomerRandomness, MerchantBalance, MerchantRandomness, StateCommitment,
};

use zeekoe::{abort, escrow::types::ContractId, merchant::{config::Service, database::QueryMerchant, server::SessionKey, Chan}, offer_abort, proceed, protocol::{self, establish, ChannelStatus, Party::Merchant}};

use super::{approve, Method};

pub struct Establish;

#[async_trait]
impl Method for Establish {
    type Protocol = protocol::Establish;

    async fn run(
        &self,
        mut rng: StdRng,
        client: &reqwest::Client,
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

        // TODO: customer sends merchant:
        // - customer's tezos public key (eddsa public key)
        // - customer's tezos account tz1 address corresponding to that public key
        // - SHA3-256 of:
        //   * merchant's pointcheval-sanders public key (`zkabacus_crypto::PublicKey`)
        //   * tz1 address corresponding to merchant's public key
        //   * merchant's tezos public key

        // TODO: ensure that:
        // - customer's tezos public key is valid
        // - customer's tezos public key corresponds to the tezos account that they specified
        // - that address is actually a tz1 address
        // - submitted hash verifies against the **merchant's** pointcheval-sanders public key, tz1
        //   address, and tezos public key

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
                let error =
                    establish::Error::Rejected(approval_error.unwrap_or("internal error".into()));
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
async fn approve_and_establish(
    rng: &mut StdRng,
    database: &dyn QueryMerchant,
    zkabacus_merchant_config: &zkabacus_crypto::merchant::Config,
    customer_randomness: CustomerRandomness,
    session_key: SessionKey,
    merchant_deposit: MerchantBalance,
    customer_deposit: CustomerBalance,
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
        &[], // TODO: fill this in with bytes from merchant's tezos public key
        &[], // TODO: fill this in with bytes from customer's tezos public key
    );

    // Generate the proof context for the establish proof
    // TODO: the context should actually be formed from a session transcript up to this point
    let context = ProofContext::new(&session_key.to_bytes());

    // Receive the establish proof from the customer and validate it
    let (state_commitment, chan) = zkabacus_initialize(
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

    // TODO: receive contract id from customer (possibly also send block height, check spec)
    let contract_id: ContractId = todo!();

    // NOTE: This set of on-chain verification checks is **subtly insufficient** unless the
    // on-chain contract's state machine is acyclic, which at the time of writing of this note
    // (June 25, 2021), it is not. We anticipate fixing this soon.

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
            &merchant_deposit,
            &customer_deposit,
        )
        .await
        .context("Failed to insert new channel_id, contract_id in database")?;

    // TODO: check (waiting, if necessary, until a certain configurable timeout) that the
    // contract has been funded by the customer on chain and confirmed to desired block depth:
    // - if merchant contribution is zero, check contract storage is set to OPEN state for the
    //   required confirmation depth
    // - if merchant contribution is greater than zero, check contract storage is set to
    //   AWAITING FUNDING state for the required confirmation depth

    // TODO: otherwise, if any of these checks fail, invoke `abort!`

    // Transition the contract state in the database from originated to customer-funded
    database
        .compare_and_swap_channel_status(
            &channel_id,
            &ChannelStatus::Originated,
            &ChannelStatus::CustomerFunded,
        )
        .await
        .context("Failed to update database to indicate channel was customer-funded")?;

    // TODO: If the merchant contribution was greater than zero, fund the channel on chain, and
    // await confirmation that the funding has gone through to the required confirmation depth

    // TODO: If anything goes wrong, invoke `abort!`

    // Transition the contract state in the database from customer-funded to merchant-funded
    // (where merchant-funded means that the contract storage status is OPEN)
    database
        .compare_and_swap_channel_status(
            &channel_id,
            &ChannelStatus::CustomerFunded,
            &ChannelStatus::MerchantFunded,
        )
        .await
        .context("Failed to update database to indicate channel was merchant-funded")?;

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
        state_commitment,
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
) -> Result<(StateCommitment, Chan<establish::CustomerSupplyContractInfo>), anyhow::Error> {
    // Receive the establish proof from the customer
    let (proof, chan) = chan
        .recv()
        .await
        .context("Failed to receive establish proof")?;

    // Attempt to initialize the channel to produce a closing signature and state commitment
    if let Some((closing_signature, state_commitment)) = config.initialize(
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

        Ok((state_commitment, chan))
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
    state_commitment: StateCommitment,
    chan: Chan<establish::Activate>,
) -> Result<(), anyhow::Error> {
    // Transition the channel state to active
    database
        .compare_and_swap_channel_status(
            channel_id,
            &ChannelStatus::MerchantFunded,
            &ChannelStatus::Active,
        )
        .await?;

    // Generate the pay token to send to the customer
    let pay_token = config.activate(rng, state_commitment);

    // Send the pay token to the customer
    let chan = chan
        .send(pay_token)
        .await
        .context("Failed to send pay token")?;

    // Close communication with the customer
    chan.close();

    Ok(())
}
