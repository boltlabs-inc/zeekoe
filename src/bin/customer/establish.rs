use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng, std::convert::TryInto};

use zkabacus_crypto::{
    customer::{Inactive, Requested},
    ChannelId, Context as ProofContext, CustomerBalance, CustomerRandomness, MerchantBalance,
    PayToken,
};

use zeekoe::{
    abort,
    customer::{
        cli::Establish,
        client::ZkChannelAddress,
        database::{take_state, QueryCustomer, QueryCustomerExt, State},
        Chan, ChannelName, Config,
    },
    offer_abort, proceed,
    protocol::{
        establish,
        Party::{Customer, Merchant},
    },
};

use super::{connect, database, Command};

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

        // Run a **separate** session to get the merchant's public parameters
        let customer_config = get_parameters(&config, &address).await?;

        // Connect and select the Establish session
        let (session_key, chan) = connect(&config, &address)
            .await
            .context("Failed to connect to merchant")?;
        let chan = chan
            .choose::<1>()
            .await
            .context("Failed to select channel establishment session")?;

        // TODO: send customer chain-specific things

        let customer_randomness = CustomerRandomness::new(&mut rng);
        let chan = chan
            .send(customer_randomness)
            .await
            .context("Failed to send customer randomness for channel ID")?;

        // Format deposit amounts as the correct types
        let customer_deposit = CustomerBalance::try_new(
            self.deposit
                .as_minor_units()
                .ok_or(establish::Error::InvalidDeposit(Customer))?
                .try_into()?,
        )
        .map_err(|_| establish::Error::InvalidDeposit(Customer))?;

        let merchant_deposit: MerchantBalance =
            MerchantBalance::try_new(match self.merchant_deposit {
                None => 0,
                Some(d) => d
                    .as_minor_units()
                    .ok_or(establish::Error::InvalidDeposit(Merchant))?
                    .try_into()?,
            })
            .map_err(|_| establish::Error::InvalidDeposit(Merchant))?;

        // Read the contents of the note, if any
        let note = self
            .note
            .unwrap_or_else(|| zeekoe::customer::cli::Note::String(String::from("")))
            .read(config.max_note_length)?;

        // Send the request for the funding of the channel
        let chan = chan
            .send(customer_deposit)
            .await
            .context("Failed to send customer deposit amount")?
            .send(merchant_deposit)
            .await
            .context("Failed to send merchant deposit amount")?
            .send(note)
            .await
            .context("Failed to send channel establishment note")?;

        // Allow the merchant to reject the funding of the channel, else continue
        offer_abort!(in chan as Customer);

        // TODO: receive merchant account info

        let (merchant_randomness, chan) = chan
            .recv()
            .await
            .context("Failed to receive merchant randomness for channel ID")?;

        // Generate a channel ID (the merchant will share this)
        let channel_id = ChannelId::new(
            merchant_randomness,
            customer_randomness,
            customer_config.merchant_public_key(),
            &[], // TODO: fill this in with bytes from merchant account info
            &[], // TODO: fill this in with bytes from customer account info
        );

        // Generate the proof context for the pay proof
        let context = ProofContext::new(&session_key.to_bytes());

        // Use the specified label, or else use the `ZkChannelAddress` as a string
        let label = label.unwrap_or_else(|| ChannelName::new(format!("{}", address)));

        // Run zkAbacus.Initialize
        let (actual_label, chan) = zkabacus_initialize(
            &mut rng,
            database.as_ref(),
            customer_config,
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

        // Allow the merchant to indicate whether it funded the channel
        offer_abort!(in chan as Customer);

        // TODO: initialize contract on-chain via escrow agent.
        // TODO: fund contract via escrow agent
        // TODO: send contract id to merchant

        // TODO: check that merchant funding was successful: if not, recommend unilateral close
        let merchant_funding_successful: bool = true; // TODO: query tezos for merchant funding

        if merchant_funding_successful {
            abort!(in chan return establish::Error::FailedMerchantFunding);
        }
        proceed!(in chan);

        // Run zkAbacus.Activate
        zkabacus_activate(database.as_ref(), actual_label, chan)
            .await
            .context("Failed to activate channel")?;

        Ok(())
    }
}

/// Fetch the merchant's public parameters.
async fn get_parameters(
    config: &Config,
    address: &ZkChannelAddress,
) -> Result<zkabacus_crypto::customer::Config, anyhow::Error> {
    todo!("Fill in with get-parameters session from start to finish")
}

/// The core zkAbacus.Initialize protocol.
///
/// If successful returns the [`ChannelName`] that the channel was *actually* inserted into the
/// database using (which may differ from the one specified if the one specified was already in
/// use!), and the [`Chan`] ready for the next part of the establish protocol.
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

    // Send the pay proof
    let chan = chan.send(proof).await.context("Failed to send pay proof")?;

    // Allow the merchant to reject the pay proof
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
) -> Result<ChannelName, sqlx::Error> {
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
            Err((returned_inactive, Ok(_))) => {
                inactive = returned_inactive; // restore the inactive state, try again
            }
            Err((_returned_inactive, Err(error))) => {
                // TODO: what to do with the `Inactive` state here when the database has failed to allow us to persist it?
                return Err(error);
            }
        }
        count += 1;
    };

    Ok(actual_label)
}

/// The core zkAbacus.Initialize protocol.
async fn zkabacus_activate(
    database: &dyn QueryCustomer,
    label: ChannelName,
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
    activate_local(database, &label, pay_token).await
}

/// Update the local state for a channel from [`Inactive`] to [`Ready`] in the database.
async fn activate_local(
    database: &dyn QueryCustomer,
    label: &ChannelName,
    pay_token: PayToken,
) -> Result<(), anyhow::Error> {
    database
        .with_channel_state(label, |state| {
            let inactive = take_state(State::inactive, state)?;
            *state = Some(State::Ready(
                inactive
                    .activate(pay_token)
                    .map_err(|_| establish::Error::InvalidPayToken)?,
            ));
            Ok(())
        })
        .await??
}
