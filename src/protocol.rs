use dialectic::prelude::*;

use zkabacus_crypto::{
    customer, revlock::*, CloseStateCommitment, ClosingSignature, Nonce, PayProof, PayToken,
    StateCommitment,
};

pub type Ping = Session! {
    loop {
        send String;
        recv String;
    }
};

type OfferContinue<Next> = Session! {
    offer {
        0 => {},
        1 => Next,
    }
};

#[macro_export]
macro_rules! offer_continue {
    (in $chan:ident else $err:expr) => {
        dialectic::offer!(in $chan {
            0 => {
                $chan.close();
                return Err($err);
            }
            1 => $chan,
        })
    }
}

type ChooseContinue<Next> = Session! {
    choose {
        0 => {},
        1 => Next,
    }
};

#[macro_export]
macro_rules! choose_abort {
    (in $chan:ident) => {
        match $chan.choose::<0>().await {
            Ok($chan) => Ok($chan.close()),
            Err(e) => Err(e),
        }
    };
}

#[macro_export]
macro_rules! choose_continue {
    (in $chan:ident) => {
        $chan.choose::<1>().await
    };
}

// All protocols are from the perspective of the customer.

pub use parameters::Parameters;
pub use pay::Pay;

pub type ZkChannels = Session! {
    choose {
        0 => Parameters,
        1 => Pay,
    }
};

pub mod parameters {
    use super::*;

    /// Get the public parameters for the merchant.
    pub type Parameters = Session! {
        // TODO: receive on-chain keys here
        recv customer::Config;
    };
}

pub mod pay {
    use super::*;

    /// The full "pay" protocol's session type.
    pub type Pay = Session! {
        send usize;
        send String;
        OfferContinue<CustomerStartPayment>;
    };

    pub type CustomerStartPayment = Session! {
        send Nonce;
        send PayProof;
        send RevocationLockCommitment;
        send CloseStateCommitment;
        send StateCommitment;
        OfferContinue<MerchantAcceptPayment>;
    };

    pub type MerchantAcceptPayment = Session! {
        recv ClosingSignature;
        ChooseContinue<CustomerRevokePreviousPayToken>;
    };

    pub type CustomerRevokePreviousPayToken = Session! {
        send RevocationLock;
        send RevocationSecret;
        send RevocationLockBlindingFactor;
        OfferContinue<MerchantIssueNewPayToken>;
    };

    pub type MerchantIssueNewPayToken = Session! {
        recv PayToken;
    };
}
