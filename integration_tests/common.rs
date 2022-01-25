use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    sync::Mutex,
};

use futures::future::Join;
use tokio::task::JoinHandle;

use {
    futures::future,
    thiserror::Error,
    tracing::{error, info, info_span},
    tracing_futures::Instrument,
    zeekoe::timeout::WithTimeout,
};

pub const CUSTOMER_CONFIG: &str = "TestCustomer.toml";
pub const MERCHANT_CONFIG: &str = "TestMerchant.toml";
pub const ERROR_FILENAME: &str = "errors.log";

// Give a name to the slightly annoying type of the joined server futures
type ServerFuture = Join<JoinHandle<()>, JoinHandle<()>>;

#[derive(Debug)]
#[allow(unused)]
pub enum Party {
    MerchantServer,
    CustomerWatcher,
}

#[allow(unused)]
impl Party {
    const fn to_string(&self) -> &str {
        match self {
            Party::MerchantServer => "merchant server",
            Party::CustomerWatcher => "customer watcher",
        }
    }
}

pub async fn setup() -> ServerFuture {
    // write config options for each party
    write_config_file(CUSTOMER_CONFIG, customer_test_config());
    write_config_file(MERCHANT_CONFIG, merchant_test_config());

    tracing_subscriber::fmt()
        .with_writer(Mutex::new(
            File::create(ERROR_FILENAME).expect("Failed to open log file"),
        ))
        .init();
    info!("spawning tasks now...");

    // Stand-in task for the customer watcher.
    let one = tokio::spawn(
        async {
            loop {
                error!("This task worked exactly like I expected it to");
                tokio::time::sleep(tokio::time::Duration::new(3, 0)).await;
            }
        }
        .instrument(info_span!("customer watcher")),
    );

    // Stand-in task for the merchant server
    let two = tokio::spawn(
        async {
            loop {
                error!("This task is also good at its job");
                tokio::time::sleep(tokio::time::Duration::new(5, 0)).await;
            }
        }
        .instrument(info_span!("merchant server")),
    );

    future::join(one, two)
}

pub async fn teardown(server_future: ServerFuture) {
    // Ignore the result because we expect it to be an `Expired` error
    let _result = server_future
        .with_timeout(tokio::time::Duration::new(1, 0))
        .await;
}

/// Encode the customizable fields of the zeekoe customer Config struct for testing.
fn customer_test_config() -> String {
    let m = HashMap::from([
        ("database", "{ sqlite = \"customer-sandbox.db\" }"),
        ("trust_certificate", "\"localhost.crt\""),
        ("tezos_account", "{ alias = \"alice\" }"),
        ("tezos_uri", "\"http://localhost:20000\""),
        ("self_delay", "120"),
        ("confirmation_depth", "1"),
    ]);

    m.into_iter().fold("".to_string(), |acc, (key, value)| {
        format!("{}{} = {}\n", acc, key.to_string(), value.to_string())
    })
}

/// Encode the customizable fields of the zeekoe merchant Config struct for testing.
fn merchant_test_config() -> String {
    let tezos_config = customer_test_config()
        .replace("alice", "bob")
        .replace("customer", "merchant")
        .replace("trust_certificate = \"localhost.crt\"\n", "");

    // helper to write out the service for ipv4 and v6 localhost addresses
    let generate_service = |addr: &str| {
        HashMap::from([
            ("address", addr),
            ("private_key", "localhost.key"),
            ("certificate", "localhost.crt"),
        ])
        .into_iter()
        .fold("\n[[service]]".to_string(), |acc, (key, value)| {
            format!("{}\n{} = \"{}\"", acc, key, value)
        })
    };

    format!(
        "{}{}\n{}",
        tezos_config,
        generate_service("::1"),
        generate_service("127.0.0.1")
    )
}

/// Write out the configuration in `contents` to the file at `path`.
fn write_config_file(path: &str, contents: String) {
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .unwrap_or_else(|_| panic!("Could not open config file: {}", path))
        .write_all(contents.as_bytes())
        .unwrap_or_else(|_| panic!("Failed to write to config file: {}", path));
}

#[derive(Debug, Error)]
pub enum LogError {
    #[error("Failed to open log file: {0}")]
    OpenFailed(std::io::Error),
    #[error("Failed to read contents of file: {0}")]
    ReadFailed(std::io::Error),
}

/// Get any errors from the log file.
///
/// Current behavior: outputs the entire log
/// Ideal behavior: pass a Party, maybe an indicator of which test / channel name we want. Return
/// only the lines relevant to that setting.
pub fn get_error() -> Result<String, LogError> {
    let mut file = File::open(ERROR_FILENAME).map_err(LogError::OpenFailed)?;
    let mut error = String::new();
    file.read_to_string(&mut error)
        .map_err(LogError::ReadFailed)?;

    Ok(error)
}
