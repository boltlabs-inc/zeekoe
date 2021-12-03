use {
    serde::{Deserialize, Serialize},
    std::fmt::{Display, Formatter},
    thiserror::Error,
};

use zkabacus_crypto::{
    customer as zkabacus, impl_sqlx_for_bincode_ty, ChannelId, CustomerBalance, MerchantBalance,
};

/// The current state of the channel, from the perspective of the customer.
///
/// This enumeration only includes states that are persisted to the database.
#[derive(Debug, Serialize, Deserialize)]
pub enum State {
    /// Funding approved but channel is not yet active.
    Inactive(zkabacus::Inactive),
    /// Channel has an originated contract but is not funded.
    Originated(zkabacus::Inactive),
    /// Channel has a customer-funded contract but has not received merchant funding.
    CustomerFunded(zkabacus::Inactive),
    /// Channel has received all funding but is not yet active.
    MerchantFunded(zkabacus::Inactive),
    /// Channel is ready for payment.
    Ready(zkabacus::Ready),
    /// Channel has started Pay.
    PendingPayment(zkabacus::Ready),
    /// Payment has been started, which means customer can close on new or old balance.
    Started(zkabacus::Started),
    /// There was an unrecoverable error during the [`State::Started`] state.
    StartedFailed(zkabacus::Started),
    /// Customer has revoked their ability to close on the old balance, but has not yet received
    /// the ability to make a new payment.
    Locked(zkabacus::Locked),
    /// There was an unrecoverable error during the [`State::Locked`] state.
    LockedFailed(zkabacus::Locked),
    /// The customer initiated a mutual close procedure.
    PendingMutualClose(zkabacus::ClosingMessage),
    /// The merchant posted a request to close the channel, claiming all the balances, and the
    /// customer will not post updated balances.
    ///
    /// Note: this [`ClosingMessage`](zkabacus::ClosingMessage) indicates the channel state known
    /// to the customer at the time the merchant's request was posted.
    PendingExpiry(zkabacus::ClosingMessage),
    /// A party has initiated a unilateral close.
    PendingClose(zkabacus::ClosingMessage),
    /// The customer posted a claim to their funds, but the transfer is not yet complete.
    PendingCustomerClaim(zkabacus::ClosingMessage),
    /// Merchant posted evidence that disputes the close balances proposed by the customer.
    ///
    /// Note: this [`ClosingMessage`](zkabacus::ClosingMessage) indicates the
    /// disputed channel state proposed by the customer.
    Dispute(zkabacus::ClosingMessage),
    /// Channel has been closed on chain: the total balance that can be claimed by the customer
    /// has been claimed and confirmed.
    ///
    /// Note: this [`ClosingMessage`](zkabacus::ClosingMessage) indicates the channel state as
    /// proposed by the customer, which may be different from the final balances.
    Closed(zkabacus::ClosingMessage),
}

/// The set of zkAbacus states that are associated with at least one channel status.
pub trait IsZkAbacusState: Sized {}

impl IsZkAbacusState for zkabacus::Inactive {}
impl IsZkAbacusState for zkabacus::Ready {}
impl IsZkAbacusState for zkabacus::Started {}
impl IsZkAbacusState for zkabacus::Locked {}
impl IsZkAbacusState for zkabacus::ClosingMessage {}

impl_sqlx_for_bincode_ty!(State);

pub mod zkchannels_state {
    //! Individual structs that compose the ZkChannel statuses and conversion functions to
    //! unambiguously retrieve channel states from the database.

    use super::{IsZkAbacusState, State, StateName, UnexpectedState};
    use zkabacus_crypto::customer as zkabacus;

    /// The set of states that a zkChannel can be in.
    pub trait ZkChannelState {
        type ZkAbacusState: IsZkAbacusState;

        /// Retrieve the zkAbacus state from a [`State`] variant. Fails if the `State` variant
        /// does not match `Self`.
        fn zkabacus_state(channel_state: State) -> Result<Self::ZkAbacusState, UnexpectedState>;

        /// Indicate whether the [`State`] variant matches `Self`.
        fn matches(self, channel_state: &State) -> bool;
    }

    /// Implement the [`ZkChannelState`] trait.
    /// Links the state struct, [`State`] variant, [`StateName`] variant, and zkAbacus data.
    macro_rules! impl_zkchannel_state {
        ($state:ident, $zkabacus:ident) => {
            pub struct $state;

            impl ZkChannelState for $state {
                type ZkAbacusState = zkabacus::$zkabacus;

                fn zkabacus_state(
                    channel_state: State,
                ) -> Result<Self::ZkAbacusState, UnexpectedState> {
                    match channel_state {
                        State::$state(inner) => Ok(inner),
                        wrong_state => Err(UnexpectedState {
                            expected_state: StateName::$state,
                            actual_state: wrong_state.state_name(),
                        }),
                    }
                }

                fn matches(self, channel_state: &State) -> bool {
                    if let State::$state(_) = channel_state {
                        true
                    } else {
                        false
                    }
                }
            }
        };
    }

    impl_zkchannel_state!(Inactive, Inactive);
    impl_zkchannel_state!(Originated, Inactive);
    impl_zkchannel_state!(CustomerFunded, Inactive);
    impl_zkchannel_state!(MerchantFunded, Inactive);
    impl_zkchannel_state!(Ready, Ready);
    impl_zkchannel_state!(PendingPayment, Ready);
    impl_zkchannel_state!(Started, Started);
    impl_zkchannel_state!(StartedFailed, Started);
    impl_zkchannel_state!(Locked, Locked);
    impl_zkchannel_state!(LockedFailed, Locked);
    impl_zkchannel_state!(PendingMutualClose, ClosingMessage);
    impl_zkchannel_state!(PendingExpiry, ClosingMessage);
    impl_zkchannel_state!(PendingClose, ClosingMessage);
    impl_zkchannel_state!(PendingCustomerClaim, ClosingMessage);
    impl_zkchannel_state!(Dispute, ClosingMessage);
    impl_zkchannel_state!(Closed, ClosingMessage);
}

/// The names of the different states a channel can be in (does not contain actual state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateName {
    Inactive,
    Originated,
    CustomerFunded,
    MerchantFunded,
    Ready,
    PendingPayment,
    Started,
    StartedFailed,
    Locked,
    LockedFailed,
    PendingMutualClose,
    PendingExpiry,
    PendingClose,
    PendingCustomerClaim,
    Dispute,
    Closed,
}

impl Display for StateName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StateName::Inactive => "inactive",
            StateName::Originated => "originated",
            StateName::CustomerFunded => "customer funded",
            StateName::MerchantFunded => "merchant funded",
            StateName::Ready => "ready",
            StateName::PendingPayment => "pending payment",
            StateName::Started => "started",
            StateName::StartedFailed => "started error",
            StateName::Locked => "locked",
            StateName::LockedFailed => "locked error",
            StateName::PendingMutualClose => "pending mutual close",
            StateName::PendingExpiry => "pending expiry",
            StateName::PendingClose => "pending close",
            StateName::PendingCustomerClaim => "pending customer claim",
            StateName::Dispute => "disputed",
            StateName::Closed => "closed",
        }
        .fmt(f)
    }
}

impl State {
    /// Get the name of this state.
    pub fn state_name(&self) -> StateName {
        match self {
            State::Inactive(_) => StateName::Inactive,
            State::Originated(_) => StateName::Originated,
            State::CustomerFunded(_) => StateName::CustomerFunded,
            State::MerchantFunded(_) => StateName::MerchantFunded,
            State::Ready(_) => StateName::Ready,
            State::PendingPayment(_) => StateName::PendingPayment,
            State::Started(_) => StateName::Started,
            State::StartedFailed(_) => StateName::StartedFailed,
            State::Locked(_) => StateName::Locked,
            State::LockedFailed(_) => StateName::LockedFailed,
            State::PendingMutualClose(_) => StateName::PendingMutualClose,
            State::PendingExpiry(_) => StateName::PendingExpiry,
            State::PendingClose(_) => StateName::PendingClose,
            State::PendingCustomerClaim(_) => StateName::PendingCustomerClaim,
            State::Dispute(_) => StateName::Dispute,
            State::Closed(_) => StateName::Closed,
        }
    }

    /// Get the current [`CustomerBalance`] of this state.
    pub fn customer_balance(&self) -> CustomerBalance {
        *match self {
            State::Inactive(inactive) => inactive.customer_balance(),
            State::Originated(inactive) => inactive.customer_balance(),
            State::CustomerFunded(inactive) => inactive.customer_balance(),
            State::MerchantFunded(inactive) => inactive.customer_balance(),
            State::Ready(ready) => ready.customer_balance(),
            State::PendingPayment(ready) => ready.customer_balance(),
            State::Started(started) => started.customer_balance(),
            State::StartedFailed(started) => started.customer_balance(),
            State::Locked(locked) => locked.customer_balance(),
            State::LockedFailed(locked) => locked.customer_balance(),
            State::PendingMutualClose(closing_message) => closing_message.customer_balance(),
            State::PendingExpiry(closing_message) => closing_message.customer_balance(),
            State::PendingClose(closing_message) => closing_message.customer_balance(),
            State::PendingCustomerClaim(closing_message) => closing_message.customer_balance(),
            State::Dispute(closing_message) => closing_message.customer_balance(),
            State::Closed(closed) => closed.customer_balance(),
        }
    }

    pub fn merchant_balance(&self) -> MerchantBalance {
        *match self {
            State::Inactive(inactive) => inactive.merchant_balance(),
            State::Originated(inactive) => inactive.merchant_balance(),
            State::CustomerFunded(inactive) => inactive.merchant_balance(),
            State::MerchantFunded(inactive) => inactive.merchant_balance(),
            State::Ready(ready) => ready.merchant_balance(),
            State::PendingPayment(ready) => ready.merchant_balance(),
            State::Started(started) => started.merchant_balance(),
            State::StartedFailed(started) => started.merchant_balance(),
            State::Locked(locked) => locked.merchant_balance(),
            State::LockedFailed(locked) => locked.merchant_balance(),
            State::PendingMutualClose(closing_message) => closing_message.merchant_balance(),
            State::PendingExpiry(closing_message) => closing_message.merchant_balance(),
            State::PendingClose(closing_message) => closing_message.merchant_balance(),
            State::PendingCustomerClaim(closing_message) => closing_message.merchant_balance(),
            State::Dispute(closing_message) => closing_message.merchant_balance(),
            State::Closed(closed) => closed.merchant_balance(),
        }
    }

    pub fn channel_id(&self) -> &ChannelId {
        match self {
            State::Inactive(inactive) => inactive.channel_id(),
            State::Originated(inactive) => inactive.channel_id(),
            State::MerchantFunded(inactive) => inactive.channel_id(),
            State::CustomerFunded(inactive) => inactive.channel_id(),
            State::Ready(ready) => ready.channel_id(),
            State::PendingPayment(ready) => ready.channel_id(),
            State::Started(started) => started.channel_id(),
            State::StartedFailed(started) => started.channel_id(),
            State::Locked(locked) => locked.channel_id(),
            State::LockedFailed(locked) => locked.channel_id(),
            State::PendingMutualClose(closing_message) => closing_message.channel_id(),
            State::PendingExpiry(closing_message) => closing_message.channel_id(),
            State::PendingClose(closing_message) => closing_message.channel_id(),
            State::PendingCustomerClaim(closing_message) => closing_message.channel_id(),
            State::Dispute(closing_message) => closing_message.channel_id(),
            State::Closed(closed) => closed.channel_id(),
        }
    }
}

/// Error thrown when an operation requires a channel to be in a particular state, but it is in a
/// different one instead.
#[derive(Debug, Serialize, Deserialize, Error)]
#[error("Expected channel in {expected_state} state, but it was in {actual_state} state")]
pub struct UnexpectedState {
    expected_state: StateName,
    actual_state: StateName,
}
