use zkabacus_crypto::{
    revlock::*, ClosingSignature, CustomerRandomness, MerchantRandomness, Nonce, PayProof, PayToken,
};
use {
    dialectic::prelude::*,
    serde::{Deserialize, Serialize},
    std::fmt::{self, Display, Formatter},
    thiserror::Error,
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
        return ::anyhow::Context::context(Err(err), "Protocol aborted");
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContractId {/* TODO */}
zkabacus_crypto::impl_sqlx_for_bincode_ty!(ContractId);

impl Display for ContractId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // TODO: Fill in with actual contract ID
        std::fmt::Debug::fmt(self, f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[sqlx(rename_all = "snake_case", type_name = "text")]
pub enum ChannelStatus {
    Originated,
    CustomerFunded,
    MerchantFunded,
    Active,
    PendingClose,
    Closed,
}

impl Display for ChannelStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Originated => "originated",
                Self::CustomerFunded => "customer funded",
                Self::MerchantFunded => "merchant and customer funded",
                Self::Active => "active",
                Self::PendingClose => "pending close",
                Self::Closed => "closed",
            }
        )
    }
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

pub use close::Close;
pub use establish::Establish;
pub use parameters::Parameters;
pub use pay::Pay;

pub type ZkChannels = Session! {
    choose {
        0 => Parameters,
        1 => Establish,
        2 => Pay,
        3 => Close,
    }
};

pub mod parameters {
    use zkabacus_crypto::{CommitmentParameters, PublicKey, RangeProofParameters};

    use super::*;

    /// Get the public parameters for the merchant.
    pub type Parameters = Session! {
        recv PublicKey;
        recv CommitmentParameters; // TODO: this is a global default, does not need to be sent
        recv RangeProofParameters;
        // TODO: tz1 address corresponding to merchant's public key
        // TODO: merchant's tezos eddsa public key
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
        send CustomerRandomness;
        CustomerProposeFunding;
    };

    pub type CustomerProposeFunding = Session! {
        send CustomerBalance;
        send MerchantBalance;
        send String; // Channel establishment justification note
        // TODO: customer sends merchant:
        // - customer's tezos public key (eddsa public key)
        // - customer's tezos account tz1 address corresponding to that public key
        // - SHA3-256 of:
        //   * merchant's pointcheval-sanders public key (`zkabacus_crypto::PublicKey`)
        //   * tz1 address corresponding to merchant's public key
        //   * merchant's tezos public key
        MerchantApproveEstablish;
    };

    pub type MerchantApproveEstablish = Session! {
        OfferAbort<MerchantSupplyInfo, Error>;
    };

    pub type MerchantSupplyInfo = Session! {
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
pub mod close {
    use dialectic::types::Done;
    use zkabacus_crypto::{CloseState, CloseStateSignature};

    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize, Error)]
    pub enum Error {
        #[error("Customer sent an invalid signature")]
        InvalidCloseStateSignature,
        #[error("Customer sent a close state that has already been seen")]
        KnownRevocationLock,
        #[error("Merchant send an invalid authorization signature")]
        InvalidMerchantAuthSignature,
        #[error("Arbiter failed to accept mutual close")]
        ArbiterRejectedMutualClose,
    }

    /// Mutual close session.
    pub type Close = CustomerSendSignature;

    pub type CustomerSendSignature = Session! {
        send CloseStateSignature;
        send CloseState;
        OfferAbort<MerchantSendAuthorization, Error>
    };

    pub type MerchantSendAuthorization = Session! {
       // TODO: Send auth signature from tezos.
       ChooseAbort<Done, Error>
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
