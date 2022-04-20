use std::{
    collections::HashMap,
    fmt,
    fs::{self, File},
    io::Write,
    process::Command,
    sync::Mutex,
};

use futures::future::{self, Join};
use structopt::StructOpt;
use strum::IntoEnumIterator;
use strum_macros::EnumIter;
use tokio::{task::JoinHandle, time::Duration};
use tracing::info_span;
use tracing_futures::Instrument;

use crate::{await_log, TestLogs};

use zeekoe::{
    customer::zkchannels::Command as _, merchant::zkchannels::Command as _, timeout::WithTimeout,
};

pub const CUSTOMER_CONFIG: &str = "integration_tests/gen/TestCustomer.toml";
pub const MERCHANT_CONFIG: &str = "integration_tests/gen/TestMerchant.toml";
pub const ERROR_FILENAME: &str = "integration_tests/gen/errors.log";

/// The default merchant services we will set up for tests (all run on localhost)
#[derive(Debug, Clone, Copy, EnumIter)]
enum MerchantServices {
    IpV4,
    // The server supports IPv6 but it doesn't run on the Github Actions test harness.
    //IpV6,
}

impl MerchantServices {
    fn to_str(self) -> &'static str {
        match self {
            Self::IpV4 => "127.0.0.1",
            //Self::IpV6 => "::1",
        }
    }
}

impl fmt::Display for MerchantServices {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Note: this hard-codes the default port.
        let ipaddr = match self {
            Self::IpV4 => self.to_str().to_string(),
            //Self::IpV6 => format!("[{}]", self.to_str()),
        };
        write!(f, "{}:2611", ipaddr)
    }
}

/// Give a name to the slightly annoying type of the joined server futures
type ServerFuture =
    Join<JoinHandle<Result<(), anyhow::Error>>, JoinHandle<Result<(), anyhow::Error>>>;

/// Set of processes that run during a test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Party {
    MerchantServer,
    CustomerWatcher,
    /// The process corresponding to the `Operation` executed by the test harness
    ActiveOperation(&'static str),
}

impl Party {
    pub const fn to_str(self) -> &'static str {
        match self {
            Party::MerchantServer => "party: merchant server",
            Party::CustomerWatcher => "party: customer watcher",
            Party::ActiveOperation(description) => description,
        }
    }
}

// Form a customer CLI request. These cannot be constructed directly because the CLI types are
// non-exhaustive.
macro_rules! customer_cli {
    ($cli:ident, $args:expr) => {
        match ::zeekoe::customer::cli::Customer::from_iter(
            ::std::iter::once("zkchannels-customer").chain($args),
        ) {
            ::zeekoe::customer::cli::Customer::$cli(result) => result,
            _ => panic!("Failed to parse customer CLI"),
        }
    };
}
pub(crate) use customer_cli;

/// Form a merchant CLI request. These cannot be constructed directly because the CLI types are
/// non-exhaustive.
macro_rules! merchant_cli {
    ($cli:ident, $args:expr) => {
        match ::zeekoe::merchant::cli::Merchant::from_iter(
            ::std::iter::once("zkchannels-merchant").chain($args),
        ) {
            ::zeekoe::merchant::cli::Merchant::$cli(result) => result,
            _ => panic!("Failed to parse merchant CLI"),
        }
    };
}
pub(crate) use merchant_cli;

pub async fn setup(tezos_uri: String) -> ServerFuture {
    let _ = fs::create_dir("integration_tests/gen");

    // Create self-signed SSL certificate in the generated directory
    Command::new("./dev/generate-certificates")
        .arg("integration_tests/gen")
        .spawn()
        .expect("Failed to generate new certificates");

    // write config options for each party
    let customer_config = customer_test_config(&tezos_uri).await;
    let merchant_config = merchant_test_config(&tezos_uri).await;

    // set up tracing for all log messages
    tracing_subscriber::fmt()
        .with_writer(Mutex::new(
            File::create(ERROR_FILENAME).expect("Failed to open log file"),
        ))
        .init();

    // Form the merchant run request and execute
    let run = merchant_cli!(Run, vec!["run"]);
    let merchant_handle = tokio::spawn(
        run.run(merchant_config)
            .instrument(info_span!(Party::MerchantServer.to_str())),
    );

    // Form the customer watch request and execute
    let watch = customer_cli!(Watch, vec!["watch"]);
    let customer_handle = tokio::spawn(
        watch
            .run(customer_config)
            .instrument(info_span!(Party::CustomerWatcher.to_str())),
    );

    // Check the logs of merchant + customer for indication of a successful set-up
    // Note: hard-coded to match the 2-service merchant with default port.
    let checks = vec![
        await_log(
            Party::MerchantServer,
            TestLogs::MerchantServerSpawned(MerchantServices::IpV4.to_string()),
        ),
        /*
        await_log(
            Party::MerchantServer,
            TestLogs::MerchantServerSpawned(MerchantServices::IpV6.to_string()),
        ),
        */
        await_log(Party::CustomerWatcher, TestLogs::CustomerWatcherSpawned),
    ];

    // Wait up to 30sec for the servers to set up or fail
    match future::join_all(checks)
        .with_timeout(Duration::from_secs(30))
        .await
    {
        Err(_) => panic!("Server setup timed out"),
        Ok(results) => {
            match results
                .into_iter()
                .collect::<Result<Vec<()>, anyhow::Error>>()
            {
                Ok(_) => {}
                Err(err) => panic!(
                    "Failed to read logs while waiting for servers to set up: {:?}",
                    err
                ),
            }
        }
    }

    future::join(customer_handle, merchant_handle)
}

pub async fn teardown(server_future: ServerFuture) {
    // Ignore the result because we expect it to be an `Expired` error
    let _result = server_future.with_timeout(Duration::from_secs(1)).await;

    // Delete data from this run
    let _ = fs::remove_dir_all("integration_tests/gen/");
}

/// Encode the customizable fields of the zeekoe customer Config struct for testing.
async fn customer_test_config(tezos_uri: &str) -> zeekoe::customer::Config {
    let quoted_tezos_uri = format!("\"{}\"", tezos_uri);
    let m = HashMap::from([
        ("database", "{ sqlite = \"customer.db\" }"),
        ("trust_certificate", "\"localhost.crt\""),
        ("tezos_account", "{ alias = \"alice\" }"),
        ("tezos_uri", quoted_tezos_uri.as_str()),
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
async fn merchant_test_config(tezos_uri: &str) -> zeekoe::merchant::Config {
    let quoted_tezos_uri = format!("\"{}\"", tezos_uri);
    let m = HashMap::from([
        ("database", "{ sqlite = \"merchant.db\" }"),
        ("tezos_account", "{ alias = \"bob\" }"),
        ("tezos_uri", quoted_tezos_uri.as_str()),
        ("self_delay", "120"),
        ("confirmation_depth", "1"),
    ]);

    let tezos_contents = m.into_iter().fold("".to_string(), |acc, (key, value)| {
        format!("{}{} = {}\n", acc, key.to_string(), value.to_string())
    });

    // Helper to write out the service for the merchant service addresses
    let generate_service = |addr: MerchantServices| {
        HashMap::from([
            ("address", addr.to_str()),
            ("private_key", "localhost.key"),
            ("certificate", "localhost.crt"),
        ])
        .into_iter()
        .fold("\n[[service]]".to_string(), |acc, (key, value)| {
            format!("{}\n{} = \"{}\"", acc, key, value)
        })
    };

    let services = MerchantServices::iter()
        .map(generate_service)
        .fold(String::new(), |acc, next| format!("{}\n{}", acc, next));

    let contents = format!("{}{}", tezos_contents, services,);

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

/// Wrapper type to deserialize blockchain level.
#[derive(Debug, serde::Deserialize)]
struct BlockchainLevel {
    level: BlockchainLevelDetail,
}

/// Inner type used to deserialize blockchain level.
#[derive(Debug, serde::Deserialize)]
struct BlockchainLevelDetail {
    level: u64,
}

/// The minimum required depth to originate contracts on the Tezos blockchain.
static MINIMUM_LEVEL: u64 = 60;

/// Waits for the blockchain level to reach the required minimum depth. Necessary when using the
/// sandbox, which will start at 0 by default.
pub async fn await_leveled_blockchain(
    config: &zeekoe::customer::Config,
) -> Result<(), anyhow::Error> {
    eprintln!("Waiting for blockchain to reach depth {}...", MINIMUM_LEVEL);
    loop {
        let body = reqwest::get(format!(
            "{}/chains/main/blocks/head/metadata",
            config.tezos_uri
        ))
        .await?
        .text()
        .await?;

        let level = serde_json::from_str::<BlockchainLevel>(&body)?.level.level;

        if level >= MINIMUM_LEVEL {
            break;
        }

        let wait_time = (MINIMUM_LEVEL - level) * 4;
        eprintln!("Current level: {:?}. Waiting {} seconds", level, wait_time);
        tokio::time::sleep(tokio::time::Duration::from_secs(wait_time)).await;
    }
    Ok(())
}
