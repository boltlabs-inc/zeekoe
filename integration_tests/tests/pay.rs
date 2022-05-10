use crate::{
    common::Party,
    tests::{
        to_customer_balance, to_merchant_balance, ChannelOutcome, Operation, Outcome, Test,
        DEFAULT_BALANCE, DEFAULT_PAYMENT,
    },
};
use zeekoe::{
    customer::database::StateName as CustomerStatus, protocol::ChannelStatus as MerchantStatus,
};
use zkabacus_crypto::{CustomerBalance, MerchantBalance};

pub fn tests() -> Vec<Test> {
    vec![
        Test {
            name: "Payment equal to the balance is successful".to_string(),
            operations: vec![
                (
                    Operation::Establish(DEFAULT_BALANCE),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: to_customer_balance(DEFAULT_BALANCE),
                            merchant_balance: MerchantBalance::zero(),
                            closing_balances: None,
                        }),
                    },
                ),
                (
                    Operation::Pay(DEFAULT_BALANCE),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: CustomerBalance::zero(),
                            merchant_balance: to_merchant_balance(DEFAULT_BALANCE),
                            closing_balances: None,
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Payment less than the balance is successful".to_string(),
            operations: vec![
                (
                    Operation::Establish(DEFAULT_BALANCE),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: to_customer_balance(DEFAULT_BALANCE),
                            merchant_balance: MerchantBalance::zero(),
                            closing_balances: None,
                        }),
                    },
                ),
                (
                    Operation::Pay(DEFAULT_PAYMENT),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: to_customer_balance(
                                DEFAULT_BALANCE - DEFAULT_PAYMENT,
                            ),
                            merchant_balance: to_merchant_balance(DEFAULT_PAYMENT),
                            closing_balances: None,
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Payment more than the balance fails".to_string(),
            operations: vec![
                (
                    Operation::Establish(DEFAULT_BALANCE),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: to_customer_balance(DEFAULT_BALANCE),
                            merchant_balance: MerchantBalance::zero(),
                            closing_balances: None,
                        }),
                    },
                ),
                (
                    Operation::Pay(DEFAULT_BALANCE + 1),
                    Outcome {
                        error: Some(Party::ActiveOperation("pay")),
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::PendingPayment,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: to_customer_balance(DEFAULT_BALANCE),
                            merchant_balance: MerchantBalance::zero(),
                            closing_balances: None,
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Payment on non-established channel fails".to_string(),
            operations: vec![(
                Operation::Pay(DEFAULT_PAYMENT),
                Outcome {
                    error: Some(Party::ActiveOperation("pay")),
                    channel_outcome: None,
                },
            )],
        },
    ]
}
