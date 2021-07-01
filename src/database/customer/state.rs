use {
    serde::{Deserialize, Serialize},
    std::{
        convert::TryFrom,
        fmt::{Display, Formatter},
    },
    thiserror::Error,
};

use zkabacus_crypto::{
    customer::{ClosingMessage, Inactive, Locked, Ready, Started},
    impl_sqlx_for_bincode_ty, ChannelId, CustomerBalance, MerchantBalance,
};

/// The current state of the channel, from the perspective of the customer.
///
/// This enumeration only includes states that are persisted to the database.
#[derive(Debug, Serialize, Deserialize)]
pub enum State {
    /// Funding approved but channel is not yet active.
    Inactive(Inactive),
    /// Channel is ready for payment.
    Ready(Ready),
    /// Payment has been started, which means customer can close on new or old balance.
    Started(Started),
    /// Customer has revoked their ability to close on the old balance, but has not yet received the
    /// ability to make a new payment.
    Locked(Locked),
    /// Channel has to be closed because of an error, but has not yet been closed.
    PendingClose(ClosingMessage),
    /// Channel has been closed on chain.
    Closed(Closed),
}

impl_sqlx_for_bincode_ty!(State);

/// The final balances of a channel closed on chain.
#[derive(Debug, Serialize, Deserialize)]
pub struct Closed {
    channel_id: ChannelId,
    customer_balance: CustomerBalance,
    merchant_balance: MerchantBalance,
}

impl Closed {
    /// Create a new [`Closed`] state given balances.
    pub fn new(
        channel_id: ChannelId,
        customer_balance: CustomerBalance,
        merchant_balance: MerchantBalance,
    ) -> Self {
        Closed {
            channel_id,
            customer_balance,
            merchant_balance,
        }
    }

    /// Get the final [`CustomerBalance`] for this closed channel state.
    pub fn customer_balance(&self) -> &CustomerBalance {
        &self.customer_balance
    }

    /// Get the final [`MerchantBalance`] for this closed channel state.
    pub fn merchant_balance(&self) -> &MerchantBalance {
        &self.merchant_balance
    }

    /// Get the [`ChannelId`] for this closed channel state.
    pub fn channel_id(&self) -> &ChannelId {
        &self.channel_id
    }
}

/// The names of the different states a channel can be in (does not contain actual state).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum StateName {
    Inactive,
    Ready,
    Started,
    Locked,
    PendingClose,
    Closed,
}

impl Display for StateName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            StateName::Inactive => "inactive",
            StateName::Ready => "ready",
            StateName::Started => "started",
            StateName::Locked => "locked",
            StateName::PendingClose => "pending close",
            StateName::Closed => "closed",
        }
        .fmt(f)
    }
}

/// The set of states which have a name.
pub trait IsState: Into<State> + TryFrom<State, Error = (UnexpectedState, State)> {
    /// Get the [`StateName`] for this state.
    fn state_name() -> StateName;
}

macro_rules! impl_is_state {
    ($name:ident($ty:ident)) => {
        impl IsState for $ty {
            fn state_name() -> StateName {
                StateName::$name
            }
        }

        impl From<$ty> for State {
            fn from(s: $ty) -> Self {
                Self::$name(s)
            }
        }

        impl TryFrom<State> for $ty {
            type Error = (UnexpectedState, State);

            fn try_from(state: State) -> Result<Self, (UnexpectedState, State)> {
                if let State::$name(s) = state {
                    Ok(s)
                } else {
                    Err((
                        UnexpectedState {
                            expected_state: <$ty as IsState>::state_name(),
                            actual_state: state.state_name(),
                        },
                        state,
                    ))
                }
            }
        }
    };
}

impl_is_state!(Inactive(Inactive));
impl_is_state!(Ready(Ready));
impl_is_state!(Started(Started));
impl_is_state!(Locked(Locked));
impl_is_state!(PendingClose(ClosingMessage));
impl_is_state!(Closed(Closed));

impl State {
    /// Get the name of this state.
    pub fn state_name(&self) -> StateName {
        match self {
            State::Inactive(_) => StateName::Inactive,
            State::Ready(_) => StateName::Ready,
            State::Started(_) => StateName::Started,
            State::Locked(_) => StateName::Locked,
            State::PendingClose(_) => StateName::PendingClose,
            State::Closed(_) => StateName::Closed,
        }
    }

    /// Get the current [`CustomerBalance`] of this state.
    pub fn customer_balance(&self) -> &CustomerBalance {
        match self {
            State::Inactive(inactive) => inactive.customer_balance(),
            State::Ready(ready) => ready.customer_balance(),
            State::Started(started) => started.customer_balance(),
            State::Locked(locked) => locked.customer_balance(),
            State::PendingClose(closing_message) => closing_message.customer_balance(),
            State::Closed(closed) => closed.customer_balance(),
        }
    }

    pub fn merchant_balance(&self) -> &MerchantBalance {
        match self {
            State::Inactive(inactive) => inactive.merchant_balance(),
            State::Ready(ready) => ready.merchant_balance(),
            State::Started(started) => started.merchant_balance(),
            State::Locked(locked) => locked.merchant_balance(),
            State::PendingClose(closing_message) => closing_message.merchant_balance(),
            State::Closed(closed) => closed.merchant_balance(),
        }
    }

    pub fn channel_id(&self) -> &ChannelId {
        match self {
            State::Inactive(inactive) => inactive.channel_id(),
            State::Ready(ready) => ready.channel_id(),
            State::Started(started) => started.channel_id(),
            State::Locked(locked) => locked.channel_id(),
            State::PendingClose(closing_message) => closing_message.channel_id(),
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
