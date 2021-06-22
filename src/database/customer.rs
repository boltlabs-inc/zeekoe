use {
    async_trait::async_trait,
    serde::{Deserialize, Serialize},
    sqlx::SqlitePool,
    std::any::Any,
};

use zkchannels_crypto::impl_sqlx_for_bincode_ty;

use zkabacus_crypto::customer::{Inactive, Locked, Ready, Requested, Started};

use crate::customer::client::ZkChannelAddress;

/// The current state of the channel, from the perspective of the customer.
#[derive(Debug, Serialize, Deserialize)]
pub enum State {
    /// Funding requested but not yet approved.
    Requested(Requested),
    /// Funding approved but channel is not yet active.
    Inactive(Inactive),
    /// Channel is ready for payment.
    Ready(Ready),
    /// Payment has been started, which means customer can close on new or old balance.
    Started(Started),
    /// Customer has revoked their ability to close on the old balance, but has not yet received the
    /// ability to make a new payment.
    Locked(Locked),
}

impl_sqlx_for_bincode_ty!(State);

impl State {
    /// Get the [`Requested`] state, if this state is one, otherwise returning `self`.
    pub fn requested(self) -> Result<Requested, State> {
        if let State::Requested(r) = self {
            Ok(r)
        } else {
            Err(self)
        }
    }

    /// Get the [`Inactive`] state, if this state is one, otherwise returning `self`.
    pub fn inactive(self) -> Result<Inactive, State> {
        if let State::Inactive(r) = self {
            Ok(r)
        } else {
            Err(self)
        }
    }

    /// Get the [`Ready`] state, if this state is one, otherwise returning `self`.
    pub fn ready(self) -> Result<Ready, State> {
        if let State::Ready(r) = self {
            Ok(r)
        } else {
            Err(self)
        }
    }

    /// Get the [`Started`] state, if this state is one, otherwise returning `self`.
    pub fn started(self) -> Result<Started, State> {
        if let State::Started(r) = self {
            Ok(r)
        } else {
            Err(self)
        }
    }

    /// Get the [`Locked`] state, if this state is one, otherwise returning `self`.
    pub fn locked(self) -> Result<Locked, State> {
        if let State::Locked(r) = self {
            Ok(r)
        } else {
            Err(self)
        }
    }
}

/// Available functions to query the customer database.
///
/// These are implemented automatically for any database handle which implements
/// [`ErasedQueryCustomer`]; when passing a trait object, use that trait instead, but prefer to call
/// the methods of this trait.
#[async_trait]
pub trait QueryCustomer {
    /// Insert a newly initialized [`Requested`] channel into the customer database, associated with
    /// a unique label and [`ZkChannelAddress`].
    async fn new_channel(
        &self,
        label: &str,
        address: &ZkChannelAddress,
        requested: Requested,
    ) -> Result<(), (Requested, sqlx::Error)>;

    /// Relabel an existing channel from a given label to a new one.
    ///
    /// Returns `true` if the label existed and `false` if it did not.
    async fn relabel_channel(&self, label: &str, new_label: &str) -> sqlx::Result<bool>;

    /// Assign a new [`ZkChannelAddress`] to an existing channel.
    ///
    /// Returns `true` if the label existed and `false` if it did not.
    async fn readdress_channel(
        &self,
        label: &str,
        new_address: &ZkChannelAddress,
    ) -> sqlx::Result<bool>;

    /// Given a channel's unique label, mutate its state in the database using one of two provided
    /// closures, depending on whether that state is dirty or clean.
    ///
    /// Each closure may return some result value which will be returned from the function as a
    /// whole.
    ///
    /// If this function is interrupted by a panic or crash mid-execution, the state in the database
    /// will be marked dirty, so the next time it is run, the `with_dirty_state` closure will be
    /// invoked.
    ///
    /// **Important:** Operations performed in this function should be pure, aside from the side
    /// effect of modifying their given `&mut Option<State>`.
    async fn with_channel_state<'a, T: Send + 'static, E: Send + 'static>(
        &'a self,
        label: &str,
        with_clean_state: impl for<'s> Fn(&'s mut Option<State>) -> T + Send + Sync + 'a,
        with_dirty_state: impl for<'s> Fn(&'s mut Option<State>) -> E + Send + Sync + 'a,
    ) -> sqlx::Result<Result<T, E>>;
}

/// Trait-object safe version of [`QueryCustomer`]: use this type in trait objects and implement it
/// for database backends, but prefer to call the methods from [`QueryCustomer`], since all
/// [`ErasedQueryCustomer`] are [`QueryCustomer`].
#[async_trait]
pub trait ErasedQueryCustomer {
    /// See [`QueryCustomer::new_channel`].
    async fn new_channel(
        &self,
        label: &str,
        address: &ZkChannelAddress,
        requested: Requested,
    ) -> Result<(), (Requested, sqlx::Error)>;

    /// See [`QueryCustomer::relabel_channel`].
    async fn relabel_channel(&self, label: &str, new_label: &str) -> sqlx::Result<bool>;

    /// See [`QueryCustomer::readdress_channel`].
    async fn readdress_channel(
        &self,
        label: &str,
        new_address: &ZkChannelAddress,
    ) -> sqlx::Result<bool>;

    /// See [`QueryCustomer::with_channel_state`]. Note that this method uses `Box<dyn Any + Send>`
    /// to avoid the use of generic parameters, which is what allows the trait to be object safe.
    ///
    /// # Panics
    ///
    /// The corresponding method [`QueryCustomer::with_channel_state`] will panic if the boxed
    /// [`Any`] type returned by `with_clean_state` does not match that of the `Ok` case of the
    /// function's result, and similarly if the boxed [`Any`] type returned by `with_dirty_state`
    /// does not match the `Err` case of the function's result. It is expected that any
    /// implementation of this function merely forwards these values to the returned `Result<Box<dyn
    /// Any>, Box<dyn Any>>`.
    async fn with_channel_state(
        &self,
        label: &str,
        with_clean_state: &(dyn for<'a> Fn(&'a mut Option<State>) -> Box<dyn Any + Send> + Sync),
        with_dirty_state: &(dyn for<'a> Fn(&'a mut Option<State>) -> Box<dyn Any + Send> + Sync),
    ) -> sqlx::Result<Result<Box<dyn Any>, Box<dyn Any>>>;
}

#[async_trait]
impl ErasedQueryCustomer for SqlitePool {
    async fn new_channel(
        &self,
        label: &str,
        address: &ZkChannelAddress,
        requested: Requested,
    ) -> Result<(), (Requested, sqlx::Error)> {
        let state = State::Requested(requested);
        let state_ref = &state;

        sqlx::query!(
            "INSERT INTO customer_channels (label, address, state, clean) VALUES (?, ?, ?, ?)",
            label,
            address,
            state_ref,
            true,
        )
        .execute(self)
        .await
        .map(|_| ())
        .map_err(|e| (state.requested().unwrap(), e))
    }

    async fn relabel_channel(&self, label: &str, new_label: &str) -> sqlx::Result<bool> {
        sqlx::query!(
            "UPDATE customer_channels SET label = ? WHERE label = ?",
            new_label,
            label,
        )
        .execute(self)
        .await
        .map(|r| r.rows_affected() == 1)
    }

    async fn readdress_channel(
        &self,
        label: &str,
        new_address: &ZkChannelAddress,
    ) -> sqlx::Result<bool> {
        sqlx::query!(
            "UPDATE customer_channels SET address = ? WHERE label = ?",
            new_address,
            label,
        )
        .execute(self)
        .await
        .map(|r| r.rows_affected() == 1)
    }

    async fn with_channel_state(
        &self,
        label: &str,
        with_clean_state: &(dyn for<'a> Fn(&'a mut Option<State>) -> Box<dyn Any + Send> + Sync),
        with_dirty_state: &(dyn for<'a> Fn(&'a mut Option<State>) -> Box<dyn Any + Send> + Sync),
    ) -> sqlx::Result<Result<Box<dyn Any>, Box<dyn Any>>> {
        let mut transaction = self.begin().await?;

        // Determine if the state was clean
        let clean = sqlx::query!("SELECT clean FROM customer_channels WHERE label = ?", label)
            .fetch_one(&mut transaction)
            .await?
            .clean;

        // Set the state to dirty, so if for any reason this operation is interrupted, then we will
        // not be able to re-try any operations on this state
        sqlx::query!(
            "UPDATE customer_channels SET clean = ? WHERE label = ?",
            false,
            label,
        )
        .execute(&mut transaction)
        .await?;

        // Retrieve the state so that we can modify it
        let mut state: Option<State> = sqlx::query!(
            r#"SELECT state AS "state: State" FROM customer_channels WHERE label = ?"#,
            label,
        )
        .fetch_one(&mut transaction)
        .await?
        .state;

        // Perform the operation with the state fetched from the database
        let output = if clean {
            with_clean_state(&mut state)
        } else {
            with_dirty_state(&mut state)
        };

        // Store the new state to the database and set it to clean again
        sqlx::query!(
            "UPDATE customer_channels SET clean = ?, state = ? WHERE label = ?",
            true,
            state,
            label,
        )
        .execute(&mut transaction)
        .await?;

        // Commit the transaction
        transaction.commit().await?;

        Ok(Ok(output))
    }
}

// Blanket implementation of [`QueryCustomer`] for all [`ErasedQueryCustomer`]
#[async_trait]
impl<P: ErasedQueryCustomer + Sync> QueryCustomer for P {
    async fn new_channel(
        &self,
        label: &str,
        address: &ZkChannelAddress,
        requested: Requested,
    ) -> Result<(), (Requested, sqlx::Error)> {
        <Self as ErasedQueryCustomer>::new_channel(self, label, address, requested).await
    }

    async fn relabel_channel(&self, label: &str, new_label: &str) -> sqlx::Result<bool> {
        <Self as ErasedQueryCustomer>::relabel_channel(self, label, new_label).await
    }

    async fn readdress_channel(
        &self,
        label: &str,
        new_address: &ZkChannelAddress,
    ) -> sqlx::Result<bool> {
        <Self as ErasedQueryCustomer>::readdress_channel(self, label, new_address).await
    }

    async fn with_channel_state<'a, T: Send + 'static, E: Send + 'static>(
        &'a self,
        label: &str,
        with_clean_state: impl for<'s> Fn(&'s mut Option<State>) -> T + Send + Sync + 'a,
        with_dirty_state: impl for<'s> Fn(&'s mut Option<State>) -> E + Send + Sync + 'a,
    ) -> sqlx::Result<Result<T, E>> {
        <Self as ErasedQueryCustomer>::with_channel_state(
            self,
            label,
            &|state| Box::new(with_clean_state(state)),
            &|state| Box::new(with_dirty_state(state)),
        )
        .await
        .map(|result| {
            result
                .map(|t| *t.downcast().unwrap())
                .map_err(|e| *e.downcast().unwrap())
        })
    }
}
