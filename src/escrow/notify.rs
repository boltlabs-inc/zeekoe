use serde::{Deserialize, Serialize};
use std::{
    pin::Pin,
    task::{Context, Poll},
};

use tezedge::OperationHash;

use super::types::ContractId;
use futures::stream::Stream;
use std::{
    cmp::Reverse,
    future::Future,
    hash::Hash,
    ops::{Add, Sub},
};

pub struct Notifications {}

pub struct ContractEventStream {}

#[allow(unused)]
pub struct ContractEvent {
    contract: ContractId,
    operation: OperationHash,
    event: ZkChannelEvent,
}

pub enum ZkChannelEvent {
    // TODO: zkchannels domain specific event types
}

pub enum Error {
    Reorg,
    Io(std::io::Error),
    // TODO: maybe other kinds of errors, add them here
}

#[allow(unused)]
impl Stream for ContractEventStream {
    type Item = Result<ContractEvent, std::io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        todo!()
    }
}

pub struct ContractEventStreamHandle {}

#[allow(unused)]
impl ContractEventStreamHandle {
    /// Add the given [`ContractId`] to the set of streamed contracts.
    /// The [`Level`] parameter indicates the level at which the [`ContractId`] was originated
    /// on chain.
    ///
    /// This will stream all events for the [`ContractId`] that have occurred on chain between
    /// [`Level`] and the current chain height.
    pub async fn add_contract(&self, contract_hash: &ContractId, originated: Level) {
        todo!()
    }

    /// Remove the given [`ContractId`] from the set of streamed contracts.
    pub async fn remove_contract(&self, contract_hash: &ContractId) {
        todo!()
    }

    /// Replace the set of streamed contracts with the given `contract_hashes`.
    /// The [`Level`] parameters indicate the level at which the [`ContractId`]s are originated
    /// on chain.
    ///
    /// This will not cause duplicated events for [`ContractId`]s that were already
    /// in the set of streamed contracts.
    /// This will stream all events for each _new_ [`ContractId`] that have occurred on chain
    /// between [`Level`] and the current chain height.
    pub async fn set_contracts(
        &self,
        contract_hashes: impl IntoIterator<Item = &(ContractId, Level)>,
    ) {
        todo!()
    }
}

#[allow(unused)]
impl Notifications {
    /// Wait for confirmation that the specified operation is confirmed at the given [`Depth`].
    ///
    /// This can be used for confirmation that an operation will not be lost in a reorg
    /// or for checking that a specified timeout has elapsed.
    pub async fn confirm_operation(
        &self,
        operation_hash: &OperationHash,
        confirmations: Depth,
    ) -> Result<(), Error> {
        todo!()
    }

    /// Get a stream of events and a linked handle that allows the contents of the stream to be
    /// updated by another task.
    pub async fn contract_events(&self) -> (ContractEventStreamHandle, ContractEventStream) {
        todo!()
    }
}

#[derive(
    Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, sqlx::Type,
)]
#[sqlx(transparent)]
pub struct Level(u32);

impl From<u32> for Level {
    fn from(n: u32) -> Self {
        Self(n)
    }
}

impl From<Level> for u32 {
    fn from(h: Level) -> Self {
        h.0
    }
}

impl Add<u32> for Level {
    type Output = Level;

    fn add(self, rhs: u32) -> Self::Output {
        Level(self.0 + rhs)
    }
}

impl Sub<u32> for Level {
    type Output = Level;

    fn sub(self, rhs: u32) -> Self::Output {
        Level(self.0 - rhs)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Depth(Reverse<usize>);

impl From<usize> for Depth {
    fn from(n: usize) -> Self {
        Self(Reverse(n))
    }
}

impl From<Depth> for usize {
    fn from(d: Depth) -> Self {
        d.0 .0
    }
}

pub enum Confirmation {
    Confirmed,
    Dropped,
}

#[allow(unused)]
pub struct Cache<B: Block, F: Fetch> {
    blocks: Vec<B>,
    fetcher: F,
}

impl<B: Block, F: Fetch> Cache<B, F> where B::Id: Hash + Eq + Clone {}

pub trait Block {
    type Id;

    fn id(&self) -> &Self::Id;

    fn predecessor(&self) -> &Self::Id;

    fn height(&self) -> Level;
}

pub trait Fetch {
    type Block: Block;
    type Error;
    type Future: Future<Output = Result<Self::Block, Self::Error>>;

    fn fetch_id(&mut self, id: <Self::Block as Block>::Id) -> Self::Future;

    fn fetch_height(&mut self, height: usize) -> Self::Future;

    fn fetch_head(&mut self) -> Self::Future;
}
