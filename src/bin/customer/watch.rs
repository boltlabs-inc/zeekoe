use std::time::Duration;

use {
    anyhow::Context, async_trait::async_trait, rand::rngs::StdRng, std::sync::Arc, tokio::signal,
};

use zeekoe::{
    customer::database::zkchannels_state::{self, ZkChannelState},
    customer::{
        cli::Watch,
        database::{ChannelDetails, QueryCustomer},
        Config,
    },
    escrow::types::ContractStatus,
};

use super::{close, database, load_tezos_client, Command, TezosClientError};

#[async_trait]
impl Command for Watch {
    async fn run(self, rng: StdRng, config: Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Customer chain-watching daemon failed to connect to local database")?;

        let config = Arc::new(config);

        // Make sure Tezos keys are accessible from disk
        let key_load_test = config.load_tezos_key_material()?;
        drop(key_load_test);

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

        // Set the polling service interval to run every 60 seconds or every self-delay interval
        let interval_seconds = std::cmp::min(60, config.self_delay - 1);
        let mut interval = tokio::time::interval(Duration::from_secs(interval_seconds));

        // Run the polling service
        let polling_service_join_handle = tokio::spawn(async move {
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
                    let config = config.clone();
                    let mut rng = rng.clone();
                    let off_chain = self.off_chain;
                    tokio::spawn(async move {
                        match dispatch_channel(
                            &mut rng,
                            &config,
                            database.as_ref(),
                            &channel,
                            off_chain,
                        )
                        .await
                        {
                            Ok(()) => eprintln!("Successfully dispatched {}", &channel.label),
                            Err(e) => eprintln!("Error dispatching on {}: {}", &channel.label, e),
                        }
                    });
                }
                interval.tick().await;
            }
        });

        tokio::select! {
            _ = signal::ctrl_c() => {
                eprintln!("Terminated by user");
                Ok(())
            },
            result = polling_service_join_handle => result?,
        }

        /*
        // Note: We do not run the server in the polling architecture because we do not expect any
        // incoming requests.

        // Future that completes on graceful shutdown
        let wait_terminate = async move { wait_terminate.recv().await.unwrap_or(()) };

        server
            .serve_while(address, None, initialize, interact, wait_terminate)
            .await?;
        */
    }
}

async fn dispatch_channel(
    rng: &mut StdRng,
    config: &Config,
    database: &dyn QueryCustomer,
    channel: &ChannelDetails,
    off_chain: bool,
) -> Result<(), anyhow::Error> {
    let tezos_client = match load_tezos_client(config, &channel.label, database).await {
        Ok(tezos_client) => tezos_client,
        Err(TezosClientError::ContractDetailsNotSet(_)) => return Ok(()),
        error => error?,
    };
    let contract_state = tezos_client.get_contract_state().await?;

    // The channel has not reacted to an expiry transaction being posted
    // The condition is
    // - the contract is in Expiry state
    // - the local state is neither PendingClose nor PendingExpiry
    if contract_state.status()? == ContractStatus::Expiry
        && !(zkchannels_state::PendingClose.matches(&channel.state)
            || zkchannels_state::PendingExpiry.matches(&channel.state))
    {
        // TODO: this should wait for any payments to complete.

        close::unilateral_close(&channel.label, config, off_chain, rng, database)
            .await
            .context("Chain watcher failed to process contract in expiry state")?;
    }

    // The channel has not claimed funds after custClose timeout expired
    // The condition is:
    // - the contract is in the CustomerClose state
    // - the timeout has been set and expired
    // - the local state is PendingClose (customer did not yet try to claim funds)
    if contract_state.status()? == ContractStatus::CustomerClose
        && contract_state.timeout_expired().unwrap_or(false)
        && zkchannels_state::PendingClose.matches(&channel.state)
    {
        close::claim_funds(database, config, &channel.label)
            .await
            .context("Chain watcher failed to claim funds")?;
        close::finalize_customer_claim(database, &channel.label)
            .await
            .context("Chain watcher failed to finalized claimed funds")?;
    }

    // The channel has not reacted to a merchDispute transaction being posted
    // The condition is:
    // - the contract is Closed
    // - the local state is PendingClose
    if contract_state.status()? == ContractStatus::Closed
        && zkchannels_state::PendingClose.matches(&channel.state)
    {
        close::process_dispute(database, &channel.label)
            .await
            .context("Chain watcher failed to process disputed contract")?;
        close::finalize_dispute(database, &channel.label)
            .await
            .context("Chain watcher failed to process finalized disputed contract")?;
    }

    // The channel has not reacted to a merchClaim transaction being posted
    // The condition is:
    // - the contract is Closed
    // - the local state is PendingExpiry (the customer did not post corrected balances after
    //   the merchant posted expiry)
    if contract_state.status()? == ContractStatus::Closed
        && zkchannels_state::PendingExpiry.matches(&channel.state)
    {
        close::finalize_expiry(database, &channel.label)
            .await
            .context("Chain watcher failed to process expired contract")?;
    }

    Ok(())
}
