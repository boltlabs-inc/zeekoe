use crate::{
    common::Party,
    tests::{
        to_customer_balance, to_merchant_balance, ChannelOutcome, Operation, Outcome, Test,
        DEFAULT_BALANCE, DEFAULT_PAYMENT,
    },
};
use zeekoe::{
    customer::database::{StateName as CustomerStatus},
    database::ClosingBalances,
    protocol::ChannelStatus as MerchantStatus,
};
use zkabacus_crypto::{CustomerBalance, MerchantBalance};

pub fn tests() -> Vec<Test> {
    vec![
        Test {
            name: "Mutual close where customer has full balance is successful".to_string(),
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
                    Operation::MutualClose,
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Closed,
                            merchant_status: MerchantStatus::Closed,
                            customer_balance: to_customer_balance(DEFAULT_BALANCE),
                            merchant_balance: MerchantBalance::zero(),
                            closing_balances: Some(ClosingBalances {
                                merchant_balance: Some(MerchantBalance::zero()),
                                customer_balance: Some(to_customer_balance(DEFAULT_BALANCE)),
                            }),
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Mutual close where merchant has full balance is successful".to_string(),
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
                (
                    Operation::MutualClose,
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Closed,
                            merchant_status: MerchantStatus::Closed,
                            customer_balance: CustomerBalance::zero(),
                            merchant_balance: to_merchant_balance(DEFAULT_BALANCE),
                            closing_balances: Some(ClosingBalances {
                                merchant_balance: Some(to_merchant_balance(DEFAULT_BALANCE)),
                                customer_balance: Some(CustomerBalance::zero()),
                            }),
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Mutual close where both parties have some balance is successful".to_string(),
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
                (
                    Operation::MutualClose,
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Closed,
                            merchant_status: MerchantStatus::Closed,
                            customer_balance: to_customer_balance(
                                DEFAULT_BALANCE - DEFAULT_PAYMENT,
                            ),
                            merchant_balance: to_merchant_balance(DEFAULT_PAYMENT),
                            closing_balances: Some(ClosingBalances {
                                merchant_balance: Some(to_merchant_balance(DEFAULT_PAYMENT)),
                                customer_balance: Some(to_customer_balance(
                                    DEFAULT_BALANCE - DEFAULT_PAYMENT,
                                )),
                            }),
                        }),
                    },
                ),
            ],
        },
        Test {
            name: "Mutual close on non-established channel fails".to_string(),
            operations: vec![(
                Operation::MutualClose,
                Outcome {
                    error: Some(Party::ActiveOperation("close")),
                    channel_outcome: None,
                },
            )],
        },
    ]
}
