use {
    serde::{Deserialize, Serialize},
    std::fmt::{Display, Formatter},
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

/// Declare a function that eliminates one of the cases of the [`State`] struct.
macro_rules! impl_state_eliminator {
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
    impl_state_eliminator!("Inactive", inactive, Inactive, Inactive);
    impl_state_eliminator!("Ready", ready, Ready, Ready);
    impl_state_eliminator!("Started", started, Started, Started);
    impl_state_eliminator!("Locked", locked, Locked, Locked);
    impl_state_eliminator!(
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
