use zkabacus_crypto::{
    revlock::*, ClosingSignature, CommitmentParameters, CustomerRandomness, MerchantRandomness,
    Nonce, PayProof, PayToken, PublicKey, RangeProofParameters,
};
use {
    dialectic::prelude::*,
    serde::{Deserialize, Serialize},
    std::fmt::{self, Display, Formatter},
    thiserror::Error,
};

pub type Ping = Session! {
    loop {
        send String;
        recv String;
    }
};

type OfferAbort<Next, Err> = Session! {
    offer {
        0 => recv Err,
        1 => Next,
    }
};

#[macro_export]
macro_rules! offer_abort {
    (in $chan:ident as $party:expr) => {
        let $chan = ::anyhow::Context::context(dialectic::offer!(in $chan {
            0 => {
                let party_ctx = || format!("{:?} chose to abort the session", $party.opposite());
                let (err, $chan) = ::anyhow::Context::with_context(
                    ::anyhow::Context::context(
                        $chan.recv().await,
                        "Failed to receive error after receiving abort"
                    ),
                    party_ctx)?;
                $chan.close();
                return ::anyhow::Context::with_context(Err(err), party_ctx);
            }
            1 => $chan,
        }), "Failure while receiving choice of continue/abort")?;
    }
}

type ChooseAbort<Next, Err> = Session! {
    choose {
        0 => send Err,
        1 => Next,
    }
};

#[macro_export]
macro_rules! abort {
    (in $chan:ident return $err:expr ) => {{
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
        return ::anyhow::Context::context(Err(err), "Pay protocol aborted");
    }};
}

#[macro_export]
macro_rules! proceed {
    (in $chan:ident) => {
        let $chan = ::anyhow::Context::context(
            $chan.choose::<1>().await,
            "Failure while choosing to continue",
        )?;
    };
}

/// The two parties in the protocol.
#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
pub enum Party {
    /// The customer client.
    Customer,
    /// The merchant server.
    Merchant,
}

impl Display for Party {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        use Party::*;
        write!(
            f,
            "{}",
            match self {
                Customer => "customer",
                Merchant => "merchant",
            }
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[sqlx(rename_all = "snake_case", type_name = "text")]
pub enum ChannelStatus {
    Originated,
    CustomerFunded,
    MerchantFunded,
    Active,
    Closed,
}

impl Party {
    /// Get the other party.
    ///
    /// # Examples
    ///
    /// ```
    /// use zeekoe::protocol::Party::*;
    ///
    /// assert_eq!(Customer.opposite(), Merchant);
    /// assert_eq!(Merchant.opposite(), Customer);
    /// ```
    pub const fn opposite(self) -> Self {
        use Party::*;
        match self {
            Customer => Merchant,
            Merchant => Customer,
        }
    }
}

// All protocols are from the perspective of the customer.

pub use establish::Establish;
pub use parameters::Parameters;
pub use pay::Pay;

pub type ZkChannels = Session! {
    choose {
        0 => Parameters,
        1 => Pay,
        2 => Establish,
    }
};

pub mod parameters {
    use super::*;

    /// Get the public parameters for the merchant.
    pub type Parameters = Session! {
        recv PublicKey;
        recv CommitmentParameters;
        recv RangeProofParameters;
    };
}

pub mod establish {
    use super::*;
    use zkabacus_crypto::{
        ClosingSignature, CustomerBalance, EstablishProof, MerchantBalance, PayToken,
    };

    #[derive(Debug, Clone, Error, Serialize, Deserialize)]
    pub enum Error {
        #[error("Invalid {0} deposit amount")]
        InvalidDeposit(Party),
        #[error("Channel funding request rejected: {0}")]
        Rejected(String),
        #[error("Invalid channel establish proof")]
        InvalidEstablishProof,
        #[error("Invalid closing signature")]
        InvalidClosingSignature,
        #[error("Invalid payment token")]
        InvalidPayToken,
        #[error("Merchant funding not received")]
        FailedMerchantFunding,
    }

    pub type Establish = CustomerSupplyInfo;

    pub type CustomerSupplyInfo = Session! {
        // TODO: send customer-side chain-specific public stuff
        send CustomerRandomness;
        CustomerProposeFunding;
    };

    pub type CustomerProposeFunding = Session! {
        send CustomerBalance;
        send MerchantBalance;
        send String;
        OfferAbort<MerchantSupplyInfo, Error>;
    };

    pub type MerchantSupplyInfo = Session! {
        // TODO: recv merchant-side chain-specific public stuff
        recv MerchantRandomness;
        Initialize;
    };

    pub type Initialize = CustomerSupplyProof;

    pub type CustomerSupplyProof = Session! {
        send EstablishProof;
        OfferAbort<MerchantSupplyClosingSignature, Error>;
    };

    pub type MerchantSupplyClosingSignature = Session! {
        recv ClosingSignature;
        ChooseAbort<CustomerSupplyContractInfo, Error>;
    };

    pub type CustomerSupplyContractInfo = Session! {
        // TODO: send contract id
        OfferAbort<CustomerVerifyMerchantFunding, Error>;
    };

    pub type CustomerVerifyMerchantFunding = Session! {
        ChooseAbort<Activate, Error>;
    };

    pub type Activate = Session! {
        recv PayToken;
    };
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
        send String; // Payment note
        OfferAbort<CustomerStartPayment, Error>;
    };

    /// The start of the zkabacus "pay" protocol.
    pub type CustomerStartPayment = Session! {
        send Nonce;
        send PayProof;
        OfferAbort<MerchantAcceptPayment, Error>;
    };

    pub type MerchantAcceptPayment = Session! {
        recv ClosingSignature;
        ChooseAbort<CustomerRevokePreviousPayToken, Error>;
    };

    pub type CustomerRevokePreviousPayToken = Session! {
        send RevocationLock;
        send RevocationSecret;
        send RevocationLockBlindingFactor;
        OfferAbort<MerchantIssueNewPayToken, Error>;
    };

    pub type MerchantIssueNewPayToken = Session! {
        recv PayToken;
        MerchantProvideService;
    };

    pub type MerchantProvideService = Session! {
        recv Option<String>;
    };
}
