use std::{
    collections::HashMap,
    fs::{self, File},
    io::{Read, Write},
    sync::Mutex,
};

use {
    futures::future::{self, Join},
    rand::prelude::StdRng,
    structopt::StructOpt,
    thiserror::Error,
    tokio::task::JoinHandle,
    tracing::{error, info_span},
    tracing_futures::Instrument,
};

use zeekoe::{
    customer::zkchannels::Command,
    merchant::{cli::Merchant as MerchantCli, zkchannels::Command as _},
    timeout::WithTimeout,
};

pub const CUSTOMER_CONFIG: &str = "integration_tests/gen/TestCustomer.toml";
pub const MERCHANT_CONFIG: &str = "integration_tests/gen/TestMerchant.toml";
pub const ERROR_FILENAME: &str = "integration_tests/gen/errors.log";

/// Give a name to the slightly annoying type of the joined server futures
type ServerFuture =
    Join<JoinHandle<Result<(), anyhow::Error>>, JoinHandle<Result<(), anyhow::Error>>>;

/// Set of processes that run during a test.
#[derive(Debug, PartialEq)]
#[allow(unused)]
pub enum Party {
    MerchantServer,
    CustomerWatcher,
    /// The process corresponding to the `Operation` executed by the test harness
    ActiveOperation,
}

#[allow(unused)]
impl Party {
    const fn to_string(&self) -> &str {
        match self {
            Party::MerchantServer => "merchant server",
            Party::CustomerWatcher => "customer watcher",
            Party::ActiveOperation => "active operation",
        }
    }
}

// Form a customer CLI request. These cannot be constructed directly because the CLI types are
// non-exhaustive.
macro_rules! parse_customer_cli {
    ($cli:ident, $args:expr) => {
        match ::zeekoe::customer::cli::Customer::from_iter($args) {
            ::zeekoe::customer::cli::Customer::$cli(result) => result,
            _ => panic!("Failed to parse customer CLI"),
        }
    };
}
pub(crate) use parse_customer_cli;

pub async fn setup(rng: &StdRng) -> ServerFuture {
    // delete existing data from previous runs
    let _ = fs::remove_dir_all("integration_tests/gen/");
    let _ = fs::create_dir("integration_tests/gen");

    // ...copy keys from dev/ directory to here
    let _ = fs::copy("dev/localhost.crt", "integration_tests/gen/localhost.crt");
    let _ = fs::copy("dev/localhost.key", "integration_tests/gen/localhost.key");

    // write config options for each party
    let customer_config = customer_test_config().await;
    let merchant_config = merchant_test_config().await;

    tracing_subscriber::fmt()
        .with_writer(Mutex::new(
            File::create(ERROR_FILENAME).expect("Failed to open log file"),
        ))
        .init();

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

    let watch = parse_customer_cli!(Watch, vec!["./target/debug/zkchannel-customer", "watch"]);

    let customer_handle = tokio::spawn(
        watch
            .run(rng.clone(), customer_config)
            .instrument(info_span!(Party::CustomerWatcher.to_string())),
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
        ("database", "{ sqlite = \"customer.db\" }"),
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
        ("database", "{ sqlite = \"merchant.db\" }"),
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
