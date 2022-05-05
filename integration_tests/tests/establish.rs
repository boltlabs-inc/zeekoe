use crate::{
    common::Party,
    tests::{to_customer_balance, ChannelOutcome, Operation, Outcome, Test},
};
use zeekoe::{
    customer::database::StateName as CustomerStatus, protocol::ChannelStatus as MerchantStatus,
};
use zkabacus_crypto::MerchantBalance;

pub fn tests() -> Vec<Test> {
    let default_balance = 5;
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
                        customer_balance: to_customer_balance(default_balance),
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
                    Operation::Establish(default_balance),
                    Outcome {
                        error: None,
                        channel_outcome: Some(ChannelOutcome {
                            customer_status: CustomerStatus::Ready,
                            merchant_status: MerchantStatus::Active,
                            customer_balance: to_customer_balance(default_balance),
                            merchant_balance: MerchantBalance::zero(),
                            closing_balances: None,
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
                            customer_balance: to_customer_balance(default_balance),
                            merchant_balance: MerchantBalance::zero(),
                            closing_balances: None,
                        }),
                    },
                ),
            ],
        },
    ]
}
