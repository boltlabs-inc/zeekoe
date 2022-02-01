use {
    anyhow::Context,
    async_trait::async_trait,
    rand::rngs::StdRng,
    serde::Serialize,
    std::{convert::TryInto, fs::File, path::PathBuf},
};

use std::convert::Infallible;

use zkabacus_crypto::{
    customer::{Inactive, Requested},
    ChannelId, Context as ProofContext, CustomerBalance, CustomerRandomness, MerchantBalance,
    PublicKey, CLOSE_SCALAR,
};

use crate::{
    abort,
    customer::{
        cli::Establish,
        client::ZkChannelAddress,
        database::{zkchannels_state, QueryCustomer, QueryCustomerExt, State},
        Chan, ChannelName, Config,
    },
    escrow::{
        tezos,
        types::{ContractDetails, KeyHash},
    },
    offer_abort, proceed,
    protocol::{establish, Party::Customer},
    timeout::WithTimeout,
};

use tezedge::crypto::Prefix;
use tracing::{error, info};

use super::{connect, database, load_tezos_client, Command};

#[derive(Debug, Clone, Serialize)]
struct Establishment {
    merchant_ps_public_key: PublicKey,
    customer_deposit: CustomerBalance,
    merchant_deposit: MerchantBalance,
    channel_id: ChannelId,
    close_scalar_bytes: [u8; 32],
}

#[async_trait]
impl Command for Establish {
    type Output = ();

    async fn run(
        self,
        mut rng: StdRng,
        config: self::Config,
    ) -> Result<Self::Output, anyhow::Error> {
        let Self {
            label,
            merchant: address,
            deposit,
            merchant_deposit,
            note,
            off_chain,
            ..
        } = self;

        // Connect to the customer database
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;

        // Format deposit amounts as the correct types
        let customer_balance = deposit.try_into()?;

        let merchant_balance = match merchant_deposit {
            None => MerchantBalance::zero(),
            Some(deposit) => deposit.try_into()?,
        };

        // Run a **separate** session to get the merchant's public parameters
        let (zkabacus_customer_config, contract_details) =
            get_parameters(&config, &address).await?;

        // Connect with the merchant...
        let (session_key, chan) = connect(&config, &address)
            .await
            .context("Failed to connect to merchant")?;

        // ...and select the Establish session
        let chan = chan
            .choose::<1>()
            .await
            .context("Failed to select channel establishment session")?;

        // Load the customer's Tezos account details
        let tezos_key_material = config.load_tezos_key_material()?;

        // Format the customer and merchant funding information
        let merchant_funding_info = tezos::MerchantFundingInformation {
            balance: merchant_balance,
            address: contract_details.merchant_funding_address(),
            public_key: contract_details.merchant_tezos_public_key.clone(),
        };
        let customer_funding_info = tezos::CustomerFundingInformation {
            balance: customer_balance,
            address: tezos_key_material.funding_address(),
            public_key: tezos_key_material.public_key().clone(),
        };

        // Send initial request for a new channel with the specified funding information
        // Timeout accounts for 8 messages sent and received, plus extra time to get approval
        let (channel_id, chan) = async {
            // Generate randomness for the channel ID
            let customer_randomness = CustomerRandomness::new(&mut rng);

            // Read the contents of the channel establishment note, if any: this is the justification,
            // if any is needed, for why the channel should be allowed to be established (format
            // unspecified, specific to merchant)
            let note = note.unwrap_or_default().read(config.max_note_length)?;

            // Compute a hash of the merchant's public key material.
            let key_hash = KeyHash::new(
                zkabacus_customer_config.merchant_public_key(),
                merchant_funding_info.address.clone(),
                &merchant_funding_info.public_key,
            );

            // Send the request for the funding of the channel
            let chan = chan
                .send(customer_randomness)
                .await
                .context("Failed to send customer randomness for channel ID")?
                .send(customer_funding_info.balance)
                .await
                .context("Failed to send customer deposit amount")?
                .send(merchant_funding_info.balance)
                .await
                .context("Failed to send merchant deposit amount")?
                .send(note)
                .await
                .context("Failed to send channel establishment note")?
                .send(customer_funding_info.public_key.clone())
                .await
                .context("Failed to send customer's Tezos public key")?
                .send(customer_funding_info.address.clone())
                .await
                .context("Failed to send customer's Tezos account")?
                .send(key_hash)
                .await
                .context("Failed to send hash of merchant's public keys")?;

            // Allow the merchant to reject the funding of the channel, else continue
            offer_abort!(in chan as Customer);

            // Receive merchant randomness contribution to the channel ID formation
            let (merchant_randomness, chan) = chan
                .recv()
                .await
                .context("Failed to receive merchant randomness for channel ID")?;

            // Generate channel ID (merchant will share this same value since they use the same inputs)
            let channel_id = ChannelId::new(
                merchant_randomness,
                customer_randomness,
                // Merchant's Pointcheval-Sanders public key:
                zkabacus_customer_config.merchant_public_key(),
                // Merchant's Tezos public key
                merchant_funding_info.public_key.as_ref(),
                // Customer's Tezos public key
                customer_funding_info.public_key.as_ref(),
            );

            Ok((channel_id, chan))
        }
        .with_timeout(8 * config.message_timeout + config.approval_timeout)
        .await
        .context("Establish timed out while waiting for channel approval")?
        .context("Channel was not approved by merchant")?;

        // Generate the proof context for the establish proof
        // TODO: the context should actually be formed from a session transcript up to this point
        let context = ProofContext::new(&session_key.to_bytes());

        let zkabacus_request_parameters = ZkAbacusRequestParameters {
            channel_id,
            merchant_balance,
            customer_balance,
            context,
        };

        // Run zkAbacus.Initialize
        // Timeout accounts for 4 messages sent and received
        let (channel_name, chan) = zkabacus_initialize(
            &mut rng,
            database.as_ref(),
            &zkabacus_customer_config,
            zkabacus_request_parameters,
            &contract_details,
            &address,
            chan,
            label,
        )
        .with_timeout(4 * config.message_timeout)
        .await
        .context("Establish timed out while initializing channel")?
        .context("Failed to initialize the channel")?;

        // Originate contract
        if off_chain {
            // Write out establishment struct to disk if operating in off-chain mode
            let establishment = Establishment {
                merchant_ps_public_key: zkabacus_customer_config.merchant_public_key().clone(),
                customer_deposit: customer_funding_info.balance,
                merchant_deposit: merchant_funding_info.balance,
                channel_id,
                close_scalar_bytes: CLOSE_SCALAR.to_bytes(),
            };
            write_establish_json(&establishment)?;
        }
        let (contract_id, origination_status) = if off_chain {
            // TODO: prompt user to submit the origination of the contract
            todo!("prompt user to submit contract origination details")
        } else {
            let tezos_key_material = config.load_tezos_key_material()?;
            // Originate the contract on-chain
            tezos::originate(
                Some(&config.tezos_uri),
                &merchant_funding_info,
                &customer_funding_info,
                zkabacus_customer_config.merchant_public_key(),
                &tezos_key_material,
                &channel_id,
                config.confirmation_depth,
                config.self_delay,
            )
            .await
            .context("Failed to originate contract on-chain")?
        };

        // Check to make sure origination succeeded
        if !matches!(origination_status, tezos::OperationStatus::Applied) {
            todo!("Abort protocol because origination failed?")
        }

        // Update database to indicate successful contract origination.
        database
            .with_channel_state(
                &channel_name,
                zkchannels_state::Inactive,
                |inactive| -> Result<_, Infallible> { Ok((State::Originated(inactive), ())) },
            )
            .await
            .context(format!(
                "Failed to update channel {} to Originated status",
                &channel_name
            ))??;

        database
            .initialize_contract_details(&channel_name, &contract_id)
            .await
            .context(format!(
                "Failed to store contract details for {}",
                &channel_name
            ))?;

        // Notify merchant that the contract successfully originated and wait for them to verify
        let chan = async {
            let contract_details = database.contract_details(&channel_name).await?;
            let contract_id = contract_details
                .contract_id
                .context("Contract ID not set")?;

            // Send the contract id to the merchant.
            let chan = chan
                .send(contract_id)
                .await
                .context("Failed to send contract id to merchant")?;
            offer_abort!(in chan as Customer);

            Ok(chan)
        }
        .with_timeout(config.message_timeout + config.verification_timeout)
        .await
        .context("Establish timed out while waiting for merchant to verify originated contract")?
        .context("Merchant failed to verify originated contract")?;

        // Fund the channel
        let customer_funding_status = if off_chain {
            // TODO: prompt user to fund the contract on chain
            todo!("prompt user to fund contract on chain and submit details")
        } else {
            let tezos_client = load_tezos_client(&config, &channel_name, database.as_ref()).await?;
            tezos_client
                .add_customer_funding(&customer_funding_info)
                .await?
        };

        // Check to make sure funding succeeded
        if !matches!(customer_funding_status, tezos::OperationStatus::Applied) {
            todo!("Abort protocol because funding failed?")
        }

        // Update database to indicate successful customer funding.
        database
            .with_channel_state(
                &channel_name,
                zkchannels_state::Originated,
                |inactive| -> Result<_, Infallible> { Ok((State::CustomerFunded(inactive), ())) },
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to update channel {} to CustomerFunded status",
                    channel_name
                )
            })??;

        // Allow the merchant to confirm customer funding, then confirm merchant funding
        // Timeout is set to allow each party to receive notification of funding and to verify it
        // on chain, plus time for the merchant to post funding on chain.
        let chan = async {
            let chan = chan
                .send(establish::ContractFunded)
                .await
                .context("Failed to notify merchant contract was funded")?;

            // Wait for merchant to confirm funding
            offer_abort!(in chan as Customer);

            // Allow the merchant to indicate whether it funded the channel
            let (_contract_funded, chan) = chan
                .recv()
                .await
                .context("Failed to receive merchant funding confirmation")?;

            let merchant_funding_successful: bool = if off_chain {
                // TODO: prompt user to check that the merchant funding was provided
                true
            } else {
                let tezos_client =
                    load_tezos_client(&config, &channel_name, database.as_ref()).await?;
                tezos_client.verify_merchant_funding().await.map_or_else(
                    |err| {
                        error!("Could not verify merchant funding: {}", err);
                        false
                    },
                    |_| true,
                )
            };

            // Abort if merchant funding was not successful
            if !merchant_funding_successful {
                abort!(in chan return establish::Error::FailedMerchantFunding);
            }

            // Update database to indicate successful merchant funding.
            database
                .with_channel_state(
                    &channel_name,
                    zkchannels_state::CustomerFunded,
                    |inactive| -> Result<_, Infallible> {
                        Ok((State::MerchantFunded(inactive), ()))
                    },
                )
                .await
                .with_context(|| {
                    format!(
                        "Failed to update channel {} to MerchantFunded status",
                        channel_name
                    )
                })??;

            proceed!(in chan);

            Ok(chan)
        }
        .with_timeout(
            2 * (config.message_timeout + config.verification_timeout) + config.transaction_timeout,
        )
        .await
        .context("Establish timed out waiting for funding confirmation")?
        .context("Failed to confirm that both parties funded the channel")?;

        // Run zkAbacus.Activate
        // Timeout accounts for one message sent and reacted to
        zkabacus_activate(
            &config,
            database.as_ref(),
            &channel_name,
            chan,
            &zkabacus_customer_config,
        )
        .with_timeout(2 * config.message_timeout)
        .await
        .context("Establish timed out while activating channel")?
        .context("Failed to activate channel")?;

        // Print success
        info!(
            "Successfully established new channel with label \"{}\"",
            channel_name
        );

        Ok(())
    }
}

/// Fetch the merchant's public parameters.
async fn get_parameters(
    config: &Config,
    address: &ZkChannelAddress,
) -> Result<(zkabacus_crypto::customer::Config, ContractDetails), anyhow::Error> {
    // Connect to the merchant
    let (_session_key, chan) = connect(config, address).await?;

    // Select the get-parameters session
    let chan = chan.choose::<0>().await?;

    // Get the merchant's Pointcheval-Sanders public key
    let (merchant_public_key, chan) = chan
        .recv()
        .await
        .context("Failed to receive merchant's Pointcheval-Sanders public key")?;

    // Get the merchant's commitment parameters
    let (revocation_commitment_parameters, chan) = chan
        .recv()
        .await
        .context("Failed to receive merchant's revocation commitment parameters")?;

    // Get the merchant's range proof parameters
    let (range_constraint_parameters, chan) = chan
        .recv()
        .await
        .context("Failed to receive merchant's range proof parameters")?;

    if range_constraint_parameters.validate().is_err() {
        return Err(establish::Error::InvalidParameters.into());
    }

    // Get the merchant's tz1 address
    let (merchant_funding_address, chan) = chan
        .recv()
        .await
        .context("Failed to receive merchant's funding address")?;

    // Get the merchant's Tezos public key
    let (merchant_tezos_public_key, chan) = chan
        .recv()
        .await
        .context("Failed to receive merchant's Tezos public key")?;

    chan.close();

    // Check that merchant's tezos public key corresponds to the tezos account that they specified
    let merchant_account_matches = merchant_tezos_public_key.hash() == merchant_funding_address;

    // Check that address is actually a tz1 address - e.g. uses EdDSA signature scheme.
    let merchant_address_is_tz1 = matches!(merchant_funding_address.get_prefix(), Prefix::tz1);

    if !(merchant_account_matches && merchant_address_is_tz1) {
        return Err(establish::Error::InvalidParameters.into());
    }

    Ok((
        zkabacus_crypto::customer::Config::from_parts(
            merchant_public_key,
            revocation_commitment_parameters,
            range_constraint_parameters,
        ),
        ContractDetails {
            merchant_tezos_public_key,
            contract_id: None,
        },
    ))
}

struct ZkAbacusRequestParameters {
    channel_id: ChannelId,
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
    context: ProofContext,
}

/// The core zkAbacus.Initialize protocol.
///
/// If successful returns the [`ChannelName`] that the channel was *actually* inserted into the
/// database using (which may differ from the one specified if the one specified was already in
/// use!), and the [`Chan`] ready for the next part of the establish protocol.
#[allow(clippy::too_many_arguments)]
async fn zkabacus_initialize(
    mut rng: &mut StdRng,
    database: &dyn QueryCustomer,
    zkabacus_config: &zkabacus_crypto::customer::Config,
    request_parameters: ZkAbacusRequestParameters,
    contract_details: &ContractDetails,
    address: &ZkChannelAddress,
    chan: Chan<establish::Initialize>,
    channel_name: Option<ChannelName>,
) -> Result<(ChannelName, Chan<establish::CustomerSupplyContractInfo>), anyhow::Error> {
    let (requested, proof) = Requested::new(
        &mut rng,
        zkabacus_config,
        request_parameters.channel_id,
        request_parameters.merchant_balance,
        request_parameters.customer_balance,
        &request_parameters.context,
    );

    // Send the establish proof
    let chan = chan
        .send(proof)
        .await
        .context("Failed to send establish proof")?;

    // Allow the merchant to reject the establish proof
    offer_abort!(in chan as Customer);

    // Receive a closing signature
    let (closing_signature, chan) = chan
        .recv()
        .await
        .context("Failed to receive closing signature")?;

    // Attempt to validate the closing signature
    let inactive = match requested.complete(closing_signature, zkabacus_config) {
        Ok(inactive) => inactive,
        Err(_) => abort!(in chan return establish::Error::InvalidClosingSignature),
    };

    // Move forward in the protocol
    proceed!(in chan);

    // Store the inactive channel state in the database
    let label = store_inactive_local(
        database,
        zkabacus_config,
        address,
        inactive,
        contract_details,
        channel_name,
    )
    .await
    .context("Failed to store inactive channel state in local database")?;

    Ok((label, chan))
}

/// Store an [`Inactive`] channel state in the database with a given label and address. If the label
/// is already in use, find another label that is not and return that.
async fn store_inactive_local(
    database: &dyn QueryCustomer,
    zkabacus_config: &zkabacus_crypto::customer::Config,
    address: &ZkChannelAddress,
    inactive: Inactive,
    contract_details: &ContractDetails,
    channel_name: Option<ChannelName>,
) -> Result<ChannelName, anyhow::Error> {
    // Use the specified label, or else use the `ZkChannelAddress` as a string
    let label = channel_name.unwrap_or_else(|| ChannelName::new(address.to_string()));

    // Try inserting the inactive state with this label
    match database
        .new_channel(&label, address, inactive, contract_details, zkabacus_config)
        .await
    {
        Ok(()) => Ok(label),
        Err((_returned_inactive, error)) => {
            // TODO: what to do with the `Inactive` state here when the database has failed to allow us to persist it?
            Err(error.into())
        }
    }
}

/// The core zkAbacus.Activate protocol.
async fn zkabacus_activate(
    config: &Config,
    database: &dyn QueryCustomer,
    label: &ChannelName,
    chan: Chan<establish::Activate>,
    zkabacus_customer_config: &zkabacus_crypto::customer::Config,
) -> Result<(), anyhow::Error> {
    // Receive the pay token from the merchant
    let (pay_token, chan) = chan
        .recv()
        .await
        .context("Failed to receive blinded pay token")?;

    // Close communication with the merchant
    chan.close();

    // Try to run the zkAbacus.Activate subprotocol.
    // If it succeeds, update the channel status to `Ready`.
    database
        .with_channel_state(
            label,
            zkchannels_state::MerchantFunded,
            // This closure tries to run zkAbacus.Activate
            |inactive: Inactive| match inactive.activate(pay_token, zkabacus_customer_config) {
                Ok(ready) => Ok((State::Ready(ready), ())),
                Err(_) => Err(establish::Error::InvalidPayToken),
            },
        )
        .await
        .with_context(|| format!("Failed to update channel {} to Ready status", &label))??;

    // Notify the on-chain monitoring daemon that there's a new channel.
    refresh_daemon(config).await
}

/// Write the establish_json if performing operations off-chain.
fn write_establish_json(establishment: &Establishment) -> Result<(), anyhow::Error> {
    // Write the establishment information to disk
    let establish_json_path = PathBuf::from(format!(
        "{}.establish.json",
        hex::encode(establishment.channel_id.to_bytes())
    ));
    let mut establish_file = File::create(&establish_json_path).with_context(|| {
        format!(
            "Could not open file for writing: {:?}",
            &establish_json_path
        )
    })?;
    serde_json::to_writer(&mut establish_file, &establishment).with_context(|| {
        format!(
            "Could not write establishment data to file: {:?}",
            &establish_json_path
        )
    })?;

    eprintln!("Establishment data written to {:?}", &establish_json_path);
    Ok(())
}

/// Invoke `Refresh` on the customer daemon.
async fn refresh_daemon(_config: &Config) -> anyhow::Result<()> {
    // TODO: if daemon becomes relevant as a server, uncomment this
    // let (_session_key, chan) = connect_daemon(config)
    //     .await
    //     .context("Failed to connect to daemon")?;

    // chan.choose::<0>()
    //     .await
    //     .context("Failed to select daemon Refresh")?
    //     .close();

    Ok(())
}
