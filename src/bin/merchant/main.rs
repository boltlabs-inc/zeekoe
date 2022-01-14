use {
    anyhow::Context,
    async_trait::async_trait,
    dialectic::offer,
    futures::{
        stream::{FuturesUnordered, StreamExt},
        FutureExt,
    },
    rand::{rngs::StdRng, SeedableRng},
    sqlx::SqlitePool,
    std::{convert::identity, sync::Arc},
    structopt::StructOpt,
    tokio::signal,
    tokio::sync::broadcast,
};

use std::time::Duration;
use tracing::{error, info};

use zeekoe::{
    escrow::{
        tezos::TezosClient,
        types::{ContractStatus, TezosKeyMaterial},
    },
    merchant::{
        cli::{self, Run},
        config::DatabaseLocation,
        database::{connect_sqlite, ChannelDetails, QueryMerchant},
        defaults::config_path,
        Chan, Cli, Config, Server,
    },
    protocol::{ChannelStatus, ZkChannels},
};

mod approve;
mod close;
mod establish;
mod manage;
mod parameters;
mod pay;

use close::Close;
use establish::Establish;
use parameters::Parameters;
use pay::Pay;
use zkabacus_crypto::ChannelId;

const MAX_INTERVAL_SECONDS: u64 = 60;

/// A single merchant-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`cli::Merchant`] should implement this, except [`cli::Merchant::Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, config: Config) -> Result<(), anyhow::Error>;
}

#[async_trait]
impl Command for Run {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        // Either initialize the merchant's config afresh, or get existing config if it exists
        let zkabacus_config = database(&config)
            .await
            .context("Failed to connect to merchant database")?
            .fetch_or_create_config(&mut StdRng::from_entropy()) // TODO: allow determinism
            .await
            .context("Failed to create or retrieve cryptography configuration")?;

        // Share the configuration between all server threads
        let zkabacus_config = Arc::new(zkabacus_config);
        let client = reqwest::Client::new();
        let config = config.clone();

        // Sender and receiver to indicate graceful shutdown should occur
        let (terminate, _) = broadcast::channel(1);

        // Collect the futures for the result of running each specified server
        let mut server_futures: FuturesUnordered<_> = config
            .services
            .iter()
            .map(|service| {
                // Clone `Arc`s for the various resources we need in this server
                let client = client.clone();
                let config = config.clone();
                let zkabacus_config = zkabacus_config.clone();
                let service = Arc::new(service.clone());
                let mut wait_terminate = terminate.subscribe();

                async move {
                    // Initialize a new `Server` with parameters taken from the configuration
                    let mut server: Server<ZkChannels> = Server::new();
                    server
                        .timeout(service.connection_timeout)
                        .max_pending_retries(Some(service.max_pending_connection_retries))
                        .max_length(service.max_message_length);

                    // Serve on this address
                    let address = (service.address, service.port);
                    let certificate = service.certificate.clone();
                    let private_key = service.private_key.clone();

                    // There is no meaningful initialization necessary per request
                    let initialize = || async { Some(()) };

                    // For each request, dispatch to the appropriate method, defined elsewhere
                    let interact = move |session_key, (), chan: Chan<ZkChannels>| {
                        // Clone `Arc`s for the various resources we need in this request
                        let client = client.clone();
                        let zkabacus_config = zkabacus_config.clone();
                        let service = service.clone();
                        let config = config.clone();

                        // TODO: permit configuration option to make this deterministic for testing
                        let rng = StdRng::from_entropy();

                        async move {
                            offer!(in chan {
                                0 => Parameters.run(
                                    &config,
                                    &zkabacus_config,
                                    chan,
                                ).await?,
                                1 => Establish.run(
                                    rng,
                                    &client,
                                    &config,
                                    &service,
                                    &zkabacus_config,
                                    session_key,
                                    chan,
                                ).await?,
                                2 => Pay.run(
                                    rng,
                                    &client,
                                    &config,
                                    &service,
                                    session_key,
                                    chan,
                                ).await?,
                                3 => Close.run(
                                    &config,
                                    &service,
                                    &zkabacus_config,
                                    chan,
                                ).await?,

                            })?;
                            Ok::<_, anyhow::Error>(())
                        }
                    };

                    // Future that completes on graceful shutdown
                    let wait_terminate = async move { wait_terminate.recv().await.unwrap_or(()) };

                    // Run the server until graceful shutdown
                    server
                        .serve_while(
                            address,
                            Some((&certificate, &private_key)),
                            initialize,
                            interact,
                            wait_terminate,
                        )
                        .await?;
                    Ok::<_, anyhow::Error>(())
                }
            })
            .collect();

        // In production, the self_delay should be long (at least 48h) so this will always end up
        // being 60s. In development, you may see lower values to allow for quicker testing.
        let interval_seconds = std::cmp::min(config.self_delay / 2, MAX_INTERVAL_SECONDS);
        let mut polling_interval = tokio::time::interval(Duration::from_secs(interval_seconds));

        // Get a join handle for the polling service
        let polling_service_join_handle = tokio::spawn(async move {
            // Clone resources
            let config = config.clone();
            let database = database(&config).await?;

            loop {
                // Retrieve list of channels from database
                let channels = match database
                    .get_channels()
                    .await
                    .context("Merchant chain watcher failed to retrieve contract IDs")
                {
                    Ok(channels) => channels,
                    Err(e) => return Err::<(), anyhow::Error>(e),
                };

                // Query each contract ID and dispatch on the result
                for channel in channels {
                    let database = database.clone();
                    let config = config.clone();
                    tokio::spawn(async move {
                        match dispatch_channel(database.as_ref(), &channel, &config).await {
                            Ok(()) => info!("Successfully dispatched {}", &channel.channel_id),
                            Err(e) => {
                                error!("Error dispatching on {}: {}", &channel.channel_id, e)
                            }
                        }
                    });
                }
                polling_interval.tick().await;
            }
        });

        // Wait for either the servers or the polling service to finish
        tokio::select! {
            _ = signal::ctrl_c() => info!("Terminated by user"),
            Some(Err(e)) = server_futures.next() => {
                error!("Error: {}", e);
            },
            Err(e) = polling_service_join_handle => {
                error!("Error: {}", e);
            }
            else => {
                info!("Shutting down...")
            }
        }

        Ok(())
    }
}

async fn dispatch_channel(
    database: &dyn QueryMerchant,
    channel: &ChannelDetails,
    config: &Config,
) -> Result<(), anyhow::Error> {
    let tezos_client = load_tezos_client(config, &channel.channel_id, database).await?;
    let contract_state = tezos_client.get_contract_state().await?;

    // The channel has not claimed funds after the expiry timeout expired
    // The condition is
    // - the contract is in expiry state
    // - the contract timeout is expired
    // - the channel status is PendingExpiry, indicating it has not yet claimed funds
    if contract_state.status()? == ContractStatus::Expiry
        && contract_state.timeout_expired().unwrap_or(false)
        && channel.status == ChannelStatus::PendingExpiry
    {
        close::claim_expiry_funds(config, database, &channel.channel_id).await?;
        close::finalize_expiry_close(database, &channel.channel_id).await?;
    }

    // The channel has not reacted to a customer posting close balances on chain
    // The condition is
    // - the contract is in customer close state
    // - the channel status indicates a funded channel that has not already entered a close flow:
    //   MerchantFunded, if the the customer didn't receive a valid pay token in activate
    //   Active, if the customer initiated the close flow on a channel without any error
    //   PendingExpiry, if the merchant initiated the close flow on a channel
    //   PendingMutualClose, if the customer posted a unilateral close operation instead of the
    //     agreed-upon mutual close transaction
    if contract_state.status()? == ContractStatus::CustomerClose
        && (channel.status == ChannelStatus::MerchantFunded
            || channel.status == ChannelStatus::Active
            || channel.status == ChannelStatus::PendingExpiry
            || channel.status == ChannelStatus::PendingMutualClose)
    {
        let revocation_lock = contract_state.revocation_lock()?.ok_or_else(|| {
            anyhow::anyhow!(
                "Failed to retrieve revocation lock from contract storage for {}",
                channel.channel_id
            )
        })?;
        let final_balances = contract_state.final_balances()?.ok_or_else(|| {
            anyhow::anyhow!(
                "Failed to retrieve final balances from contract storage for {}",
                channel.channel_id
            )
        })?;
        close::process_customer_close(config, database, &channel.channel_id, &revocation_lock)
            .await?;
        close::finalize_customer_close(
            database,
            &channel.channel_id,
            final_balances.customer_balance(),
            final_balances.merchant_balance(),
        )
        .await?;
    }

    // The channel has not reacted to a customer posting a mutual close transaction on chain
    // The condition is
    // - the contract is closed, but
    // - the channel status is `PendingMutualClose`
    if contract_state.status()? == ContractStatus::Closed
        && channel.status == ChannelStatus::PendingMutualClose
    {
        // Update the database to indicate a successful mutual close
        close::finalize_mutual_close(
            database,
            &channel.channel_id,
        )
        .await
        .context(
            "Failed to finalize mutual close - perhaps the contract was closed by a different flow",
        )?;
    }

    Ok(())
}

pub async fn main_with_cli(cli: Cli) -> Result<(), anyhow::Error> {
    let config_path = cli.config.ok_or_else(config_path).or_else(identity)?;
    let config = Config::load(&config_path).map(|result| {
        result.with_context(|| {
            format!(
                "Could not load merchant configuration from {:?}",
                config_path
            )
        })
    });

    use cli::Merchant::*;
    match cli.merchant {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        List(list) => list.run(config.await?).await,
        Show(show) => show.run(config.await?).await,
        Run(run) => run.run(config.await?).await,
        Close(close) => close.run(config.await?).await,
    }
}

/// Connect to the database specified by the configuration.
pub async fn database(config: &Config) -> Result<Arc<dyn QueryMerchant>, anyhow::Error> {
    let database = match config.database {
        DatabaseLocation::Ephemeral => Arc::new(
            SqlitePool::connect("file::memory:")
                .await
                .context("Could not create in-memory SQLite database")?,
        ),
        DatabaseLocation::Sqlite(ref path) => {
            let conn = connect_sqlite(path).await?;
            conn.migrate().await?;
            conn
        }
        DatabaseLocation::Postgres(_) => {
            return Err(anyhow::anyhow!(
                "Postgres database support is not yet implemented"
            ))
        }
    };
    Ok(database)
}

pub async fn load_tezos_client(
    config: &Config,
    channel_id: &ChannelId,
    database: &dyn QueryMerchant,
) -> Result<TezosClient, anyhow::Error> {
    let contract_id = database.contract_details(channel_id).await?;

    Ok(TezosClient {
        uri: Some(config.tezos_uri.clone()),
        contract_id,
        client_key_pair: TezosKeyMaterial::read_key_pair(&config.tezos_account)?,
        confirmation_depth: config.confirmation_depth,
        self_delay: config.self_delay,
    })
}

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
