use {
    async_trait::async_trait,
    dialectic::{offer, Session},
    futures::stream::{FuturesUnordered, StreamExt},
    rand::{rngs::StdRng, SeedableRng},
    sqlx::SqlitePool,
    std::{
        convert::identity,
        io,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
    },
    structopt::StructOpt,
};

use zeekoe::{
    merchant::{
        cli::{self, Run},
        config::{DatabaseLocation, Service},
        database::QueryMerchant,
        defaults::config_path,
        server::SessionKey,
        Chan, Cli, Config, Server,
    },
    protocol::ZkChannels,
};

mod parameters;
mod pay;

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
        let database: Arc<dyn QueryMerchant + Send + Sync> = match config.database {
            DatabaseLocation::InMemory => Arc::new(SqlitePool::connect("file::memory:").await?),
            DatabaseLocation::Sqlite(uri) => Arc::new(SqlitePool::connect(&uri).await?),
            DatabaseLocation::Postgres(_) => {
                return Err(anyhow::anyhow!(
                    "Postgres database support is not yet implemented"
                ))
            }
        };

        let merchant_config: zkabacus_crypto::merchant::Config =
            todo!("fetch merchant config from database");

        // Share the configuration between all server threads
        let config = Arc::new(config);
        let merchant_config = Arc::new(merchant_config);
        let client = reqwest::Client::new();

        // Flag to indicate graceful shutdown should occur
        let running = Arc::new(AtomicBool::new(true));

        // Collect the futures for the result of running each specified server
        let mut server_futures: FuturesUnordered<_> = config
            .services
            .iter()
            .map(|service| {
                // Clone `Arc`s for the various resources we need in this server
                let config = config.clone();
                let client = client.clone();
                let merchant_config = merchant_config.clone();
                let database = database.clone();
                let running = running.clone();
                let approve = Arc::new(service.approve.clone());
                let service = Arc::new(service.clone());

                async move {
                    // Initialize a new `Server` with parameters taken from the configuration
                    let mut server: Server<ZkChannels> =
                        Server::new(&service.certificate, &service.private_key)?;
                    server
                        .timeout(service.connection_timeout)
                        .max_pending_retries(Some(service.max_pending_connection_retries))
                        .max_length(service.max_message_length);

                    // Serve on this address
                    let address = (service.address, service.port);

                    // Every request, check to see if the graceful shutdown has been issued, and
                    // stop accepting new requests, if so
                    let initialize = || async {
                        if running.load(Ordering::Relaxed) {
                            Some(())
                        } else {
                            None
                        }
                    };

                    // For each request, dispatch to the appropriate method, defined elsewhere
                    let interact = move |session_key, (), chan: Chan<ZkChannels>| {
                        // Clone `Arc`s for the various resources we need in this request
                        let config = config.clone();
                        let client = client.clone();
                        let merchant_config = merchant_config.clone();
                        let database = database.clone();
                        let approve = approve.clone();
                        let service = service.clone();

                        // TODO: permit configuration option to make this deterministic for testing
                        let rng = StdRng::from_entropy();

                        async move {
                            offer!(in chan {
                                0 => Parameters.run(rng, &client, &service, &merchant_config, database.as_ref(), session_key, chan).await?,
                                1 => {
                                    let pay = Pay { approve: approve.clone() };
                                    pay.run(rng, &client, &service, &merchant_config, database.as_ref(), session_key, chan).await?
                                },
                            })?;
                            Ok::<_, anyhow::Error>(())
                        }
                    };

                    // Run the server until the graceful shutdown is issued
                    server.serve_while(address, initialize, interact).await?;
                    Ok::<_, anyhow::Error>(())
                }
            })
            .collect();

        // Task to await ^C and shut down the server gracefully if it is received
        tokio::spawn(async move {
            tokio::signal::ctrl_c().await?;
            running.store(false, Ordering::Relaxed);
            eprintln!("Shutting down (waiting for all open sessions to complete) ...");
            Ok::<_, io::Error>(())
        });

        // Wait for each server to finish, and print its error if it did
        while let Some(result) = server_futures.next().await {
            if let Err(e) = result {
                eprintln!("Error: {}", e);
            }
        }

        Ok(())
    }
}

#[async_trait]
pub trait Method
where
    Self::Protocol: Session,
    <Self::Protocol as Session>::Dual: Session,
{
    type Protocol;

    async fn run(
        &self,
        rng: StdRng,
        client: &reqwest::Client,
        config: &Service,
        merchant_config: &zkabacus_crypto::merchant::Config,
        database: &(dyn QueryMerchant + Send + Sync),
        session_key: SessionKey,
        chan: Chan<Self::Protocol>,
    ) -> Result<(), anyhow::Error>;
}

pub async fn main_with_cli(cli: Cli) -> Result<(), anyhow::Error> {
    let config_path = cli.config.ok_or_else(config_path).or_else(identity)?;
    let config = Config::load(&config_path);

    use cli::Merchant::*;
    match cli.merchant {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        Run(run) => run.run(config.await?).await,
    }
}

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
