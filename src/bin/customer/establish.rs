use {
    anyhow::Context,
    async_trait::async_trait,
    rand::rngs::StdRng,
    serde::Serialize,
    std::{convert::TryInto, fs::File, path::PathBuf},
};

use zkabacus_crypto::{
    customer::{Inactive, Requested},
    ChannelId, Context as ProofContext, CustomerBalance, CustomerRandomness, MerchantBalance,
    PayToken, PublicKey, CLOSE_SCALAR,
};

use zeekoe::{
    abort,
    customer::{
        cli::Establish,
        client::ZkChannelAddress,
        database::{self, QueryCustomer, QueryCustomerExt, State, StateName},
        Chan, ChannelName, Config,
    },
    offer_abort, proceed,
    protocol::{
        establish,
        Party::{Customer, Merchant},
    },
};

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

        // Format deposit amounts as the correct types
        let customer_deposit = CustomerBalance::try_new(
            self.deposit
                .try_into_minor_units()
                .ok_or(establish::Error::InvalidDeposit(Customer))?
                .try_into()?,
        )
        .map_err(|_| establish::Error::InvalidDeposit(Customer))?;

        let merchant_deposit = MerchantBalance::try_new(match self.merchant_deposit {
            None => 0,
            Some(d) => d
                .try_into_minor_units()
                .ok_or(establish::Error::InvalidDeposit(Merchant))?
                .try_into()?,
        })
        .map_err(|_| establish::Error::InvalidDeposit(Merchant))?;

        // Connect to the customer database
        let database = database(&config)
            .await
            .context("Failed to connect to local database")?;

        // Run a **separate** session to get the merchant's public parameters
        let zkabacus_customer_config = get_parameters(&config, &address).await?;

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

        // Generate and send the customer randomness to the merchant
        let customer_randomness = CustomerRandomness::new(&mut rng);

        // Send the request for the funding of the channel
        let chan = chan
            .send(customer_randomness)
            .await
            .context("Failed to send customer randomness for channel ID")?
            .send(customer_deposit)
            .await
            .context("Failed to send customer deposit amount")?
            .send(merchant_deposit)
            .await
            .context("Failed to send merchant deposit amount")?
            .send(note)
            .await
            .context("Failed to send channel establishment note")?;

        // TODO: customer sends merchant:
        // - customer's tezos public key (eddsa public key)
        // - customer's tezos account tz1 address corresponding to that public key
        // - SHA3-256 of:
        //   * merchant's pointcheval-sanders public key (`zkabacus_crypto::PublicKey`)
        //   * tz1 address corresponding to merchant's public key
        //   * merchant's tezos public key

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
            &[], // TODO: fill this in with bytes of merchant's tezos public key
            &[], // TODO: fill this in with bytes of customer's tezos public key
        );

        // Generate structure holding information about the establishment that's about to take place
        let establishment = Establishment {
            merchant_ps_public_key: zkabacus_customer_config.merchant_public_key().clone(),
            customer_deposit,
            merchant_deposit,
            channel_id,
            close_scalar_bytes: CLOSE_SCALAR.to_bytes(),
        };

        // Generate the proof context for the establish proof
        // TODO: the context should actually be formed from a session transcript up to this point
        let context = ProofContext::new(&session_key.to_bytes());

        // Use the specified label, or else use the `ZkChannelAddress` as a string
        let label = label.unwrap_or_else(|| ChannelName::new(format!("{}", address)));

        // Run zkAbacus.Initialize
        let (actual_label, chan) = zkabacus_initialize(
            &mut rng,
            database.as_ref(),
            zkabacus_customer_config,
            label,
            &address,
            channel_id,
            context,
            merchant_deposit,
            customer_deposit,
            chan,
        )
        .await
        .context("Failed to initialize the channel")?;

        if !self.off_chain {
            // TODO: initialize contract on-chain via escrow agent (this should return a stream of
            // updates to the contract)

            // TODO: fund contract via escrow agent
        }

        // TODO: send contract id to merchant (possibly also send block height, check spec)

        // Allow the merchant to indicate whether it funded the channel
        offer_abort!(in chan as Customer);

        if !self.off_chain {
            // TODO: if merchant contribution was non-zero, check that merchant funding was provided
            // within a configurable timeout and to the desired block depth and that the status of the
            // contract is locked: if not, recommend unilateral close
        }

        let merchant_funding_successful: bool = true; // TODO: query tezos for merchant funding

        if !merchant_funding_successful {
            abort!(in chan return establish::Error::FailedMerchantFunding);
        }
        proceed!(in chan);

        if self.off_chain {
            // Write the establishment information to disk
            write_establish_json(&establishment)?;
        }

        // Run zkAbacus.Activate
        zkabacus_activate(database.as_ref(), &actual_label, chan)
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
) -> Result<zkabacus_crypto::customer::Config, anyhow::Error> {
    // Connect to the merchant
    let (_session_key, chan) = connect(config, address).await?;

    // Select the get-parameters session
    let chan = chan.choose::<0>().await?;

    // Get the merchant's Pointcheval-Sanders public key
    let (merchant_public_key, chan) = chan
        .recv()
        .await
        .context("Failed to receive merchant's Pointcheval-Sanders public key")?;

    // Get the merchant's commitment parameters (TODO: these should be a global default)
    let (revocation_commitment_parameters, chan) = chan
        .recv()
        .await
        .context("Failed to receive merchant's revocation commitment parameters")?;

    // Get the merchant's range proof parameters
    let (range_proof_parameters, chan) = chan
        .recv()
        .await
        .context("Failed to receive merchant's range proof parameters")?;

    // TODO: get the merchant's tz1 address

    // TODO: get the merchant's tezos public key

    chan.close();

    // TODO: ensure that:
    // - merchant's public key (in the config) is a valid Pointcheval-Sanders public key
    // - merchant's range proof parameters consist of valid Pointcheval-Sanders public key and
    //   valid signatures on the correct range
    // - merchant's commitment parameters are "the right ones" (this check can't currently be
    //   done because the parameters are randomly generated at first merchant startup)
    // - merchant's tezos public key is valid
    // - merchant's tezos public key corresponds to the tezos account that they specified
    // - that address is actually a tz1 address

    Ok(zkabacus_crypto::customer::Config::from_parts(
        merchant_public_key,
        revocation_commitment_parameters,
        range_proof_parameters,
    ))
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
    customer_config: zkabacus_crypto::customer::Config,
    label: ChannelName,
    address: &ZkChannelAddress,
    channel_id: ChannelId,
    context: ProofContext,
    merchant_deposit: MerchantBalance,
    customer_deposit: CustomerBalance,
    chan: Chan<establish::Initialize>,
) -> Result<(ChannelName, Chan<establish::CustomerSupplyContractInfo>), anyhow::Error> {
    let (requested, proof) = Requested::new(
        &mut rng,
        customer_config,
        channel_id,
        merchant_deposit,
        customer_deposit,
        &context,
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
    let actual_label = store_inactive_local(database, label, &address, inactive)
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
            .new_channel(&actual_label, &address, inactive)
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

/// The core zkAbacus.Initialize protocol.
async fn zkabacus_activate(
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

    // Step the local channel state forward to `Ready`
    activate_local(database, label, pay_token).await
}

/// Update the local state for a channel from [`Inactive`] to [`Ready`] in the database.
async fn activate_local(
    database: &dyn QueryCustomer,
    label: &ChannelName,
    pay_token: PayToken,
) -> Result<(), anyhow::Error> {
    database
        .with_channel_state(label, StateName::Inactive, |inactive: Inactive| {
            let ready = inactive
                .activate(pay_token)
                .map_err(|_| establish::Error::InvalidPayToken)?;
            Ok((State::Ready(ready), ()))
        })
        .await?
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
