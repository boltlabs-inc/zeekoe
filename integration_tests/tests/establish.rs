use crate::{
    common::Party,
    tests::{to_customer_balance, ChannelOutcome, Operation, Outcome, Test, DEFAULT_BALANCE},
};
use zeekoe::{
    customer::database::StateName as CustomerStatus, protocol::ChannelStatus as MerchantStatus,
};
use zkabacus_crypto::MerchantBalance;

pub fn tests() -> Vec<Test> {
    vec![
        Test {
            name: "Channel establishes correctly".to_string(),
            operations: vec![(
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
            )],
        },
        Test {
            name: "Channels cannot share names".to_string(),
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
                    Operation::Establish(DEFAULT_BALANCE),
                    Outcome {
                        error: Some(Party::ActiveOperation("establish")),
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: to_customer_balance(DEFAULT_BALANCE),
                            merchant_balance: MerchantBalance::zero(),
                            closing_balances: None,
                        }),
                    },
                ),
            ],
        },
    ]
}
