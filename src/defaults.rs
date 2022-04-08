use dialectic_reconnect::Backoff;
use directories::ProjectDirs;
use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    time::Duration,
};

fn project_dirs() -> Result<ProjectDirs, anyhow::Error> {
    ProjectDirs::from("", shared::ORGANIZATION, shared::APPLICATION)
        .ok_or_else(|| anyhow::anyhow!("Could not open user's home directory"))
}

pub(crate) mod shared {
    use super::*;

    pub const ORGANIZATION: &str = "Bolt Labs";

    pub const APPLICATION: &str = "zkchannel";

    pub const fn max_pending_connection_retries() -> usize {
        4
    }

    pub const fn max_message_length() -> usize {
        1024 * 16
    }

    pub const fn port() -> u16 {
        2611
    }

    /// Length of time a party must wait before claiming funds.
    pub const fn self_delay() -> u64 {
        // 2 days, in seconds.
        2 * 24 * 60 * 60
    }

    /// Depth at which on-chain transactions can be considered finalized.
    pub const fn confirmation_depth() -> u64 {
        20
    }

    /// Length of time (seconds) that a party waits for a normal message to be computed and sent.
    pub const fn message_timeout() -> Duration {
        Duration::from_secs(60)
    }

    /// Length of time (seconds) for a party to post and confirm a transaction on Tezos.
    pub const fn transaction_timeout() -> Duration {
        Duration::from_secs(25 * 60)
    }

    /// Length of time (seconds) for a party to retrieve and verify the status of a Tezos contract.
    pub const fn verification_timeout() -> Duration {
        Duration::from_secs(180)
    }
}

pub mod merchant {
    use super::*;

    pub use super::shared::*;

    pub const fn address() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
    }

    pub const CONFIG_FILE: &str = "Merchant.toml";

    pub fn config_path() -> Result<PathBuf, anyhow::Error> {
        Ok(project_dirs()?.config_dir().join(CONFIG_FILE))
    }
}

pub mod customer {
    use super::*;
    use crate::customer::config::DatabaseLocation;

    pub use super::shared::*;

    pub fn backoff() -> Backoff {
        Backoff::with_delay(Duration::from_secs(1))
    }

    pub const fn connection_timeout() -> Option<Duration> {
        Some(Duration::from_secs(60))
    }

    pub const CONFIG_FILE: &str = "Customer.toml";

    pub const DATABASE_FILE: &str = "customer.db";

    pub fn config_path() -> Result<PathBuf, anyhow::Error> {
        Ok(project_dirs()?.config_dir().join(CONFIG_FILE))
    }

    pub fn database_location() -> Result<DatabaseLocation, anyhow::Error> {
        Ok(DatabaseLocation::Sqlite(
            project_dirs()?
                .data_dir()
                .join(DATABASE_FILE)
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid UTF-8 in database location path"))?
                .into(),
        ))
    }

    pub const fn max_note_length() -> u64 {
        1024 * 8
    }

    pub const fn daemon_port() -> u16 {
        // ZKD :3
        26114
    }

    /// Length of time (seconds) that a customer waits for the merchant to approve a new channel
    /// or a payment.
    pub const fn approval_timeout() -> Duration {
        Duration::from_secs(360)
    }
}
