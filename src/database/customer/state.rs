use {
    serde::{Deserialize, Serialize},
    std::fmt::{Display, Formatter},
    thiserror::Error,
};

use zkchannels_crypto::impl_sqlx_for_bincode_ty;

use zkabacus_crypto::customer::{ClosingMessage, Inactive, Locked, Ready, Started};

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
            StateName::Closed => "close",
        }
        .fmt(f)
    }
}

pub trait NameState {
    fn name() -> StateName;
}

macro_rules! impl_name_state {
    ($t:ty, $name:expr) => {
        impl NameState for $t {
            fn name() -> StateName {
                $name
            }
        }
    };
}

impl_name_state!(Inactive, StateName::Inactive);
impl_name_state!(Ready, StateName::Ready);
impl_name_state!(Started, StateName::Started);
impl_name_state!(Locked, StateName::Locked);

impl_sqlx_for_bincode_ty!(State);

/// Declare a function that tries to unwrap a particular constructor of the [`State`] enum.
macro_rules! impl_state_try_getter {
    ($doc:tt, $method:ident, $constructor:ident, $state:ty $(,)?) => {
        #[doc = "Get the enclosed [`"]
        #[doc = $doc]
        #[doc = "`] state, if this state is one, otherwise returning `Err(self)`."]
        pub fn $method(self) -> Result<$state, State> {
            if let State::$constructor(r) = self {
                Ok(r)
            } else {
                Err(self)
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

    /// Get the [`StateName`] of this state.
    pub fn name(&self) -> StateName {
        match self {
            State::Inactive(_) => StateName::Inactive,
            State::Ready(_) => StateName::Ready,
            State::Started(_) => StateName::Started,
            State::Locked(_) => StateName::Locked,
            State::PendingClose(_) => StateName::PendingClose,
        }
    }
}

/// Try to match the specified case of a state, or generate an error if it doesn't match.
pub fn take_state<T: NameState>(
    getter: impl FnOnce(State) -> Result<T, State>,
    state: &mut Option<State>,
) -> Result<T, UnexpectedState> {
    // Ensure state is not closed, throwing an error describing the situation if so
    let open_state = state.take().ok_or(UnexpectedState {
        expected_state: T::name(),
        actual_state: StateName::Closed,
    })?;

    // Try to get the state using the getter
    let t = getter(open_state).map_err(|other_state| {
        // What was the actual state we encountered?
        let actual_state = other_state.name();

        // Restore the state back to the reference
        *state = Some(other_state);

        // Return an error describing the discrepancy
        UnexpectedState {
            expected_state: T::name(),
            actual_state,
        }
    })?;

    Ok(t)
}

/// Error thrown when an operation requires a channel to be in a particular state, but it is in a
/// different one instead.
#[derive(Debug, Serialize, Deserialize, Error)]
#[error("Expected channel in {expected_state} state, but it was in {actual_state} state")]
pub struct UnexpectedState {
    expected_state: StateName,
    actual_state: StateName,
}
