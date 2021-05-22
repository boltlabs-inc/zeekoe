use dialectic::prelude::*;
use libzkchannels_toolkit::{nonce::Nonce, proofs::PayProof, revlock::*, states::*};

pub type Ping = Session! {
    recv String;
    send String;
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
        recv CloseStateBlindedSignature;
        ChooseAbort<CustomerRevokePreviousPayToken>;
    };

    pub type CustomerRevokePreviousPayToken = Session! {
        send RevocationLock;
        send RevocationSecret;
        send RevocationLockBlindingFactor;
        OfferAbort<MerchantIssueNewPayToken>;
    };

    pub type MerchantIssueNewPayToken = Session! {
        recv BlindedPayToken;
    };
}
