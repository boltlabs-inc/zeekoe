//! Complete customer logic for the zkchannels protocol

#[cfg(not(feature = "allow_explicit_certificate_trust"))]
use tracing::warn;
use webpki::DnsNameRef;

use anyhow::Context;
use async_trait::async_trait;
use sqlx::SqlitePool;
use std::{sync::Arc, time::Duration};
use thiserror::Error;

use crate::{
    customer::{
        client::{Backoff, SessionKey},
        config::DatabaseLocation,
        database::{self, connect_sqlite, QueryCustomer},
        defaults, Chan, ChannelName, Client, Config,
    },
    escrow::tezos::TezosClient,
    protocol,
};

pub(crate) mod close;
mod establish;
mod manage;
mod pay;
mod watch;

use crate::transport::ZkChannelAddress;
pub use manage::PublicChannelDetails;

/// A single customer-side command, parameterized by the currently loaded configuration.
///
/// All subcommands of [`cli::Customer`](crate::customer::cli::Customer) should implement this,
/// except [`Configure`](crate::customer::cli::Customer::Configure), which does not need
/// to start with a valid loaded configuration.
#[async_trait]
pub trait Command {
    type Output;

    /// Run the command to completion using the given random number generator for all randomness and
    /// the given customer configuration.
    async fn run(self, config: Config) -> Result<Self::Output, anyhow::Error>;
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
        warn!(
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

    let address = DnsNameRef::try_from_ascii_str("localhost").unwrap();
    let client: Client<protocol::daemon::Daemon> = Client::new(backoff);
    Ok(client.connect(&address.into(), config.daemon_port).await?)
}

/// Connect to the database specified by the configuration.
pub async fn database(config: &Config) -> Result<Arc<dyn QueryCustomer>, anyhow::Error> {
    let location = match config.database.clone() {
        None => defaults::database_location()?,
        Some(l) => l,
    };

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

#[derive(Debug, Error)]
pub enum TezosClientError {
    #[error("Contract details for {0} are not set")]
    ContractDetailsNotSet(ChannelName),
    #[error("Failed to  load key material: {0}")]
    InvalidKeyMaterial(#[from] anyhow::Error),
    #[error(transparent)]
    DatabaseError(#[from] database::Error),
}

pub async fn load_tezos_client(
    config: &Config,
    channel_name: &ChannelName,
    database: &dyn QueryCustomer,
) -> Result<TezosClient, TezosClientError> {
    let contract_id = match database.contract_details(channel_name).await?.contract_id {
        Some(contract_id) => contract_id,
        None => {
            return Err(TezosClientError::ContractDetailsNotSet(
                channel_name.clone(),
            ))
        }
    };

    Ok(TezosClient {
        uri: Some(config.tezos_uri.clone()),
        contract_id,
        client_key_pair: config.load_tezos_key_material()?,
        confirmation_depth: config.confirmation_depth,
        self_delay: config.self_delay,
    })
}
