use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng};

use zkabacus_crypto::{
    merchant::Config as ZkAbacusConfig, ChannelId, Context as ProofContext, CustomerBalance,
    MerchantBalance, MerchantRandomness, StateCommitment,
};

use zeekoe::{
    abort,
    merchant::{
        config::{Approver, Service},
        database::QueryMerchant,
        server::SessionKey,
        Chan,
    },
    offer_abort, proceed,
    protocol::{self, establish, ChannelStatus, ContractId, Party::Merchant},
};

use super::Method;

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
        database: &(dyn QueryMerchant + Send + Sync),
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
        if let Err(approval_error) = approve_channel_establish(
            client,
            &service.approve,
            &customer_deposit,
            &merchant_deposit,
            note,
        )
        .await
        {
            let error = establish::Error::Rejected(approval_error.unwrap_or("".into()));
            abort!(in chan return error);
        }

        // The approval service has approved
        proceed!(in chan);

        // Generate and send merchant's random contribution to the channel ID
        let merchant_randomness = MerchantRandomness::new(&mut rng);
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
            &mut rng,
            zkabacus_merchant_config,
            context,
            &channel_id,
            merchant_deposit,
            customer_deposit,
            chan,
        )
        .await
        .context("Failed to initialize channel")?;

        // TODO receive and store the following on-chain information:
        //
        // - Contract Id
        // - tz1 address
        // - Tezos EdDSA PublicKey
        //
        let contract_id = ContractId {};

        database
            .new_channel(&channel_id, &contract_id)
            .await
            .context("Failed to insert new channel_id, contract_id in database")?;

        // Look up contract and ensure it is well-formed and correctly funded.
        database
            .update_channel_status(
                &channel_id,
                &ChannelStatus::Originated,
                &ChannelStatus::CustomerFunded,
            )
            .await?;

        // Fund if necessary.
        database
            .update_channel_status(
                &channel_id,
                &ChannelStatus::CustomerFunded,
                &ChannelStatus::MerchantFunded,
            )
            .await?;

        // If not, abort.

        // Move forward in the protocol
        proceed!(in chan);

        // The customer verifies on-chain that we've funded and has the chance to abort.
        offer_abort!(in chan as Merchant);

        // Set the active state and send the pay_token.
        zkabacus_activate(
            &mut rng,
            database,
            zkabacus_merchant_config,
            chan,
            &channel_id,
            state_commitment,
        )
        .await?;

        // TODO: send alert to response_url that channel successfully established?

        Ok(())
    }
}

/// Ask the specified approver to approve the new channel balances and note (or not), returning
/// either `Ok(())` if it is approved, and `Err` if it is not approved.
///
/// Rejected channels may provide an `Option<String>` indicating the reason for the channel's
/// rejection, where `None` indicates that it was rejected due to an internal error in the approver
/// service. This information is forwarded directly to the customer, so we do not provide further
/// information about the nature of the internal error, to prevent internal state leakage.
async fn approve_channel_establish(
    _client: &reqwest::Client,
    approver: &Approver,
    _customer_balance: &CustomerBalance,
    _merchant_balance: &MerchantBalance,
    _establish_note: String,
) -> Result<(), Option<String>> {
    match approver {
        Approver::Automatic => Ok(()),
        Approver::Url(_) => todo!("External approver support not yet implemented"),
    }
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

    match config.initialize(
        rng,
        channel_id,
        customer_balance,
        merchant_balance,
        proof,
        &context,
    ) {
        Some((closing_signature, state_commitment)) => {
            // Send closing signature to customer
            proceed!(in chan);
            let chan = chan
                .send(closing_signature)
                .await
                .context("Failed to send initial closing signature")?;

            // Allow customer to reject signature if it is invalid
            offer_abort!(in chan as Merchant);

            Ok((state_commitment, chan))
        }
        None => {
            let error = establish::Error::InvalidEstablishProof;
            abort!(in chan return error);
        }
    }
}

/// The core zkAbacus.Activate protocol.
async fn zkabacus_activate(
    rng: &mut StdRng,
    database: &(dyn QueryMerchant + Send + Sync),
    config: &ZkAbacusConfig,
    chan: Chan<establish::Activate>,
    channel_id: &ChannelId,
    state_commitment: StateCommitment,
) -> Result<(), anyhow::Error> {
    database
        .update_channel_status(
            channel_id,
            &ChannelStatus::MerchantFunded,
            &ChannelStatus::Active,
        )
        .await?;
    // Generate and send pay token.
    let pay_token = config.activate(rng, state_commitment);
    let chan = chan
        .send(pay_token)
        .await
        .context("Failed to send pay token.")?;
    chan.close();
    Ok(())
}
