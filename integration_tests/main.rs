pub(crate) mod common;

use zeekoe::{
    amount::Amount,
    customer::{self, database::StateName as CustomerStatus, zkchannels::Command},
    merchant::{self, zkchannels::Command as _},
    protocol::ChannelStatus as MerchantStatus,
    TestLogs,
};
use zkabacus_crypto::{CustomerBalance, MerchantBalance};

use anyhow::Context;
use common::{customer_cli, merchant_cli, Party};
use std::{
    convert::TryInto,
    fs::{File, OpenOptions},
    io::Read,
    panic,
    str::FromStr,
    time::Duration,
};
use structopt::StructOpt;
use thiserror::Error;

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
    let tests = tests();
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

/// Get a list of tests to execute.
/// Assumption: none of these will cause a fatal error to the long-running processes (merchant
/// server or customer watcher).
fn tests() -> Vec<Test> {
    let default_balance = 5;
    let default_payment = 1;
    vec![
        Test {
            name: "Channel establishes correctly".to_string(),
            operations: vec![(
                Operation::Establish(default_balance),
                Outcome {
                    error: None,
                    channel_outcome: Some(ChannelOutcome {
                        customer_status: CustomerStatus::Ready,
                        merchant_status: MerchantStatus::Active,
                        customer_balance: into_customer_balance(default_balance),
                        merchant_balance: MerchantBalance::zero(),
                    }),
                },
            )],
        },
        Test {
            name: "Channels cannot share names".to_string(),
            operations: vec![
                (
                    Operation::Establish(default_balance),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: into_customer_balance(default_balance),
                            merchant_balance: MerchantBalance::zero(),
                        }),
                    },
                ),
                (
                    Operation::Establish(default_balance),
                    Outcome {
                        error: Some(Party::ActiveOperation("establish")),
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: into_customer_balance(default_balance),
                            merchant_balance: MerchantBalance::zero(),
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Payment equal to the balance is successful".to_string(),
            operations: vec![
                (
                    Operation::Establish(default_balance),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: into_customer_balance(default_balance),
                            merchant_balance: MerchantBalance::zero(),
                        }),
                    },
                ),
                (
                    Operation::Pay(default_balance),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: CustomerBalance::zero(),
                            merchant_balance: into_merchant_balance(default_balance),
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Payment less than the balance is successful".to_string(),
            operations: vec![
                (
                    Operation::Establish(default_balance),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: into_customer_balance(default_balance),
                            merchant_balance: MerchantBalance::zero(),
                        }),
                    },
                ),
                (
                    Operation::Pay(default_payment),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: into_customer_balance(
                                default_balance - default_payment,
                            ),
                            merchant_balance: into_merchant_balance(default_payment),
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Payment more than the balance fails".to_string(),
            operations: vec![
                (
                    Operation::Establish(default_balance),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: into_customer_balance(default_balance),
                            merchant_balance: MerchantBalance::zero(),
                        }),
                    },
                ),
                (
                    Operation::Pay(default_balance + 1),
                    Outcome {
                        error: Some(Party::ActiveOperation("pay")),
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::PendingPayment,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: into_customer_balance(default_balance),
                            merchant_balance: MerchantBalance::zero(),
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Payment on non-established channel fails".to_string(),
            operations: vec![(
                Operation::Pay(default_payment),
                Outcome {
                    error: Some(Party::ActiveOperation("pay")),
                    channel_outcome: None,
                },
            )],
        },
    ]
}

#[derive(Debug, Error)]
enum TestError {
    #[error("Operation {0:?} not yet implemented")]
    NotImplemented(Operation),

    #[error(
        "The error behavior did not satisfy expected behavior {op:?}. Got
    CUSTOMER WATCHER OUTPUT:
    {customer_errors}
    MERCHANT SERVER OUTPUT:
    {merchant_errors}
    OPERATION OUTPUT:
    {op_error:?}"
    )]
    InvalidErrorBehavior {
        op: Operation,
        customer_errors: String,
        merchant_errors: String,
        op_error: Result<(), anyhow::Error>,
    },

    #[error("After {op:?}, expected customer status {expected:?}, got {actual:?}")]
    InvalidCustomerStatus {
        op: Operation,
        expected: CustomerStatus,
        actual: CustomerStatus,
    },

    #[error("After {op:?}, expected merchant status {expected:?}, got {actual:?}")]
    InvalidMerchantStatus {
        op: Operation,
        expected: MerchantStatus,
        actual: MerchantStatus,
    },

    #[error(
        "After {op:?}, expected customer, merchant balances 
    ({expected_customer:?}, {expected_merchant:?}), got
    ({actual_customer:?}, {actual_merchant:?})"
    )]
    InvalidChannelBalances {
        op: Operation,
        expected_customer: CustomerBalance,
        expected_merchant: MerchantBalance,
        actual_customer: CustomerBalance,
        actual_merchant: MerchantBalance,
    },
}

impl Test {
    async fn execute(
        &self,
        customer_config: &customer::Config,
        merchant_config: &merchant::Config,
    ) -> Result<(), anyhow::Error> {
        for (op, expected_outcome) in &self.operations {
            // Clone inputs. A future refactor should look into having the `Command` trait take
            // these by reference instead.

            let outcome = match op {
                Operation::Establish(amount) => {
                    let formatted_amount = format!("{} XTZ", amount);
                    let est = customer_cli!(
                        Establish,
                        vec![
                            "establish",
                            "zkchannel://localhost",
                            "--label",
                            &self.name,
                            "--deposit",
                            &formatted_amount,
                        ]
                    );
                    est.run(customer_config.clone())
                }
                Operation::Pay(amount) => {
                    let formatted_amount = format!("{} XTZ", amount);
                    let pay = customer_cli!(Pay, vec!["pay", &self.name, &formatted_amount,]);
                    pay.run(customer_config.clone())
                }
                Operation::NoOp => Box::pin(async { Ok(()) }),
                err_op => return Err(TestError::NotImplemented(*err_op).into()),
            }
            .await;

            // Sleep until the servers have finished their thing, approximately
            tokio::time::sleep(op.wait_time()).await;

            // Get error logs for each party - we make the following assumptions:
            // - logs are deleted after each test, so all errors correspond to this test
            // - any Operation that throws an error is the final Operation in the test
            // These mean that any error found in the logs is caused by the current operation
            let customer_errors = get_logs(LogType::Error, Party::CustomerWatcher)?;
            let merchant_errors = get_logs(LogType::Error, Party::MerchantServer)?;

            // Check whether the process errors matched the expectation.
            match (
                &expected_outcome.error,
                &outcome,
                customer_errors.is_empty(),
                merchant_errors.is_empty(),
            ) {
                // No party threw an error
                (None, Ok(_), true, true) => Ok(()),
                // Only the active operation threw an error
                (Some(Party::ActiveOperation(_)), Err(_), true, true) => Ok(()),
                // Only the customer watcher threw an error
                (Some(Party::CustomerWatcher), Ok(_), false, true) => Ok(()),
                //Only the merchant server threw an error
                (Some(Party::CustomerWatcher), Ok(_), true, false) => Ok(()),

                // In any other case, something went wrong. Provide lots of details to diagnose
                _ => Err(TestError::InvalidErrorBehavior {
                    op: *op,
                    customer_errors,
                    merchant_errors,
                    op_error: outcome,
                }),
            }?;

            match expected_outcome.channel_outcome {
                None => return Ok(()),
                Some(ChannelOutcome {
                    customer_status: expected_customer_status,
                    merchant_status: expected_merchant_status,
                    customer_balance: expected_customer_balance,
                    merchant_balance: expected_merchant_balance,
                }) => {
                    // Parse current channel details for customer
                    let customer_detail_json =
                        customer_cli!(Show, vec!["show", &self.name, "--json"])
                            .run(customer_config.clone())
                            .await
                            .context("Failed to show customer channel")?;

                    let customer_channel: customer::zkchannels::PublicChannelDetails =
                        serde_json::from_str(&customer_detail_json)?;

                    // Parse current channel details for merchant
                    let channel_id = &customer_channel.channel_id().to_string();
                    let merchant_details_json =
                        merchant_cli!(Show, vec!["show", channel_id, "--json"])
                            .run(merchant_config.clone())
                            .await
                            .context("Failed to show merchant channel")?;

                    let merchant_channel: merchant::zkchannels::PublicChannelDetails =
                        serde_json::from_str(&merchant_details_json)?;

                    // Check each party's status
                    if customer_channel.status() != expected_customer_status {
                        return Err(TestError::InvalidCustomerStatus {
                            op: *op,
                            expected: expected_customer_status,
                            actual: customer_channel.status(),
                        }
                        .into());
                    }
                    if merchant_channel.status() != expected_merchant_status {
                        return Err(TestError::InvalidMerchantStatus {
                            op: *op,
                            expected: expected_merchant_status,
                            actual: merchant_channel.status(),
                        }
                        .into());
                    }

                    // Check channel balances
                    if customer_channel.customer_balance() != expected_customer_balance
                        || customer_channel.merchant_balance() != expected_merchant_balance
                    {
                        return Err(TestError::InvalidChannelBalances {
                            op: *op,
                            expected_customer: expected_customer_balance,
                            expected_merchant: expected_merchant_balance,
                            actual_customer: customer_channel.customer_balance(),
                            actual_merchant: customer_channel.merchant_balance(),
                        }
                        .into());
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug)]
struct Test {
    pub name: String,
    pub operations: Vec<(Operation, Outcome)>,
}

/// Set of operations that can be executed by the test harness
#[allow(unused)]
#[derive(Debug, Clone, Copy)]
enum Operation {
    Establish(u64),
    Pay(u64),
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
            Self::Establish(_)
            | Self::Pay(_)
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

#[derive(Debug)]
struct Outcome {
    /// Which process, if any, had an error? Assumes that exactly one party will error.
    error: Option<Party>,
    channel_outcome: Option<ChannelOutcome>,
}

#[derive(Debug)]
struct ChannelOutcome {
    customer_status: CustomerStatus,
    merchant_status: MerchantStatus,
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
}

/// Helper function to convert human XTZ amount into a `CustomerBalance`.
fn into_customer_balance(amount: u64) -> CustomerBalance {
    Amount::from_str(&format!("{} XTZ", amount))
        .unwrap()
        .try_into()
        .unwrap()
}

/// Helper function to convert human XTZ amount into a `MerchantBalance`.
#[allow(unused)]
fn into_merchant_balance(amount: u64) -> MerchantBalance {
    Amount::from_str(&format!("{} XTZ", amount))
        .unwrap()
        .try_into()
        .unwrap()
}

#[derive(Debug, Error)]
#[allow(unused)]
pub enum LogError {
    #[error("Failed to open log file: {0}")]
    OpenFailed(std::io::Error),
    #[error("Failed to read contents of file: {0}")]
    ReadFailed(std::io::Error),
}

#[allow(unused)]
#[derive(Debug, Clone, Copy)]
pub enum LogType {
    Info,
    Error,
    Warn,
}

#[allow(unused)]
impl LogType {
    pub fn to_str(&self) -> &str {
        match self {
            LogType::Info => "INFO",
            LogType::Error => "ERROR",
            LogType::Warn => "WARN",
        }
    }
}

/// Get any errors from the log file, filtered by party and log type.
fn get_logs(log_type: LogType, party: Party) -> Result<String, LogError> {
    let mut file = File::open(common::ERROR_FILENAME).map_err(LogError::OpenFailed)?;
    let mut logs = String::new();
    file.read_to_string(&mut logs)
        .map_err(LogError::ReadFailed)?;

    Ok(logs
        .lines()
        .filter(|s| s.contains("zeekoe::"))
        .filter(|s| s.contains(log_type.to_str()))
        .filter(|s| s.contains(party.to_str()))
        .fold("".to_string(), |acc, s| format!("{}{}\n", acc, s)))
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
