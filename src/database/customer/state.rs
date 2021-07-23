use {
    serde::{Deserialize, Serialize},
    std::fmt::{Display, Formatter},
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
    /// Channel has an originated contract but is not funded.
    Originated(Inactive),
    /// Channel has a customer-funded contract but has not received merchant funding.
    CustomerFunded(Inactive),
    /// Channel has received all funding but is not yet active.
    MerchantFunded(Inactive),
    /// Channel is ready for payment.
    Ready(Ready),
    /// Payment has been started, which means customer can close on new or old balance.
    Started(Started),
    /// Customer has revoked their ability to close on the old balance, but has not yet received the
    /// ability to make a new payment.
    Locked(Locked),
    /// A party has initiated closing, but it is not yet finalized on chain.
    PendingClose(ClosingMessage),
    /// Merchant has evidence that disputes the close balances proposed by the customer.
    Dispute(ClosingMessage),
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StateName {
    Inactive,
    Originated,
    CustomerFunded,
    MerchantFunded,
    Ready,
    Started,
    Locked,
    PendingClose,
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
            StateName::Started => "started",
            StateName::Locked => "locked",
            StateName::PendingClose => "pending close",
            StateName::Dispute => "disputed",
            StateName::Closed => "closed",
        }
        .fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZkAbacusDataName {
    Inactive,
    Ready,
    Started,
    Locked,
    CloseMessage,
}

impl Display for ZkAbacusDataName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ZkAbacusDataName::Inactive => "inactive",
            ZkAbacusDataName::Ready => "ready",
            ZkAbacusDataName::Started => "started",
            ZkAbacusDataName::Locked => "locked",
            ZkAbacusDataName::CloseMessage => "pending close",
        }
        .fmt(f)
    }
}

/// The set of zkAbacus states that are associated with at least one channel status/state.
pub trait IsZkAbacusState: Sized {
    /// Extract Self from State if State has the given StateName.
    fn from_state(state: State, expected: StateName) -> Result<Self, (StateError, State)>;
}

impl IsZkAbacusState for Inactive {
    fn from_state(state: State, expected_state: StateName) -> Result<Self, (StateError, State)> {
        if state.state_name() != expected_state {
            return Err((
                UnexpectedState {
                    expected_state,
                    actual_state: state.state_name(),
                }
                .into(),
                state,
            ));
        }
        match state {
            State::Inactive(inactive) => Ok(inactive),
            State::Originated(inactive) => Ok(inactive),
            State::CustomerFunded(inactive) => Ok(inactive),
            State::MerchantFunded(inactive) => Ok(inactive),
            _ => Err((
                ImpossibleState {
                    zkchannels_state: expected_state,
                    zkabacus_data: ZkAbacusDataName::Inactive,
                }
                .into(),
                state,
            )),
        }
    }
}

impl IsZkAbacusState for ClosingMessage {
    fn from_state(state: State, expected_state: StateName) -> Result<Self, (StateError, State)> {
        if state.state_name() != expected_state {
            return Err((
                UnexpectedState {
                    expected_state,
                    actual_state: state.state_name(),
                }
                .into(),
                state,
            ));
        }
        match state {
            State::PendingClose(closing_message) => Ok(closing_message),
            State::Dispute(closing_message) => Ok(closing_message),
            _ => Err((
                ImpossibleState {
                    zkchannels_state: expected_state,
                    zkabacus_data: ZkAbacusDataName::CloseMessage,
                }
                .into(),
                state,
            )),
        }
    }
}

macro_rules! impl_is_zkabacus_state {
    ($name:ident($ty:ident)) => {
        impl IsZkAbacusState for $ty {
            fn from_state(
                state: State,
                expected_state: StateName,
            ) -> Result<Self, (StateError, State)> {
                if state.state_name() != expected_state {
                    return Err((
                        UnexpectedState {
                            expected_state,
                            actual_state: state.state_name(),
                        }
                        .into(),
                        state,
                    ));
                }

                if let State::$name(s) = state {
                    Ok(s)
                } else {
                    Err((
                        ImpossibleState {
                            zkchannels_state: expected_state,
                            zkabacus_data: ZkAbacusDataName::$ty,
                        }
                        .into(),
                        state,
                    ))
                }
            }
        }
    };
}

impl_is_zkabacus_state!(Ready(Ready));
impl_is_zkabacus_state!(Started(Started));
impl_is_zkabacus_state!(Locked(Locked));
//impl_try_from!(Closed(Closed));

impl State {
    /// Get the name of this state.
    pub fn state_name(&self) -> StateName {
        match self {
            State::Inactive(_) => StateName::Inactive,
            State::Originated(_) => StateName::Originated,
            State::CustomerFunded(_) => StateName::CustomerFunded,
            State::MerchantFunded(_) => StateName::MerchantFunded,
            State::Ready(_) => StateName::Ready,
            State::Started(_) => StateName::Started,
            State::Locked(_) => StateName::Locked,
            State::PendingClose(_) => StateName::PendingClose,
            State::Dispute(_) => StateName::Dispute,
            State::Closed(_) => StateName::Closed,
        }
    }

    /// Get the current [`CustomerBalance`] of this state.
    pub fn customer_balance(&self) -> &CustomerBalance {
        match self {
            State::Inactive(inactive) => inactive.customer_balance(),
            State::Originated(inactive) => inactive.customer_balance(),
            State::CustomerFunded(inactive) => inactive.customer_balance(),
            State::MerchantFunded(inactive) => inactive.customer_balance(),
            State::Ready(ready) => ready.customer_balance(),
            State::Started(started) => started.customer_balance(),
            State::Locked(locked) => locked.customer_balance(),
            State::PendingClose(closing_message) => closing_message.customer_balance(),
            State::Dispute(closing_message) => closing_message.customer_balance(),
            State::Closed(closed) => closed.customer_balance(),
        }
    }

    pub fn merchant_balance(&self) -> &MerchantBalance {
        match self {
            State::Inactive(inactive) => inactive.merchant_balance(),
            State::Originated(inactive) => inactive.merchant_balance(),
            State::CustomerFunded(inactive) => inactive.merchant_balance(),
            State::MerchantFunded(inactive) => inactive.merchant_balance(),
            State::Ready(ready) => ready.merchant_balance(),
            State::Started(started) => started.merchant_balance(),
            State::Locked(locked) => locked.merchant_balance(),
            State::PendingClose(closing_message) => closing_message.merchant_balance(),
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
            State::Started(started) => started.channel_id(),
            State::Locked(locked) => locked.channel_id(),
            State::PendingClose(closing_message) => closing_message.channel_id(),
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

/// Error thrown when an operation requests a variant of zkAbacus data from a zkChannels state and
/// that does not contain such data.
#[derive(Debug, Serialize, Deserialize, Error)]
#[error(
    "Channel in {zkchannels_state} state does not contain zkAbacus data of type {zkabacus_data}"
)]
pub struct ImpossibleState {
    zkchannels_state: StateName,
    zkabacus_data: ZkAbacusDataName,
}

/// An error when manipulating zkChannels states.
#[derive(Debug, Error)]
pub enum StateError {
    /// The state was not what was expected.
    #[error(transparent)]
    UnexpectedState(#[from] UnexpectedState),
    /// The state does not contain the requested data.
    #[error(transparent)]
    ImpossibleState(#[from] ImpossibleState),
}
