use zkabacus_crypto::{
    revlock::*, ClosingSignature, CustomerRandomness, MerchantRandomness, Nonce, PayProof, PayToken,
};
use {
    dialectic::prelude::*,
    serde::{Deserialize, Serialize},
    std::fmt::{self, Display, Formatter},
    thiserror::Error,
};

#[cfg(test)]
use strum_macros::EnumIter;

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

#[derive(Debug, Clone, Copy, PartialEq, sqlx::Type)]
#[cfg_attr(test, derive(EnumIter))]
#[sqlx(rename_all = "snake_case", type_name = "text")]
pub enum ChannelStatus {
    Originated,
    CustomerFunded,
    MerchantFunded,
    Active,
    PendingExpiry,
    PendingClose,
    PendingMutualClose,
    PendingMerchantClaim,
    Dispute,
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
                Self::PendingExpiry => "pending expiry close",
                Self::PendingClose => "pending close",
                Self::PendingMutualClose => "pending mutual close",
                Self::PendingMerchantClaim => "pending merchant claim",
                Self::Dispute => "dispute",
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
    use crate::escrow::types::{TezosFundingAddress, TezosPublicKey};
    use zkabacus_crypto::{CommitmentParameters, PublicKey, RangeConstraintParameters};

    use super::*;

    /// Get the public parameters for the merchant.
    pub type Parameters = Session! {
        recv PublicKey;
        recv CommitmentParameters;
        recv RangeConstraintParameters;
        recv TezosFundingAddress;
        recv TezosPublicKey;
    };
}

pub mod establish {
    use super::*;
    use crate::escrow::types::*;
    use zkabacus_crypto::{
        ClosingSignature, CustomerBalance, EstablishProof, MerchantBalance, PayToken,
    };

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct ContractFunded;

    #[derive(Debug, Clone, Error, Serialize, Deserialize)]
    pub enum Error {
        #[error("Received invalid parameters from merchant")]
        InvalidParameters,
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
        #[error("Could not verify contract's origination on chain")]
        FailedVerifyOrigination,
        #[error("Could not verify contract was funded correctly on chain")]
        FailedVerifyCustomerFunding,
    }

    pub type Establish = CustomerSupplyInfo;

    pub type CustomerSupplyInfo = Session! {
        send CustomerRandomness;
        CustomerProposeFunding;
    };

    pub type CustomerProposeFunding = Session! {
        send CustomerBalance;
        send MerchantBalance;
        // Channel establishment justification note
        send String;
        // Customer's tezos public key (EdDSA public key)
        send TezosPublicKey;
        // Customer's tezos account tz1 address
        send TezosFundingAddress;
        // SHA3-256 of:
        // - merchant's pointcheval-sanders public key (`zkabacus_crypto::PublicKey`)
        // - tz1 address corresponding to merchant's public key
        // - merchant's tezos public key
        send KeyHash;
        MerchantApproveEstablish;
    };

    pub type MerchantApproveEstablish = Session! {
        // Merchant decides if they want to open the channel as described
        OfferAbort<MerchantSupplyInfo, Error>;
    };

    pub type MerchantSupplyInfo = Session! {
        recv MerchantRandomness;
        Initialize;
    };

    pub type Initialize = CustomerSupplyProof;

    pub type CustomerSupplyProof = Session! {
        send EstablishProof;
        // Merchant verifies the proof
        OfferAbort<MerchantSupplyClosingSignature, Error>;
    };

    pub type MerchantSupplyClosingSignature = Session! {
        recv ClosingSignature;
        // Customer verifies the signature
        ChooseAbort<CustomerSupplyContractInfo, Error>;
    };

    pub type CustomerSupplyContractInfo = Session! {
        send ContractId;
        // Merchant ensures the contract was correctly originated
        OfferAbort<MerchantVerifyCustomerFunding, Error>;
    };

    pub type MerchantVerifyCustomerFunding = Session! {
        // Notify the merchant that the customer has funded the contract.
        send ContractFunded;
        // Merchant ensures the contract was correctly funded
        OfferAbort<CustomerVerifyMerchantFunding, Error>;
    };

    pub type CustomerVerifyMerchantFunding = Session! {
        // Notify the customer that the merchant has funded the contract.
        recv ContractFunded;
        // Customer ensures the merchant funded the contract
        ChooseAbort<Activate, Error>;
    };

    pub type Activate = Session! {
        recv PayToken;
    };
}
pub mod close {
    use {
        dialectic::types::Done,
        zkabacus_crypto::{CloseState, CloseStateSignature},
    };

    use crate::{database::customer::StateName, escrow::tezos::MutualCloseAuthorizationSignature};

    use super::*;

    #[derive(Debug, Clone, Serialize, Deserialize, Error)]
    pub enum Error {
        #[error("Customer tried to close on an uncloseable state \"{0}\"")]
        UncloseableState(StateName),
        #[error("Customer sent an invalid signature")]
        InvalidCloseStateSignature,
        #[error("Customer sent a close state that has already been seen")]
        KnownRevocationLock,
        #[error("Merchant send an invalid authorization signature")]
        InvalidMerchantAuthorizationSignature,
        #[error("Arbiter failed to accept mutual close")]
        ArbiterRejectedMutualClose,
    }

    /// Mutual close session.
    pub type Close = CustomerSendSignature;

    pub type CustomerSendSignature = Session! {
        send CloseStateSignature;
        send CloseState;
        // Merchant checks whether the `CloseState` is outdated
        OfferAbort<MerchantSendAuthorization, Error>
    };

    pub type MerchantSendAuthorization = Session! {
        // Tezos authorization signature
        recv MutualCloseAuthorizationSignature;
        // Merchant verifies the signature
        ChooseAbort<Done, Error>
    };
}

pub mod pay {
    use super::*;
    use zkabacus_crypto::{self, PaymentAmount};

    #[derive(Debug, Clone, Serialize, Deserialize, Error)]
    pub enum Error {
        #[error("Payment rejected: {0}")]
        Rejected(String),
        #[error("Customer failed to generate nonce and pay proof: {0}")]
        StartFailed(#[from] zkabacus_crypto::Error),
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
        GetPaymentApproval;
    };

    pub type GetPaymentApproval = Session! {
        // Merchant decides if it wants to allow the described payment
        OfferAbort<CustomerStartPayment, Error>;
    };

    /// The start of the zkabacus "pay" protocol.
    pub type CustomerStartPayment = Session! {
        send Nonce;
        send PayProof;
        // Merchant checks that the `PayProof` is valid
        OfferAbort<MerchantAcceptPayment, Error>;
    };

    pub type MerchantAcceptPayment = Session! {
        recv ClosingSignature;
        // Customer verifies the signature
        ChooseAbort<CustomerRevokePreviousPayToken, Error>;
    };

    pub type CustomerRevokePreviousPayToken = Session! {
        send RevocationLock;
        send RevocationSecret;
        send RevocationLockBlindingFactor;
        // Merchant verifies that the revocation information is valid
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

pub mod daemon {
    use super::*;
    use dialectic::types::Done;

    pub type Daemon = Session! {
        choose {
            // Refresh
            0 => Done,
        }
    };
}
