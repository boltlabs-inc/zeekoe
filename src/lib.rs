pub mod amount;
pub mod arbiter;
pub mod customer;
pub mod escrow;
pub mod merchant;
pub mod protocol;
pub mod timeout;

mod cli;
mod config;
mod database;
mod defaults;
mod transport;
mod zkchannels;

use std::fmt;

/// Logs used to verify that an operation completed in the integration tests.
pub enum TestLogs {
    CustomerWatcherSpawned,
    /// Merchant server successfully serving at address described by parameter.
    MerchantServerSpawned(String),
}

impl fmt::Display for TestLogs {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                TestLogs::CustomerWatcherSpawned => "customer watcher created successfully".into(),
                TestLogs::MerchantServerSpawned(addr) => format!("serving on: {:?}", addr),
            }
        )
    }
}
