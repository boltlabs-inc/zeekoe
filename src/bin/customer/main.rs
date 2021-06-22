use {
    async_trait::async_trait,
    rand::{rngs::StdRng, SeedableRng},
    sqlx::SqlitePool,
    std::{convert::identity, env, path::Path, sync::Arc},
    structopt::StructOpt,
};

use zeekoe::{
    customer::{
        cli::{self, Account::*, Customer::*},
        client::{SessionKey, ZkChannelAddress},
        database::ErasedQueryCustomer,
        defaults::config_path,
        Chan, Cli, Client, Config,
    },
    protocol::{self, ZkChannels},
};

mod close;
mod establish;
mod manage;
mod pay;

/// A single customer-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`Customer`] should implement this, except [`Configure`], which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    async fn run(self, rng: StdRng, config: Config) -> Result<(), anyhow::Error>;
}

pub async fn main_with_cli(cli: Cli) -> Result<(), anyhow::Error> {
    let config_path = cli.config.ok_or_else(config_path).or_else(identity)?;
    let config = Config::load(&config_path);

    // TODO: let this be made deterministic during testing
    let rng = StdRng::from_entropy();

    match cli.customer {
        Configure(cli::Configure { .. }) => {
            drop(config);
            tokio::task::spawn_blocking(|| Ok(edit::edit_file(config_path)?)).await?
        }
        Account(Import(import)) => import.run(rng, config.await?).await,
        Account(Remove(remove)) => remove.run(rng, config.await?).await,
        List(list) => list.run(rng, config.await?).await,
        Rename(rename) => rename.run(rng, config.await?).await,
        Establish(establish) => establish.run(rng, config.await?).await,
        Pay(pay) => pay.run(rng, config.await?).await,
        Refund(refund) => refund.run(rng, config.await?).await,
        Close(close) => close.run(rng, config.await?).await,
    }
}

/// Connect to a given [`ZkChannelAddress`], configured using the parameters in the [`Config`].
pub async fn connect(
    config: &Config,
    address: ZkChannelAddress,
) -> Result<(SessionKey, Chan<protocol::ZkChannels>), anyhow::Error> {
    let Config {
        backoff,
        connection_timeout,
        max_pending_connection_retries,
        max_message_length,
        ..
    } = config;

    let mut client: Client<ZkChannels> = Client::new(*backoff);
    client
        .max_length(*max_message_length)
        .timeout(*connection_timeout)
        .max_pending_retries(*max_pending_connection_retries);

    #[cfg(feature = "allow_explicit_certificate_trust")]
    if let Ok(path_string) = env::var("ZEEKOE_TRUST_EXPLICIT_CERTIFICATE") {
        let path = Path::new(&path_string);
        if path.is_relative() {
            return Err(anyhow::anyhow!("Path specified in `ZEEKOE_TRUST_EXPLICIT_CERTIFICATE` must be absolute, but the current value, \"{}\", is relative", path_string));
        }
        client.trust_explicit_certificate(path)?;
    }

    Ok(client.connect(address).await?)
}

pub async fn database(
    config: &Config,
) -> Result<Arc<dyn ErasedQueryCustomer + Send + Sync>, anyhow::Error> {
    use zeekoe::customer::config::DatabaseLocation;
    Ok(
        match match config.database.clone() {
            None => zeekoe::customer::defaults::database_location()?,
            Some(l) => l,
        } {
            DatabaseLocation::InMemory => Arc::new(SqlitePool::connect("file::memory:").await?),
            DatabaseLocation::Sqlite(ref uri) => Arc::new(SqlitePool::connect(uri).await?),
            DatabaseLocation::Postgres(_) => {
                return Err(anyhow::anyhow!(
                    "Postgres database support is not yet implemented"
                ))
            }
        },
    )
}

#[allow(unused)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    main_with_cli(Cli::from_args()).await
}
