use zkabacus_crypto::{revlock::*, ClosingSignature, Nonce, PayProof, PayToken};
use {
    dialectic::prelude::*,
    serde::{Deserialize, Serialize},
    thiserror::Error,
};

pub type Ping = Session! {
    loop {
        send String;
        recv String;
    }
};

type OfferContinue<Next, Err> = Session! {
    offer {
        0 => recv Err,
        1 => Next,
    }
};

#[macro_export]
macro_rules! offer_abort {
    (in $chan:ident) => {
        ::anyhow::Context::context(dialectic::offer!(in $chan {
            0 => {
                let (err, $chan) = ::anyhow::Context::context($chan.recv().await, "Failed to receive error after offering abort")?;
                $chan.close();
                return Err(err.into());
            }
            1 => $chan,
        }), "Failure while receiving choice of continue/abort")?
    }
}

type ChooseContinue<Next, Err> = Session! {
    choose {
        0 => send Err,
        1 => Next,
    }
};

#[macro_export]
macro_rules! choose_abort {
    (in $chan:ident return $err:expr ) => {
        let $chan = ::anyhow::Context::context(
            $chan.choose::<0>().await,
            "Failure while choosing to abort",
        )?;
        let err = $err;
        let $chan = ::anyhow::Context::context(
            $chan.send(err.clone()).await,
            "Failed to send error after choosing to abort",
        )?;
        $chan.close();
        return Err(err.into());
    };
}

#[macro_export]
macro_rules! choose_continue {
    (in $chan:ident) => {
        ::anyhow::Context::context(
            $chan.choose::<1>().await,
            "Failure while choosing to continue",
        )?
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
    pub type Parameters = Session! {};
}

pub mod pay {
    use super::*;
    use zkabacus_crypto::PaymentAmount;

    #[derive(Debug, Clone, Serialize, Deserialize, Error)]
    pub enum Error {
        #[error("Payment rejected: {0}")]
        Rejected(String),
        #[error("Customer submitted reused nonce")]
        ReusedNonce,
        #[error("Merchant returned invalid closing signature")]
        InvalidClosingSignature,
        #[error("Customer submitted reused revocation lock")]
        ReusedRevocationLock,
        #[error("Customer submitted invalid opening of commitments")]
        InvalidRevocationOpening,
        #[error("Customer submitted invalid payment proof")]
        InvalidPayProof,
        #[error("Channel frozen: merchant returned invalid payment token")]
        InvalidPayToken,
    }

    /// The full zkchannels "pay" protocol's session type.
    pub type Pay = Session! {
        send PaymentAmount;
        send String;
        OfferContinue<CustomerStartPayment, Error>;
    };

    /// The start of the zkabacus "pay" protocol.
    pub type CustomerStartPayment = Session! {
        send Nonce;
        send PayProof;
        OfferContinue<MerchantAcceptPayment, Error>;
    };

    pub type MerchantAcceptPayment = Session! {
        recv ClosingSignature;
        ChooseContinue<CustomerRevokePreviousPayToken, Error>;
    };

    pub type CustomerRevokePreviousPayToken = Session! {
        send RevocationLock;
        send RevocationSecret;
        send RevocationLockBlindingFactor;
        OfferContinue<MerchantIssueNewPayToken, Error>;
    };

    pub type MerchantIssueNewPayToken = Session! {
        recv PayToken;
        recv Option<String>;
    };
}
