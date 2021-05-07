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

mod pay {
    use super::*;

    /// The full "pay" protocol's session type.
    pub type Pay = CustomerInit;

    type CustomerInit = Session! {
        send Nonce;
        send PayProof;
        send RevocationLockCommitment;
        send CloseStateCommitment;
        send StateCommitment;
        OfferAbort<MerchantValidate>;
    };

    type MerchantValidate = Session! {
        recv CloseStateBlindedSignature;
        ChooseAbort<CustomerRevoke>;
    };

    type CustomerRevoke = Session! {
        send RevocationLock;
        send RevocationSecret;
        send RevocationLockCommitmentRandomness;
        OfferAbort<MerchantApprove>;
    };

    type MerchantApprove = Session! {
        recv BlindedPayToken;
    };
}
