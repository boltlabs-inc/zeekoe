use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    sync::Mutex,
};

use {
    futures::future::{self, Join},
    thiserror::Error,
    tokio::task::JoinHandle,
    tracing::{error, info_span},
    tracing_futures::Instrument,
};

use rand::SeedableRng;
use structopt::StructOpt;
use zeekoe::{
    customer::{cli::Customer as CustomerCli, zkchannels::Command},
    merchant::{cli::Merchant as MerchantCli, zkchannels::Command as _},
    timeout::WithTimeout,
};

pub const CUSTOMER_CONFIG: &str = "TestCustomer.toml";
pub const MERCHANT_CONFIG: &str = "TestMerchant.toml";
pub const ERROR_FILENAME: &str = "errors.log";

// Give a name to the slightly annoying type of the joined server futures
type ServerFuture =
    Join<JoinHandle<Result<(), anyhow::Error>>, JoinHandle<Result<(), anyhow::Error>>>;

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
    let customer_config = customer_test_config().await;
    let merchant_config = merchant_test_config().await;

    tracing_subscriber::fmt()
        .with_writer(Mutex::new(
            File::create(ERROR_FILENAME).expect("Failed to open log file"),
        ))
        .init();

    // Form the customer watch request (cannot construct it directly because `Watch` is
    // non-exhaustive)
    let watch = match CustomerCli::from_iter(vec!["./target/debug/zkchannel-customer", "watch"]) {
        CustomerCli::Watch(watch) => watch,
        _ => panic!("Failed to parse customer watch CLI"),
    };

    // TODO: make this a fixed seed?
    let rng = rand::rngs::StdRng::from_entropy();

    let customer_handle = tokio::spawn(
        watch
            .run(rng, customer_config)
            .instrument(info_span!(Party::CustomerWatcher.to_string())),
    );

    // Form the merchant run request (same non-exhaustive situation here)
    let run = match MerchantCli::from_iter(vec!["./target/debug/zkchannel-merchant", "run"]) {
        MerchantCli::Run(run) => run,
        _ => panic!("Failed to parse merchant run CLI"),
    };

    // Stand-in task for the merchant server
    let merchant_handle = tokio::spawn(
        run.run(merchant_config)
            .instrument(info_span!(Party::MerchantServer.to_string())),
    );

    future::join(customer_handle, merchant_handle)
}

pub async fn teardown(server_future: ServerFuture) {
    // Ignore the result because we expect it to be an `Expired` error
    let _result = server_future
        .with_timeout(tokio::time::Duration::new(1, 0))
        .await;
}

/// Encode the customizable fields of the zeekoe customer Config struct for testing.
async fn customer_test_config() -> zeekoe::customer::Config {
    let m = HashMap::from([
        ("database", "{ sqlite = \"customer-sandbox.db\" }"),
        ("trust_certificate", "\"localhost.crt\""),
        ("tezos_account", "{ alias = \"alice\" }"),
        ("tezos_uri", "\"http://localhost:20000\""),
        ("self_delay", "120"),
        ("confirmation_depth", "1"),
    ]);

    let contents = m.into_iter().fold("".to_string(), |acc, (key, value)| {
        format!("{}{} = {}\n", acc, key.to_string(), value.to_string())
    });

    write_config_file(CUSTOMER_CONFIG, contents);

    zeekoe::customer::Config::load(CUSTOMER_CONFIG)
        .await
        .expect("Failed to load customer config")
}

/// Encode the customizable fields of the zeekoe merchant Config struct for testing.
async fn merchant_test_config() -> zeekoe::merchant::Config {
    let m = HashMap::from([
        ("database", "{ sqlite = \"merchant-sandbox.db\" }"),
        ("tezos_account", "{ alias = \"bob\" }"),
        ("tezos_uri", "\"http://localhost:20000\""),
        ("self_delay", "120"),
        ("confirmation_depth", "1"),
    ]);

    let tezos_contents = m.into_iter().fold("".to_string(), |acc, (key, value)| {
        format!("{}{} = {}\n", acc, key.to_string(), value.to_string())
    });

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

    let contents = format!(
        "{}{}\n{}",
        tezos_contents,
        generate_service("::1"),
        generate_service("127.0.0.1")
    );

    write_config_file(MERCHANT_CONFIG, contents);

    zeekoe::merchant::Config::load(MERCHANT_CONFIG)
        .await
        .expect("failed to load merchant config")
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
#[allow(unused)]
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
#[allow(unused)]
pub fn get_error() -> Result<String, LogError> {
    let mut file = File::open(ERROR_FILENAME).map_err(LogError::OpenFailed)?;
    let mut error = String::new();
    file.read_to_string(&mut error)
        .map_err(LogError::ReadFailed)?;

    Ok(error)
}
