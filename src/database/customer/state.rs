use {
    serde::{Deserialize, Serialize},
    std::fmt::{Display, Formatter},
    thiserror::Error,
};

use zkabacus_crypto::{
    customer::{ClosingMessage, Inactive, Locked, Ready, Started},
    impl_sqlx_for_bincode_ty, CustomerBalance, MerchantBalance,
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

/// The final balances of a channel closed on chain.
#[derive(Debug, Serialize, Deserialize)]
pub struct Closed {
    merchant_balance: MerchantBalance,
    customer_balance: CustomerBalance,
}

impl Closed {
    /// Get the final [`CustomerBalance`] for this closed channel state.
    pub fn customer_balance(&self) -> &CustomerBalance {
        &self.customer_balance
    }

    /// Get the final [`MerchantBalance`] for this closed channel state.
    pub fn merchant_balance(&self) -> &MerchantBalance {
        &self.merchant_balance
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
pub trait NameState {
    /// Get the [`StateName`] for this state.
    fn state_name() -> StateName;
}

macro_rules! impl_name_state {
    ($t:ty, $name:expr) => {
        impl NameState for $t {
            fn state_name() -> StateName {
                $name
            }
        }
    };
}

impl_name_state!(Inactive, StateName::Inactive);
impl_name_state!(Ready, StateName::Ready);
impl_name_state!(Started, StateName::Started);
impl_name_state!(Locked, StateName::Locked);
impl_name_state!(ClosingMessage, StateName::PendingClose);
impl_name_state!(Closed, StateName::Closed);

impl_sqlx_for_bincode_ty!(State);

/// Declare a function that tries to unwrap a particular constructor of the [`State`] enum.
macro_rules! impl_state_try_getter {
    ($doc:tt, $method:ident, $constructor:ident, $state:ty $(,)?) => {
        #[doc = "Get the enclosed [`"]
        #[doc = $doc]
        #[doc = "`] state, if this state is one, otherwise returning an error describing the mismatch, paired with `self`."]
        pub fn $method(self) -> Result<$state, (UnexpectedState, State)> {
            if let State::$constructor(r) = self {
                Ok(r)
            } else {
                Err((
                    UnexpectedState {
                        expected_state: <$state as NameState>::state_name(),
                        actual_state: self.name(),
                    },
                    self,
                ))
            }
        }
    };
}

impl State {
    impl_state_try_getter!("Inactive", inactive, Inactive, Inactive);
    impl_state_try_getter!("Ready", ready, Ready, Ready);
    impl_state_try_getter!("Started", started, Started, Started);
    impl_state_try_getter!("Locked", locked, Locked, Locked);
    impl_state_try_getter!(
        "ClosingMessage",
        pending_close,
        PendingClose,
        ClosingMessage,
    );
    impl_state_try_getter!("Closed", closed, Closed, Closed);

    /// Get the [`StateName`] of this state.
    pub fn name(&self) -> StateName {
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
}

/// Error thrown when an operation requires a channel to be in a particular state, but it is in a
/// different one instead.
#[derive(Debug, Serialize, Deserialize, Error)]
#[error("Expected channel in {expected_state} state, but it was in {actual_state} state")]
pub struct UnexpectedState {
    expected_state: StateName,
    actual_state: StateName,
}
