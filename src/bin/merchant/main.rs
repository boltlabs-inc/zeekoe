use {
    anyhow::Context,
    async_trait::async_trait,
    dialectic::{offer, Session},
    futures::{
        stream::{FuturesUnordered, StreamExt},
        FutureExt,
    },
    rand::{rngs::StdRng, SeedableRng},
    sqlx::SqlitePool,
    std::{convert::identity, sync::Arc},
    structopt::StructOpt,
    tokio::sync::broadcast,
};

use zeekoe::{
    escrow::{
        tezos,
        types::{ContractStatus, TezosKeyMaterial},
    },
    merchant::{
        cli::{self, Run},
        config::{DatabaseLocation, Service},
        database::{connect_sqlite, QueryMerchant},
        defaults::config_path,
        server::SessionKey,
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

/// A single merchant-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Merchant`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, config: Config) -> Result<(), anyhow::Error>;
}

#[async_trait]
impl Command for Run {
    async fn run(self, config: Config) -> Result<(), anyhow::Error> {
        let database = database(&config)
            .await
            .context("Failed to connect to merchant database")?;

        // Either initialize the merchant's config afresh, or get existing config if it exists
        let merchant_config = database
            .fetch_or_create_config(&mut StdRng::from_entropy()) // TODO: allow determinism
            .await?;

        // Share the configuration between all server threads
        let merchant_config = Arc::new(merchant_config);
        let client = reqwest::Client::new();
        let tezos_key_material = TezosKeyMaterial::read_key_pair(config.tezos_account.clone())?;

        // Sender and receiver to indicate graceful shutdown should occur
        let (terminate, _) = broadcast::channel(1);

        // Collect the futures for the result of running each specified server
        let mut server_futures: FuturesUnordered<_> = config
            .services
            .iter()
            .map(|service| {
                // Clone `Arc`s for the various resources we need in this server
                let client = client.clone();
                let merchant_config = merchant_config.clone();
                let database = database.clone();
                let service = Arc::new(service.clone());
                let mut wait_terminate = terminate.subscribe();
                let tezos_key_material = tezos_key_material.clone();

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
                        let merchant_config = merchant_config.clone();
                        let database = database.clone();
                        let service = service.clone();
                        let tezos_key_material = tezos_key_material.clone();

                        // TODO: permit configuration option to make this deterministic for testing
                        let rng = StdRng::from_entropy();

                        async move {
                            offer!(in chan {
                                0 => Parameters.run(
                                    rng,
                                    &client,
                                    tezos_key_material,
                                    &service,
                                    &merchant_config,
                                    database.as_ref(),
                                    session_key,
                                    chan,
                                ).await?,
                                1 => Establish.run(
                                    rng,
                                    &client,
                                    tezos_key_material,
                                    &service,
                                    &merchant_config,
                                    database.as_ref(),
                                    session_key,
                                    chan,
                                ).await?,
                                2 => Pay.run(
                                    rng,
                                    &client,
                                    tezos_key_material,
                                    &service,
                                    &merchant_config,
                                    database.as_ref(),
                                    session_key,
                                    chan,
                                ).await?,
                                3 => Close.run(
                                    rng,
                                    &client,
                                    tezos_key_material,
                                    &service,
                                    &merchant_config,
                                    database.as_ref(),
                                    session_key,
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

        // Wait for each server to finish, and print its error if it did
        loop {
            if server_futures.is_empty() {
                break;
            }

            if let Some(result) = server_futures.next().await {
                if let Err(e) = result {
                    eprintln!("Error: {}", e);
                }
            } else {
                break;
            }
        }

        Ok(())
    }
}

#[allow(unused)]
async fn merchant_poll(
    database: &dyn QueryMerchant,
    tezos_key_material: TezosKeyMaterial,
) -> Result<(), anyhow::Error> {
    let channels = database
        .get_channels()
        .await
        .context("Failed to retrieve channels")?;

    for channel in channels {
        // Retrieve on-chain contract status
        let contract_state = tezos::get_contract_state(&channel.contract_id)
            .await
            .context(format!(
                "Merchant chain watcher failed to retrieve contract state for {}",
                channel.contract_id
            ))?;

        // The channel has not claimed funds after the expiry timeout expired
        // The condition is
        // - the contract is in expiry state
        // - the contract timeout is expired
        // - the local channel status is still waiting for expiry
        if contract_state.status() == ContractStatus::Expiry
            && contract_state.timeout_expired().unwrap_or(false)
            && channel.status == ChannelStatus::PendingExpiry
        {
            close::claim_expiry_funds(database, &channel.channel_id, &tezos_key_material).await?;
        }

        // The channel has not reacted to a customer posting close balances on chain
        // The condition is
        // - the contract is in customer close state
        // - the local channel status is either active (if the customer initiated the close flow)
        //   or pending expiry (if the merchant initiated the close flow)
        if contract_state.status() == ContractStatus::CustomerClose
            && (channel.status == ChannelStatus::Active
                || channel.status == ChannelStatus::PendingExpiry)
        {
            let revocation_lock = contract_state.revocation_lock().ok_or_else(|| {
                anyhow::anyhow!(
                    "Failed to retrieve revocation lock from contract storage for {}",
                    channel.channel_id
                )
            })?;
            let final_balances = contract_state.final_balances().ok_or_else(|| {
                anyhow::anyhow!(
                    "Failed to retrieve final balances from contract storage for {}",
                    channel.channel_id
                )
            })?;
            close::process_customer_close(
                database,
                &tezos_key_material,
                &channel.channel_id,
                revocation_lock,
            )
            .await?;
            close::finalize_customer_close(
                database,
                &channel.channel_id,
                final_balances.customer_balance(),
                final_balances.merchant_balance(),
            )
            .await?;
        }
    }

    Ok(())
}

#[async_trait]
pub trait Method
where
    Self::Protocol: Session,
    <Self::Protocol as Session>::Dual: Session,
{
    type Protocol;

    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        rng: StdRng,
        client: &reqwest::Client,
        tezos_key_material: TezosKeyMaterial,
        config: &Service,
        merchant_config: &zkabacus_crypto::merchant::Config,
        database: &dyn QueryMerchant,
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error>;
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

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
