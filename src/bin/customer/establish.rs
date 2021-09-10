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

use zeekoe::{
    abort,
    customer::{
        cli::Establish,
        client::ZkChannelAddress,
        database::{self, zkchannels_state, QueryCustomer, QueryCustomerExt, State},
        Chan, ChannelName, Config,
    },
    escrow::types::{ContractDetails, KeyHash},
    offer_abort, proceed,
    protocol::{
        establish,
        Party::{Customer, Merchant},
    },
};

use tezedge::crypto::Prefix;
use zeekoe::escrow::tezos;

use super::{connect, database, Command};

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
    async fn run(self, mut rng: StdRng, config: self::Config) -> Result<(), anyhow::Error> {
        let Self {
            label,
            merchant: address,
            ..
        } = self;

        // Connect to the customer database
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;

        // Generate randomness for the channel ID
        let customer_randomness = CustomerRandomness::new(&mut rng);

        // Format deposit amounts as the correct types
        let customer_balance = CustomerBalance::try_new(
            self.deposit
                .try_into_minor_units()
                .ok_or(establish::Error::InvalidDeposit(Customer))?
                .try_into()?,
        )
        .map_err(|_| establish::Error::InvalidDeposit(Customer))?;

        let merchant_balance = MerchantBalance::try_new(match self.merchant_deposit {
            None => 0,
            Some(d) => d
                .try_into_minor_units()
                .ok_or(establish::Error::InvalidDeposit(Merchant))?
                .try_into()?,
        })
        .map_err(|_| establish::Error::InvalidDeposit(Merchant))?;

        // Load the customer's Tezos account details
        let tezos_key_material = config
            .load_tezos_key_material()
            .await
            .context("Failed to load customer key material")?;
        let tezos_public_key = tezos_key_material.public_key().clone();
        let tezos_address = tezos_public_key.hash();

        // Run a **separate** session to get the merchant's public parameters
        let (zkabacus_customer_config, contract_details) =
            get_parameters(&config, &address).await?;

        // Compute a hash of the merchant's public key material.
        let key_hash = KeyHash::new(
            zkabacus_customer_config.merchant_public_key(),
            contract_details.merchant_funding_address(),
            &contract_details.merchant_tezos_public_key,
        );

        // Connect and select the Establish session
        let (session_key, chan) = connect(&config, &address)
            .await
            .context("Failed to connect to merchant")?;
        let chan = chan
            .choose::<1>()
            .await
            .context("Failed to select channel establishment session")?;

        // Read the contents of the channel establishment note, if any: this is the justification,
        // if any is needed, for why the channel should be allowed to be established (format
        // unspecified, specific to merchant)
        let note = self
            .note
            .unwrap_or_else(|| zeekoe::customer::cli::Note::String(String::from("")))
            .read(config.max_note_length)?;

        // Send the request for the funding of the channel
        let chan = chan
            .send(customer_randomness)
            .await
            .context("Failed to send customer randomness for channel ID")?
            .send(customer_balance)
            .await
            .context("Failed to send customer deposit amount")?
            .send(merchant_balance)
            .await
            .context("Failed to send merchant deposit amount")?
            .send(note)
            .await
            .context("Failed to send channel establishment note")?
            .send(tezos_public_key.clone())
            .await
            .context("Failed to send customer's Tezos public key")?
            .send(tezos_address.clone())
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
            contract_details.merchant_tezos_public_key.as_ref(),
            // Customer's Tezos public key
            tezos_key_material.public_key().as_ref(),
        );

        // Generate the proof context for the establish proof
        // TODO: the context should actually be formed from a session transcript up to this point
        let context = ProofContext::new(&session_key.to_bytes());

        // Use the specified label, or else use the `ZkChannelAddress` as a string
        let label = label.unwrap_or_else(|| ChannelName::new(format!("{}", address)));

        // Collect the information we need to write info out to disk if necessary
        let establishment = Establishment {
            merchant_ps_public_key: zkabacus_customer_config.merchant_public_key().clone(),
            customer_deposit: customer_balance,
            merchant_deposit: merchant_balance,
            channel_id,
            close_scalar_bytes: CLOSE_SCALAR.to_bytes(),
        };

        let zkabacus_request_parameters = ZkAbacusRequestParameters {
            customer_config: zkabacus_customer_config,
            channel_id,
            merchant_balance,
            customer_balance,
            context,
        };

        // Run zkAbacus.Initialize
        let (actual_label, chan) = zkabacus_initialize(
            &mut rng,
            database.as_ref(),
            zkabacus_request_parameters,
            &contract_details,
            label,
            &address,
            chan,
        )
        .await
        .context("Failed to initialize the channel")?;

        // TODO: parameterize these hard-coded defaults
        let uri = "https://rpc.tzkt.io/edo2net/".parse().unwrap();

        // Write out establishment struct to disk if operating in off-chain mode
        if self.off_chain {
            write_establish_json(&establishment)?;
        }

        // The customer and merchant funding information
        let merchant_funding_info = tezos::establish::MerchantFundingInformation {
            balance: merchant_balance,
            address: contract_details.merchant_funding_address(),
            public_key: contract_details.merchant_tezos_public_key.clone(),
        };
        let customer_funding_info = tezos::establish::CustomerFundingInformation {
            balance: customer_balance,
            address: tezos_address.clone(),
            public_key: tezos_public_key.clone(),
        };

        let (contract_id, origination_status, origination_level) = if self.off_chain {
            // TODO: prompt user to submit the origination of the contract
            todo!("prompt user to submit contract origination details");
        } else {
            // Originate the contract on-chain
            tezos::establish::originate(
                Some(&uri),
                &merchant_funding_info,
                &customer_funding_info,
                &establishment.merchant_ps_public_key,
                &tezos_key_material,
                &channel_id,
                tezos::DEFAULT_CONFIRMATION_DEPTH,
                tezos::DEFAULT_SELF_DELAY,
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
                &actual_label,
                zkchannels_state::Inactive,
                |inactive| -> Result<_, Infallible> { Ok((State::Originated(inactive), ())) },
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to update channel {} to Originated status",
                    &actual_label
                )
            })??;

        let (customer_funding_status, _customer_funding_level) = if self.off_chain {
            // TODO: prompt user to fund the contract on chain
            todo!("prompt user to fund contract on chain and submit details")
        } else {
            tezos::establish::add_customer_funding(
                Some(&uri),
                &contract_id,
                &customer_funding_info,
                &tezos_key_material,
                tezos::DEFAULT_CONFIRMATION_DEPTH,
            )
            .await
            .context("Failed to fund contract on-chain")?
        };

        // Check to make sure funding succeeded
        if !matches!(customer_funding_status, tezos::OperationStatus::Applied) {
            todo!("Abort protocol because funding failed?")
        }

        // Update database to indicate successful customer funding.
        database
            .with_channel_state(
                &actual_label,
                zkchannels_state::Originated,
                |inactive| -> Result<_, Infallible> { Ok((State::CustomerFunded(inactive), ())) },
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to update channel {} to CustomerFunded status",
                    &actual_label
                )
            })??;

        // Send the contract id and level to the merchant
        let chan = chan
            .send(contract_id)
            .await
            .context("Failed to send contract id to merchant")?
            .send(origination_level)
            .await
            .context("Failed to send contract origination level to merchant")?;

        // Allow the merchant to indicate whether it funded the channel
        offer_abort!(in chan as Customer);

        // FIXME: remove this once filled in
        #[allow(clippy::if_same_then_else, clippy::needless_bool)]
        let merchant_funding_successful: bool = if self.off_chain {
            // TODO: prompt user to check that the merchant funding was provided
            true
        } else {
            // TODO: if merchant contribution was non-zero, check that merchant funding was provided
            // within a configurable timeout and to the desired block depth and that the status of
            // the contract is locked. if not, recommend unilateral close
            // Note: the following database update may be moved around once the merchant funding
            // check is added.
            true // FIXME: check this!
        };

        // Abort if merchant funding was not successful
        if !merchant_funding_successful {
            abort!(in chan return establish::Error::FailedMerchantFunding);
        }

        // Update database to indicate successful merchant funding.
        database
            .with_channel_state(
                &actual_label,
                zkchannels_state::CustomerFunded,
                |inactive| -> Result<_, Infallible> { Ok((State::MerchantFunded(inactive), ())) },
            )
            .await
            .with_context(|| {
                format!(
                    "Failed to update channel {} to MerchantFunded status",
                    &actual_label
                )
            })??;

        proceed!(in chan);

        // Run zkAbacus.Activate
        zkabacus_activate(&config, database.as_ref(), &actual_label, chan)
            .await
            .context("Failed to activate channel")?;

        // Print success
        eprintln!(
            "Successfully established new channel with label \"{}\"",
            actual_label
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

    // TODO: ensure that:
    // - merchant's public key (in the config) is a valid Pointcheval-Sanders public key
    // - merchant's range proof parameters consist of valid Pointcheval-Sanders public key and
    //   valid signatures on the correct range
    // - merchant's commitment parameters are valid Pedersen parameters
    // - merchant's tezos public key is valid

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
            contract_level: None,
        },
    ))
}

struct ZkAbacusRequestParameters {
    customer_config: zkabacus_crypto::customer::Config,
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
async fn zkabacus_initialize(
    mut rng: &mut StdRng,
    database: &dyn QueryCustomer,
    request_parameters: ZkAbacusRequestParameters,
    contract_details: &ContractDetails,
    label: ChannelName,
    address: &ZkChannelAddress,
    chan: Chan<establish::Initialize>,
) -> Result<(ChannelName, Chan<establish::CustomerSupplyContractInfo>), anyhow::Error> {
    let (requested, proof) = Requested::new(
        &mut rng,
        request_parameters.customer_config,
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
    let inactive = match requested.complete(closing_signature) {
        Ok(inactive) => inactive,
        Err(_) => abort!(in chan return establish::Error::InvalidClosingSignature),
    };

    // Move forward in the protocol
    proceed!(in chan);

    // Store the inactive channel state in the database
    let actual_label = store_inactive_local(database, label, address, inactive, contract_details)
        .await
        .context("Failed to store inactive channel state in local database")?;

    Ok((actual_label, chan))
}

/// Store an [`Inactive`] channel state in the database with a given label and address. If the label
/// is already in use, find another label that is not and return that.
async fn store_inactive_local(
    database: &dyn QueryCustomer,
    label: ChannelName,
    address: &ZkChannelAddress,
    mut inactive: Inactive,
    contract_details: &ContractDetails,
) -> Result<ChannelName, anyhow::Error> {
    // This loop iterates trying to insert the channel, adding suffixes "(1)", "(2)", etc.
    // onto the label name until it finds an unused label
    let mut count = 0;
    let actual_label = loop {
        let actual_label = if count > 0 {
            ChannelName::new(format!("{} ({})", label, count))
        } else {
            label.clone()
        };

        // Try inserting the inactive state with this label
        match database
            .new_channel(&actual_label, address, inactive, contract_details)
            .await
        {
            Ok(()) => break actual_label, // report the label that worked
            Err((returned_inactive, database::Error::ChannelExists(_))) => {
                inactive = returned_inactive; // restore the inactive state, try again
            }
            Err((_returned_inactive, error)) => {
                // TODO: what to do with the `Inactive` state here when the database has failed to allow us to persist it?
                return Err(error.into());
            }
        }
        count += 1;
    };

    Ok(actual_label)
}

/// The core zkAbacus.Activate protocol.
async fn zkabacus_activate(
    config: &Config,
    database: &dyn QueryCustomer,
    label: &ChannelName,
    chan: Chan<establish::Activate>,
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
            |inactive: Inactive| match inactive.activate(pay_token) {
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
