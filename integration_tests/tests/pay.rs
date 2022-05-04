use crate::{
    common::Party,
    tests::{to_customer_balance, to_merchant_balance, ChannelOutcome, Operation, Outcome, Test},
};
use zeekoe::{
    customer::database::StateName as CustomerStatus, protocol::ChannelStatus as MerchantStatus,
};
use zkabacus_crypto::{CustomerBalance, MerchantBalance};

pub fn tests() -> Vec<Test> {
    let default_balance = 5;
    let default_payment = 1;
    vec![
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
                            customer_balance: to_customer_balance(default_balance),
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
                            merchant_balance: to_merchant_balance(default_balance),
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
                            customer_balance: to_customer_balance(default_balance),
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
                            customer_balance: to_customer_balance(
                                default_balance - default_payment,
                            ),
                            merchant_balance: to_merchant_balance(default_payment),
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
                            customer_balance: to_customer_balance(default_balance),
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
                            customer_balance: to_customer_balance(default_balance),
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
