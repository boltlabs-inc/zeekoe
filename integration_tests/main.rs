pub(crate) mod common;
pub(crate) mod tests;

use zeekoe::{
    customer::{self},
    merchant::{self},
    TestLogs,
};

use common::Party;
use std::{fs::OpenOptions, panic};
use tests::{all_tests, get_logs, LogType};

#[tokio::main]
pub async fn main() {
    // Read tezos URI from arguments, or use standard sandbox URI
    let default = "http://localhost:20000".to_string();
    let tezos_uri = match std::env::args().last() {
        None => default,
        // This means no argument was passed and args just has the executable name
        Some(s) if s.contains("integration_tests") => default,
        Some(tezos_uri) => tezos_uri,
    };
    eprintln!("Using tezos URI: {}", tezos_uri);

    let server_future = common::setup(tezos_uri).await;
    let customer_config = customer::Config::load(common::CUSTOMER_CONFIG)
        .await
        .expect("Failed to load customer config");
    let merchant_config = merchant::Config::load(common::MERCHANT_CONFIG)
        .await
        .expect("Failed to load merchant config");

    common::await_leveled_blockchain(&customer_config)
        .await
        .expect("Failed to check blockchain level");

    // Run every test, printing out details if it fails
    let tests = all_tests();
    println!("Executing {} tests", tests.len());
    let mut results = Vec::with_capacity(tests.len());
    for test in tests {
        eprintln!("\n\ntest integration_tests::{} ... ", test.name);
        let result = test.execute(&customer_config, &merchant_config).await;
        if let Err(error) = &result {
            eprintln!("failed\n{:?}", error)
        } else {
            eprintln!("ok")
        }
        results.push(result);

        // Clear error log
        OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&common::ERROR_FILENAME)
            .unwrap_or_else(|e| panic!("Failed to clear error file after {}: {:?}", test.name, e));
    }

    common::teardown(server_future).await;

    // Fail if any test failed. This is separate from evaluation to enforce that _every_ test must
    // run without short-circuiting the execution at first failure
    if !results.iter().all(|result| result.is_ok()) {
        std::process::exit(101);
    }
}

/// Wait for the log file to contain a specific entry.
///
/// This checks the log every 1 second; refactor if greater granularity is needed.
async fn await_log(party: Party, log: TestLogs) -> Result<(), anyhow::Error> {
    loop {
        if get_logs(LogType::Info, party)?.contains(&log.to_string()) {
            return Ok(());
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
