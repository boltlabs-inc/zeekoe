pub(crate) mod common;

use rand::SeedableRng;
use zeekoe::{
    customer::{self, database::StateName as CustomerStatus, zkchannels::Command},
    merchant::{self, zkchannels::Command as _},
    protocol::ChannelStatus as MerchantStatus,
};

use {
    common::{customer_cli, merchant_cli, Party},
    rand::prelude::StdRng,
    std::{panic, time::Duration},
    structopt::StructOpt,
};

#[tokio::main]
pub async fn main() {
    let rng = StdRng::from_entropy();
    let server_future = common::setup(&rng).await;
    let customer_config = customer::Config::load(common::CUSTOMER_CONFIG)
        .await
        .expect("Failed to load customer config");
    let merchant_config = merchant::Config::load(common::MERCHANT_CONFIG)
        .await
        .expect("Failed to load merchant config");

    // Give the server some time to get set up. Maybe we can log and do this more precisely
    tokio::time::sleep(tokio::time::Duration::new(5, 0)).await;

    // Run every test, printing out details if it fails
    let tests = tests();
    println!("Executing {} tests", tests.len());
    let mut results = Vec::with_capacity(tests.len());
    for test in tests {
        println!("Now running: {}", test.name);
        let result = test.execute(&rng, &customer_config, &merchant_config).await;
        if let Err(error) = &result {
            eprintln!("Test failed: {}\n{}", test.name, error)
        }
        results.push(result);
    }

    // Fail if any test failed. This is separate from evaluation to enforce that _every_ test must
    // run -- otherwise, any will short-circuit the execution
    assert!(results.iter().all(|result| result.is_ok()));

    common::teardown(server_future).await;
}

/// Get a list of tests to execute.
/// Assumption: none of these will cause a fatal error to the long-running processes (merchant
/// server or customer watcher).
fn tests() -> Vec<Test> {
    vec![Test {
        name: "Channel establishes correctly".to_string(),
        operations: vec![(
            Operation::Establish,
            Outcome {
                customer_status: CustomerStatus::Ready,
                merchant_status: MerchantStatus::Active,
                error: None,
            },
        )],
    }]
    /*
    vec![Test {
        name: "Channel establishes correctly".to_string(),
        operations: vec![(
            Operation::NoOp, // this is a testing-the-tests hack to skip waiting for establish
            Outcome {
                customer_status: CustomerStatus::Ready,
                merchant_status: MerchantStatus::Active,
                error: None,
            },
        )],
    }]
    */
}

impl Test {
    async fn execute(
        &self,
        rng: &StdRng,
        customer_config: &customer::Config,
        merchant_config: &merchant::Config,
    ) -> Result<(), String> {
        for (op, expected_outcome) in &self.operations {
            // Clone inputs. A future refactor should look into having the `Command` trait take
            // these by reference instead.

            let outcome = match op {
                Operation::Establish => {
                    let est = customer_cli!(
                        Establish,
                        vec![
                            "establish",
                            "zkchannel://localhost",
                            "--label",
                            &self.name,
                            "--deposit",
                            "5 XTZ"
                        ]
                    );
                    est.run(rng.clone(), customer_config.clone())
                }
                Operation::NoOp => Box::pin(async { Ok(()) }),
                _ => return Err("Operation not implemented yet".to_string()),
            }
            .await;

            // Sleep until the servers have finished their thing
            tokio::time::sleep(op.wait_time()).await;

            // Check customer status
            let customer_detail_json = customer_cli!(Show, vec!["show", &self.name, "--json"])
                .run(rng.clone(), customer_config.clone())
                .await
                .map_err(|e| e.to_string())?;

            let customer_channel: customer::zkchannels::PublicChannelDetails =
                serde_json::from_str(&customer_detail_json).map_err(|err| format!("{}", err))?;

            assert_eq!(customer_channel.status(), expected_outcome.customer_status);

            // Check merchant status
            let channel_id = &customer_channel.channel_id().to_string();
            let merchant_details_json = merchant_cli!(Show, vec!["show", channel_id, "--json"])
                .run(merchant_config.clone())
                .await
                .map_err(|e| e.to_string())?;

            let merchant_channel: merchant::zkchannels::PublicChannelDetails =
                serde_json::from_str(&merchant_details_json).map_err(|err| format!("{}", err))?;

            assert_eq!(merchant_channel.status(), expected_outcome.merchant_status);

            // TODO: Compare error log to expected outcome: check for merchant server error, customer watcher error
            // TODO: How can we distinguish between errors from each operation and each test?
            // - the customer watcher prints errors with the label, so we can filter by label for this test
            // - the merchant watcher prints error with the channel id
            // - the merchant server just prints the error with no context
            // we can't span these with the test name.
            // should we delete error log after each test?
            // should we declare invariant that tests should only produce error on the last Operation?
            eprintln!("op outcome: {:?}", outcome);
        }
        Ok(())
    }
}
struct Test {
    pub name: String,
    pub operations: Vec<(Operation, Outcome)>,
}

/// Set of operations that can be executed by the test harness
#[allow(unused)]
enum Operation {
    Establish,
    Pay,
    PayAll,
    MutualClose,
    CustomerClose,
    MerchantExpiry,
    Store,
    Restore,
    NoOp,
}

impl Operation {
    /// Amount of time to wait before validating that an `Operation` has successfully completed.
    fn wait_time(&self) -> Duration {
        // The following actions cause delays:
        // (a) the watcher notices a contract change (60 seconds)
        // (b) a watcher posts a transaction on chain (10 seconds)
        // (c) a watcher waits for self-delay to elapse (60 seconds + noticing)
        let seconds = match self {
            Self::Establish
            | Self::Pay
            | Self::PayAll
            | Self::Store
            | Self::Restore
            | Self::NoOp => 0,

            // The merchant watcher must notice the contract status change
            Self::MutualClose => 60,
            // After the initial close tx is posted inline, either:
            // - the merchant notices, posts Dispute, and waits for it to confirm. The customer
            //   watcher notices and updates status (130)
            // - the merchant notices and does nothing. The customer watcher waits for self-delay to
            //   elapse, then posts claim (130)
            Self::CustomerClose => 130,
            // Expiry is either:
            // - the same as customer close, but preceded by the customer noticing expiry
            // and posting the corrected balances (+70)
            // - the merchant waits for self-delay to elapse, then claims funds (130)
            Self::MerchantExpiry => 200,
        };
        Duration::from_secs(seconds)
    }
}

#[derive(PartialEq)]
struct Outcome {
    customer_status: CustomerStatus,
    merchant_status: MerchantStatus,
    /// Which process, if any, had an error?
    error: Option<Party>,
}
