use {
    anyhow::Context,
    async_trait::async_trait,
    futures::FutureExt,
    rand::{rngs::StdRng, SeedableRng},
    sqlx::SqlitePool,
    std::{convert::identity, sync::Arc, time::Duration},
    structopt::StructOpt,
    webpki::DNSNameRef,
};

use zeekoe::{
    customer::{
        cli::{self, Customer::*},
        client::{Backoff, SessionKey, ZkChannelAddress},
        database::{connect_sqlite, QueryCustomer},
        defaults::config_path,
        Chan, Cli, Client, Config,
    },
    escrow::tezos,
    protocol,
};

pub(crate) mod close;
mod establish;
mod manage;
mod pay;
mod watch;

/// A single customer-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Customer`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    /// Run the command to completion using the given random number generator for all randomness and
    /// the given customer configuration.
    async fn run(self, rng: StdRng, config: Config) -> Result<(), anyhow::Error>;
}

pub async fn main_with_cli(cli: Cli) -> Result<(), anyhow::Error> {
    let config_path = cli.config.ok_or_else(config_path).or_else(identity)?;
    let config = Config::load(&config_path).map(|result| {
        result.with_context(|| {
            format!(
                "Could not load customer configuration from {:?}",
                config_path
            )
        })
    });

    // TODO: let this be made deterministic during testing
    let rng = StdRng::from_entropy();

    match cli.customer {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        List(list) => list.run(rng, config.await?).await,
        // Show(show) => show.run(rng, config.await?).await,
        Rename(rename) => rename.run(rng, config.await?).await,
        Establish(establish) => establish.run(rng, config.await?).await,
        Pay(pay) => pay.run(rng, config.await?).await,
        Refund(refund) => refund.run(rng, config.await?).await,
        Close(close) => close.run(rng, config.await?).await,
        Run(run) => run.run(rng, config.await?).await,
    }
}

/// Connect to a given [`ZkChannelAddress`], configured using the parameters in the [`Config`].
pub async fn connect(
    config: &Config,
    address: &ZkChannelAddress,
) -> Result<(SessionKey, Chan<protocol::ZkChannels>), anyhow::Error> {
    let Config {
        backoff,
        connection_timeout,
        max_pending_connection_retries,
        max_message_length,
        trust_certificate,
        ..
    } = config;

    let mut client: Client<protocol::ZkChannels> = Client::new(*backoff);
    client
        .max_length(*max_message_length)
        .timeout(*connection_timeout)
        .max_pending_retries(*max_pending_connection_retries);

    if let Some(path) = trust_certificate {
        #[cfg(feature = "allow_explicit_certificate_trust")]
        client.trust_explicit_certificate(path).with_context(|| {
            format!(
                "Failed to enable explicitly trusted certificate at {:?}",
                path
            )
        })?;

        #[cfg(not(feature = "allow_explicit_certificate_trust"))]
        eprintln!(
            "Ignoring explicitly trusted certificate at {:?} because \
            this binary was built to only trust webpki roots of trust",
            path
        );
    }

    Ok(client.connect_zkchannel(address).await?)
}

pub async fn connect_daemon(
    config: &Config,
) -> anyhow::Result<(SessionKey, Chan<protocol::daemon::Daemon>)> {
    // Always error immediately. We don't need retry/reconnect for the daemon.
    let mut backoff = Backoff::with_delay(Duration::ZERO);
    backoff.max_retries(0);

    let address = DNSNameRef::try_from_ascii_str("localhost").unwrap();
    let client: Client<protocol::daemon::Daemon> = Client::new(backoff);
    Ok(client.connect(&address.into(), config.daemon_port).await?)
}

/// Connect to the database specified by the configuration.
pub async fn database(config: &Config) -> Result<Arc<dyn QueryCustomer>, anyhow::Error> {
    let location = match config.database.clone() {
        None => zeekoe::customer::defaults::database_location()?,
        Some(l) => l,
    };

    use zeekoe::customer::config::DatabaseLocation;
    let database = match location {
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
    tezos::get_contract_test().await;
    main_with_cli(Cli::from_args()).await
}
