use dialectic::prelude::*;
use zkabacus_crypto::{
    ClosingSignature, CloseStateCommitment, Nonce, PayProof,
    PayToken, StateCommitment, revlock::*,
};

pub type Ping = Session! {
    loop {
        send String;
        recv String;
    }
};

type OfferAbort<Next> = Session! {
    offer {
        0 => {},
        1 => Next,
    }
};

type ChooseAbort<Next> = Session! {
    choose {
        0 => {},
        1 => Next,
    }
};

// All protocols are from the perspective of the customer.

pub use pay::Pay;

pub mod pay {
    use super::*;

    /// The full "pay" protocol's session type.
    pub type Pay = CustomerStartPayment;

    pub type CustomerStartPayment = Session! {
        send Nonce;
        send PayProof;
        send RevocationLockCommitment;
        send CloseStateCommitment;
        send StateCommitment;
        OfferAbort<MerchantAcceptPayment>;
    };

    pub type MerchantAcceptPayment = Session! {
        recv ClosingSignature;
        ChooseAbort<CustomerRevokePreviousPayToken>;
    };

    pub type CustomerRevokePreviousPayToken = Session! {
        send RevocationLock;
        send RevocationSecret;
        send RevocationLockBlindingFactor;
        OfferAbort<MerchantIssueNewPayToken>;
    };

    pub type MerchantIssueNewPayToken = Session! {
        recv PayToken;
    };
}
