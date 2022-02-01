pub(crate) mod common;

use rand::SeedableRng;
use zeekoe::{
    customer::{
        self,
        database::StateName as CustomerStatus,
        zkchannels::{Command, PublicChannelDetails},
    },
    protocol::ChannelStatus as MerchantStatus,
};
use {
    common::{customer_cli, Party},
    rand::prelude::StdRng,
    std::panic,
    structopt::StructOpt,
    tracing::{info_span, Instrument},
};

#[tokio::main]
pub async fn main() {
    let rng = StdRng::from_entropy();
    let server_future = common::setup(&rng).await;
    let customer_config = customer::Config::load(common::CUSTOMER_CONFIG)
        .await
        .expect("Failed to load customer config");

    // Give the server some time to get set up. Maybe we can log and do this more precisely
    tokio::time::sleep(tokio::time::Duration::new(5, 0)).await;

    // Run every test, printing out details if it fails
    let tests = tests();
    println!("Executing {} tests", tests.len());
    let mut results = Vec::with_capacity(tests.len());
    for test in tests {
        println!("Now running: {}", test.name);
        let result = test.execute(&rng, &customer_config).await;
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
    /*
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
    */
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
}

impl Test {
    async fn execute(&self, rng: &StdRng, config: &customer::Config) -> Result<(), String> {
        for (op, expected_outcome) in &self.operations {
            // Clone inputs. A future refactor should look into having the `Command` trait take
            // these by reference instead.

            let outcome = match op {
                Operation::Establish => {
                    let est = customer_cli!(
                        Establish,
                        vec![
                            "./target/debug/zkchannel-customer",
                            "establish",
                            "zkchannel://localhost",
                            "--label",
                            &self.name,
                            "--deposit",
                            "5 XTZ"
                        ]
                    );
                    est.run(rng.clone(), config.clone())
                }
                Operation::NoOp => Box::pin(async { Ok(()) }),
                _ => return Err("Operation not implemented yet".to_string()),
            }
            .await;

            // Check customer status
            let show = customer_cli!(
                Show,
                vec!["zkchannel-customer", "show", &self.name, "--json"]
            );
            let channel_detail_json = show
                .run(rng.clone(), config.clone())
                .instrument(info_span!("customer show"))
                .await
                .map_err(|e| e.to_string())?;

            let channel_details: PublicChannelDetails =
                serde_json::from_str(&channel_detail_json).map_err(|err| format!("{}", err))?;

            assert_eq!(channel_details.status(), expected_outcome.customer_status);

            // TODO: Compare outcome + error log to expected outcome
            eprintln!("op outcome: {:?}", outcome);

            //eprintln!("log output: {}", common::get_error().map_err(|e| format!("{:?}", e))?)

            // TODO: Call list to check status of each party
            // Compare statuses to expected status
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

#[derive(PartialEq)]
struct Outcome {
    customer_status: CustomerStatus,
    merchant_status: MerchantStatus,
    /// Which process, if any, had an error?
    error: Option<Party>,
}
