mod establish;
mod mutual_close;
mod pay;

use zeekoe::{
    amount::Amount,
    customer::{self, database::StateName as CustomerStatus, zkchannels::Command},
    database::ClosingBalances,
    merchant::{self, zkchannels::Command as _},
    protocol::ChannelStatus as MerchantStatus,
};
use zkabacus_crypto::{CustomerBalance, MerchantBalance};

use crate::common::{
    customer_cli, merchant_cli, restore_db_state, store_db_state, Party, ERROR_FILENAME,
};
use anyhow::Context;
use std::{convert::TryInto, fs::File, io::Read, panic, str::FromStr, time::Duration};
use structopt::StructOpt;
use thiserror::Error;

/// Default balance for establishing channels
const DEFAULT_BALANCE: u64 = 5;
/// Default payment; should be less than balance
const DEFAULT_PAYMENT: u64 = 1;

/// Struct to represent one test: includes a name and a list of operations and outcomes
#[derive(Debug, Clone)]
pub struct Test {
    pub name: String,
    pub operations: Vec<(Operation, Outcome)>,
}

/// Set of operations that can be executed by the test harness
#[allow(unused)]
#[derive(Debug, Clone, Copy)]
pub enum Operation {
    Establish(u64),
    Pay(u64),
    PayAll,
    MutualClose,
    CustomerClose,
    MerchantExpiry,
    Store(&'static str),
    Restore(&'static str),
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
            | Self::Store(_)
            | Self::Restore(_) => 0,

            // The merchant watcher must notice the contract status change
            Self::MutualClose => 60,
            // After the initial close tx is posted inline, either:
            // - the merchant notices, posts Dispute, and waits for it to confirm. The customer
            //   watcher notices and updates status (180)
            // - the merchant notices and does nothing. The customer watcher waits for self-delay to
            //   elapse, then posts claim (180)
            Self::CustomerClose => 180,
            // Expiry is either:
            // - the same as customer close, but preceded by the customer noticing expiry
            // and posting the corrected balances (+70)
            // - the merchant waits for self-delay to elapse, then claims funds (130)
            Self::MerchantExpiry => 200,
        };
        Duration::from_secs(seconds)
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    /// Which process, if any, had an error? Assumes that exactly one party will error.
    error: Option<Party>,
    /// Outcome of channel; left an Option in case we do not expect a channel from a test
    /// (e.g. interacting with a non-existent channel).
    channel_outcome: Option<ChannelOutcome>,
}

#[derive(Debug, Clone)]
struct ChannelOutcome {
    customer_status: CustomerStatus,
    merchant_status: MerchantStatus,
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
    /// Closing balances of channels, for use in close-related tests
    closing_balances: Option<ClosingBalances>,
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

    #[error(
        "After {op:?}, party: {party:?} expected customer, merchant closing balances
    {expected:?}, got {actual:?}"
    )]
    InvalidClosingBalances {
        op: Operation,
        party: String,
        expected: ClosingBalances,
        actual: ClosingBalances,
    },
}

impl Test {
    pub async fn execute(
        &self,
        customer_config: &customer::Config,
        merchant_config: &merchant::Config,
    ) -> Result<(), anyhow::Error> {
        for (op, expected_outcome) in &self.operations {
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
                Operation::MutualClose => {
                    let mutual_close = customer_cli!(Close, vec!["close", &self.name]);
                    mutual_close.run(customer_config.clone())
                }
                Operation::CustomerClose => {
                    let customer_close = customer_cli!(Close, vec!["close", &self.name, "--force"]);
                    customer_close.run(customer_config.clone())
                }
                Operation::Store(tag) => {
                    store_db_state(tag)?;
                    continue;
                }
                Operation::Restore(tag) => {
                    restore_db_state(tag)?;
                    continue;
                }
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

            let ChannelOutcome {
                customer_status: expected_customer_status,
                merchant_status: expected_merchant_status,
                customer_balance: expected_customer_balance,
                merchant_balance: expected_merchant_balance,
                closing_balances: expected_closing_balances,
            } = match &expected_outcome.channel_outcome {
                None => continue,
                Some(channel_outcome) => channel_outcome,
            };

            // Parse current channel details for customer
            let customer_detail_json = customer_cli!(Show, vec!["show", &self.name, "--json"])
                .run(customer_config.clone())
                .await
                .context("Failed to show customer channel")?;

            let customer_channel: customer::zkchannels::PublicChannelDetails =
                serde_json::from_str(&customer_detail_json)?;

            // Parse current channel details for merchant
            let channel_id = &customer_channel.channel_id().to_string();
            let merchant_details_json = merchant_cli!(Show, vec!["show", channel_id, "--json"])
                .run(merchant_config.clone())
                .await
                .context("Failed to show merchant channel")?;

            let merchant_channel: merchant::zkchannels::PublicChannelDetails =
                serde_json::from_str(&merchant_details_json)?;

            // Check each party's status
            if customer_channel.status() != *expected_customer_status {
                return Err(TestError::InvalidCustomerStatus {
                    op: *op,
                    expected: *expected_customer_status,
                    actual: customer_channel.status(),
                }
                .into());
            }
            if merchant_channel.status() != *expected_merchant_status {
                return Err(TestError::InvalidMerchantStatus {
                    op: *op,
                    expected: *expected_merchant_status,
                    actual: merchant_channel.status(),
                }
                .into());
            }

            // Check channel balances
            if customer_channel.customer_balance() != *expected_customer_balance
                || customer_channel.merchant_balance() != *expected_merchant_balance
            {
                return Err(TestError::InvalidChannelBalances {
                    op: *op,
                    expected_customer: *expected_customer_balance,
                    expected_merchant: *expected_merchant_balance,
                    actual_customer: customer_channel.customer_balance(),
                    actual_merchant: customer_channel.merchant_balance(),
                }
                .into());
            }

            // evaluate whether there should be a closing balance present in outcome
            let expected_closing_balances: ClosingBalances = match &expected_closing_balances {
                None => continue,
                Some(closing_balances) => *closing_balances,
            };

            // check Customer's version of closing balances
            if *customer_channel.closing_balances() != expected_closing_balances {
                return Err(TestError::InvalidClosingBalances {
                    op: *op,
                    party: "Customer".to_string(),
                    expected: expected_closing_balances,
                    actual: *customer_channel.closing_balances(),
                }
                .into());
            }

            // check Merchant's version of closing balances
            if *merchant_channel.closing_balances() != expected_closing_balances {
                return Err(TestError::InvalidClosingBalances {
                    op: *op,
                    party: "Merchant".to_string(),
                    expected: expected_closing_balances,
                    actual: *merchant_channel.closing_balances(),
                }
                .into());
            }
        }

        Ok(())
    }
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
    pub fn to_str(self) -> &'static str {
        match self {
            LogType::Info => "INFO",
            LogType::Error => "ERROR",
            LogType::Warn => "WARN",
        }
    }
}

/// Get any errors from the log file, filtered by party and log type.
pub fn get_logs(log_type: LogType, party: Party) -> Result<String, LogError> {
    let mut file = File::open(ERROR_FILENAME).map_err(LogError::OpenFailed)?;
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

/// Convert human XTZ amount into a `CustomerBalance`.
fn to_customer_balance(amount: u64) -> CustomerBalance {
    Amount::from_str(&format!("{} XTZ", amount))
        .unwrap()
        .try_into()
        .unwrap()
}

/// Convert human XTZ amount into a `MerchantBalance`.
fn to_merchant_balance(amount: u64) -> MerchantBalance {
    Amount::from_str(&format!("{} XTZ", amount))
        .unwrap()
        .try_into()
        .unwrap()
}

/// Get a list of tests to execute.
/// Assumption: none of these will cause a fatal error to the long-running processes (merchant
/// server or customer watcher).
pub fn all_tests() -> Vec<Test> {
    [establish::tests(), pay::tests(), mutual_close::tests()].concat()
}
