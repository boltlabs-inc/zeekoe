use std::time::Duration;

use {anyhow::Context, async_trait::async_trait, rand::rngs::StdRng, std::sync::Arc};

use zeekoe::{
    customer::database::zkchannels_state::{self, ZkChannelState},
    customer::{
        cli::Watch,
        database::{ChannelDetails, QueryCustomer},
        Config,
    },
    escrow::{
        tezos,
        types::{ContractStatus, TezosKeyMaterial},
    },
};

use super::{close, database, Command};

#[async_trait]
impl Command for Watch {
    async fn run(self, rng: StdRng, config: Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Customer chain-watching daemon failed to connect to local database")?;

        let config = Arc::new(config);

        // Retrieve Tezos keys from disk
        let customer_key_material = config
            .load_tezos_key_material()
            .await
            .context("Customer chain-watching daemon failed to load Tezos key material")?;

        /*
        // Note: commenting out the server setup because we will not use it with the polling
        // architecture; we don't expect any incoming requests.

        // Sender and receiver to indicate graceful shutdown should occur
        let (terminate, _) = broadcast::channel(1);
        let mut wait_terminate = terminate.subscribe();

        // Initialize a new `Server` with parameters taken from the configuration
        let server: Server<Daemon> = Server::new();

        // Serve on this address
        let localhost_v4 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let address = (localhost_v4, config.daemon_port);

        // There is no meaningful initialization necessary per request
        let initialize = || async { Some(()) };

        // For each request, dispatch to the appropriate method, defined elsewhere
        let interact = move |_session_key, (), _chan: Chan<Daemon>| {
            // Clone `Arc`s for the various resources we need in this request
            //let _database = database.clone();

            async move {
                offer!(in _chan {
                    // Refresh
                    0 => {
                        println!("refreshed");
                        Ok::<_, anyhow::Error>(())
                    }
                })?
            }
        };
        */

        // Set the polling service interval to run every 60 seconds
        let mut interval = tokio::time::interval(Duration::from_secs(60));

        // Run the polling service
        let polling_service_join_handle = tokio::spawn(async move {
            // Clone resources
            let database = database.clone();
            let customer_key_material = customer_key_material.clone();

            loop {
                // Retrieve list of channels from database
                let channels = match database
                    .get_channels()
                    .await
                    .context("Failed to retrieve contract IDs")
                {
                    Ok(channels) => channels,
                    Err(e) => return Err::<(), anyhow::Error>(e),
                };

                // Query each contract ID and dispatch on the result
                for channel in channels {
                    let database = database.clone();
                    let customer_key_material = customer_key_material.clone();
                    let mut rng = rng.clone();
                    let off_chain = self.off_chain;
                    tokio::spawn(async move {
                        dispatch_channel(
                            &mut rng,
                            database.as_ref(),
                            &channel,
                            &customer_key_material,
                            off_chain,
                        )
                        .await
                        .unwrap_or_else(|e| {
                            eprintln!("Error dispatching on {}: {}", &channel.label, e)
                        });
                    });
                }
                interval.tick().await;
            }
        });

        /*
        // Note: We do not run the server in the polling architecture because we do not expect any
        // incoming requests.

        // Future that completes on graceful shutdown
        let wait_terminate = async move { wait_terminate.recv().await.unwrap_or(()) };

        server
            .serve_while(address, None, initialize, interact, wait_terminate)
            .await?;
        */
        polling_service_join_handle.await?
    }
}

async fn dispatch_channel(
    rng: &mut StdRng,
    database: &dyn QueryCustomer,
    channel: &ChannelDetails,
    customer_key_material: &TezosKeyMaterial,
    off_chain: bool,
) -> Result<(), anyhow::Error> {
    // Retrieve on-chain contract status
    let contract_state = match &channel.contract_details.contract_id {
        Some(contract_id) => tezos::get_contract_state(contract_id)
            .await
            .context("Failed to retrieve contract state")?,
        None => return Ok(()),
    };

    // The channel has not reacted to an expiry transaction being posted
    // The condition is
    // - the contract is in Expiry state
    // - the local state is not PendingClose
    if contract_state.status() == ContractStatus::Expiry
        && !zkchannels_state::PendingClose.matches(&channel.state)
    {
        close::unilateral_close(
            &channel.label,
            off_chain,
            rng,
            database,
            customer_key_material,
        )
        .await?;
    }

    // The channel has not claimed funds after custClose timeout expired
    // The condition is:
    // - the contract is in the CustomerClose state
    // - the timeout has been set and expired
    // - the local state is not PendingCustomerClaim (e.g. we did not try to
    //   claim funds yet)
    if contract_state.status() == ContractStatus::CustomerClose
        && contract_state.timeout_expired().unwrap_or(false)
        && !zkchannels_state::PendingCustomerClaim.matches(&channel.state)
    {
        close::claim_funds(database, &channel.label, customer_key_material).await?;
    }

    // The channel has not reacted to a merchDispute transaction being posted
    // The condition is:
    // - the contract is Closed but the local state is still PendingClose
    // - the local merchant balance has been paid out (e.g. we are not in the
    //   expiry flow)
    if contract_state.status() == ContractStatus::Closed
        && zkchannels_state::PendingClose.matches(&channel.state)
        && channel.closing_balances.merchant_balance.is_some()
    {
        close::process_dispute(database, &channel.label).await?;
        close::finalize_dispute(database, &channel.label).await?
    }

    // The channel has not reacted to a merchClaim transaction being posted
    // The condition is:
    // - the contract is Closed but the local state is still PendingClose
    // - the local merchant balance has not been paid out (we are in the expiry flow)
    if contract_state.status() == ContractStatus::Closed
        && zkchannels_state::PendingClose.matches(&channel.state)
        && channel.closing_balances.merchant_balance.is_none()
    {
        close::finalize_expiry(database, &channel.label).await?
    }

    Ok(())
}
