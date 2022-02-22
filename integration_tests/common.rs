use std::{
    collections::HashMap,
    fmt,
    fs::{self, File},
    io::Write,
    process::Command,
    sync::Mutex,
};

use {
    futures::future::{self, Join},
    rand::prelude::StdRng,
    structopt::StructOpt,
    strum::IntoEnumIterator,
    strum_macros::EnumIter,
    tokio::{task::JoinHandle, time::Duration},
    tracing::info_span,
    tracing_futures::Instrument,
};

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
    IpV6,
}

impl MerchantServices {
    fn to_str(self) -> &'static str {
        match self {
            Self::IpV4 => "127.0.0.1",
            Self::IpV6 => "::1",
        }
    }
}

impl fmt::Display for MerchantServices {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Note: this hard-codes the default port.
        let ipaddr = match self {
            Self::IpV4 => self.to_str().to_string(),
            Self::IpV6 => format!("[{}]", self.to_str()),
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

pub async fn setup(rng: &StdRng) -> ServerFuture {
    let _ = fs::create_dir("integration_tests/gen");

    // Create self-signed SSL certificate config file and certificate
    File::create("integration_tests/gen/ssl_config")
        .expect("Failed to open file with SSL config")
        .write_all(b"[dn]\nCN=localhost\n[req]\ndistinguished_name = dn\n[EXT]\nsubjectAltName=DNS:localhost\nkeyUsage=digitalSignature\nextendedKeyUsage=serverAuth")
        .expect("Failed to write SSL config to file");
    Command::new("openssl")
        .arg("req")
        .arg("-x509")
        .args(["-out", "integration_tests/gen/localhost.crt"])
        .args(["-keyout", "integration_tests/gen/localhost.key"])
        .args(["-newkey", "rsa:2048"])
        .arg("-nodes")
        .arg("-sha256")
        .args(["-subj", "/CN=localhost"])
        .args(["-extensions", "EXT"])
        .args(["-config", "integration_tests/gen/ssl_config"])
        .spawn()
        .expect("Failed to generate certs");

    // write config options for each party
    let customer_config = customer_test_config().await;
    let merchant_config = merchant_test_config().await;

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
            .run(rng.clone(), customer_config)
            .instrument(info_span!(Party::CustomerWatcher.to_str())),
    );

    // Check the logs of merchant + customer for indication of a successful set-up
    // Note: hard-coded to match the 2-service merchant with default port.
    let checks = vec![
        await_log(
            Party::MerchantServer,
            TestLogs::MerchantServerSpawned(MerchantServices::IpV4.to_string()),
        ),
        await_log(
            Party::MerchantServer,
            TestLogs::MerchantServerSpawned(MerchantServices::IpV6.to_string()),
        ),
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

    // delete data from this run
    let _ = fs::remove_dir_all("integration_tests/gen/");
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