use super::types::ContractId;
use {
    dashmap::{DashMap, DashSet},
    futures::stream::Stream,
    skiplist::SkipMap,
    std::{
        cmp::Reverse,
        collections::{HashMap, HashSet, VecDeque},
        future::Future,
        hash::Hash,
        ops::{Add, Sub},
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll},
    },
    tezedge::{api::BlockHead, BlockHash, OperationHash},
    tokio::sync::{mpsc, oneshot},
    uuid::Uuid,
};

pub struct Notifications<F: Fetch> {
    // Outputs:
    pending_operations: Arc<Mutex<PendingOperations<F>>>,
    subscribers: Arc<DashMap<SubscriberId, SubscriberSink>>,
}

struct PendingOperations<F: Fetch> {
    confirmation: SkipMap<Level, Vec<oneshot::Sender<()>>>,
    cancellation: HashMap<<F::Block as Block>::Id, oneshot::Sender<()>>,
}

impl<F: Fetch + Send + 'static> Notifications<F>
where
    F::Block: Send + Sync,
    F::Error: Send,
    <F::Block as Block>::Id: Send + Hash + Eq,
{
    pub async fn listen(fetcher: F, confirmation_depth: usize) -> Result<Self, F::Error> {
        let (tx_confirm_operation, mut rx_confirm_operation) = mpsc::channel(1);

        tokio::spawn(async move {
            let mut cache = Cache::with_fetcher_and_capacity(fetcher, confirmation_depth).await?;
            let waiting_operations: Vec<Confirm> = Vec::new();
            let confirmation: SkipMap<Level, Vec<oneshot::Sender<()>>> = SkipMap::new();
            let cancellation: HashMap<<F::Block as Block>::Id, oneshot::Sender<()>> =
                HashMap::new();

            loop {
                tokio::select! {
                    confirm_operation = rx_confirm_operation.recv() => {
                        match confirm_operation {
                            Some(Confirm { operation, confirm_at_depth, confirm, cancel }) => {

                            },
                            None => return Ok(()),
                        }
                    },
                    next_block = cache.next_block() => match next_block {
                        Ok(NextBlock::Clean { latest, confirmed }) => todo!(),
                        Ok(NextBlock::Reorg { latest, evicted }) => todo!(),
                        Err(err) => return Err(err),
                    }
                }
            }
        });

        todo!()
    }
}

struct SubscriberId(Uuid);

enum SubscriberAction {
    Add(ContractId, Level),
    Remove(ContractId),
    Set(Vec<ContractId>),
}

struct Confirm {
    operation: OperationHash,
    confirm_at_depth: Depth,
    confirm: oneshot::Sender<()>,
    cancel: oneshot::Sender<()>,
}

struct SubscriberSink {
    sink: mpsc::Sender<ContractEvent>,
    contracts: HashSet<ContractId>,
}

pub struct ContractEventStream<F: Fetch> {
    contracts: DashSet<<F::Block as Block>::Id>,
    cache: Cache<F>,
}

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
impl<F: Fetch> Stream for ContractEventStream<F> {
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
    pub async fn add_contract(&self, contract_id: ContractId, originated: Level) {
        todo!()
    }

    /// Remove the given [`ContractId`] from the set of streamed contracts.
    pub async fn remove_contract(&self, contract_id: ContractId) {
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
    pub async fn set_contracts(&self, contract_ids: Vec<(ContractId, Level)>) {
        todo!()
    }
}

#[allow(unused)]
impl<F: Fetch> Notifications<F> {
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
    pub async fn contract_events(&self) -> (ContractEventStreamHandle, ContractEventStream<F>) {
        todo!()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Level(u64);

impl From<u64> for Level {
    fn from(n: u64) -> Self {
        Self(n)
    }
}

impl From<Level> for u64 {
    fn from(h: Level) -> Self {
        h.0
    }
}

impl Add<u64> for Level {
    type Output = Level;

    fn add(self, rhs: u64) -> Self::Output {
        Level(self.0 + rhs)
    }
}

impl Sub<u64> for Level {
    type Output = Level;

    fn sub(self, rhs: u64) -> Self::Output {
        Level(self.0 - rhs)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Depth(Reverse<u64>);

impl From<u64> for Depth {
    fn from(n: u64) -> Self {
        Self(Reverse(n))
    }
}

impl From<Depth> for u64 {
    fn from(d: Depth) -> Self {
        d.0 .0
    }
}

pub enum Confirmation {
    Confirmed,
    Dropped,
}

pub enum NextBlock<'a, B> {
    Clean { latest: &'a B, confirmed: Option<B> },
    Reorg { latest: &'a B, evicted: Vec<B> },
}

pub struct Cache<F: Fetch> {
    // Invariant: never empty
    blocks: VecDeque<<F as Fetch>::Block>,
    fetcher: F,
    capacity: usize,
}

impl<F: Fetch> Cache<F>
where
    <F::Block as Block>::Id: Hash + Eq,
{
    /// Instantiate a new `Cache` with the given capacity. The capacity *must* be larger than the
    /// maximum depth of any possible reorganization of the chain, or a panic will occur when a
    /// reorg that goes too deep occurs.
    pub async fn with_fetcher_and_capacity(
        mut fetcher: F,
        capacity: usize,
    ) -> Result<Self, F::Error> {
        let mut blocks = VecDeque::with_capacity(capacity);

        'start: loop {
            // Fetch the current head block
            let mut block = fetcher.fetch_head().await?;

            // Fill up the cache with the predecessors of the head block
            for _ in 0..capacity {
                // Fetch the previous block by level
                let predecessor = fetcher.fetch_level(block.level() - 1).await?;
                if predecessor.id() != block.predecessor() {
                    // A reorg happened during initialization; restart from beginning
                    blocks.clear();
                    continue 'start;
                }
                blocks.push_back(block);
                block = predecessor;
            }

            // Put the last predecessor into the cache
            blocks.push_back(block);

            // Exit the loop because no reorgs have happened during initialization
            return Ok(Cache {
                blocks,
                fetcher,
                capacity,
            });
        }
    }

    /// Fetch the next head block into the cache, evicting the oldest block. If a reorg has
    /// occurred, prunes the cache to evict all blocks removed in the reorg and reports those blocks
    /// as output.
    pub async fn next_block<'a>(
        &'a mut self,
    ) -> Result<NextBlock<'a, <F as Fetch>::Block>, F::Error> {
        // The current head block
        let current_head = self
            .blocks
            .front()
            .expect("Invariant violation in `next_block`: empty block cache");

        // The next head block
        let next = self.fetcher.fetch_level(current_head.level() + 1).await?;

        // The id of the predecessor to the next block
        let expected_head = next.predecessor();

        if expected_head == current_head.id() {
            // Reorg has not occurred, so merely push the next block into the cache
            self.blocks.push_front(next);
            Ok(NextBlock::Clean {
                confirmed: if self.blocks.len() > self.capacity + 1 {
                    Some(self.blocks.pop_back().unwrap())
                } else {
                    None
                },
                latest: &self.blocks.front().unwrap(),
            })
        } else {
            // Reorg has occurred, so locate the level of the divergence
            let mut evicted: Vec<<F as Fetch>::Block> = Vec::new();

            while let Some(head) = self.blocks.pop_front() {
                if expected_head == head.id() {
                    self.blocks.push_front(head);
                    self.blocks.push_front(next);
                    return Ok(NextBlock::Reorg {
                        latest: &self.blocks.front().unwrap(),
                        evicted,
                    });
                } else {
                    evicted.push(head);
                }
            }

            // If we've exhausted all cached blocks, the reorg was too deep to handle, which means
            // we should panic
            panic!("Encountered reorg too deep to recover from: use larger cache capacity")
        }
    }
}

/// A `Block` is something which participates in a hash-linked list, i.e. a block chain.
pub trait Block {
    /// The ID type of this kind of block (such as a unique hash).
    type Id;

    /// Get the unique ID of this block.
    ///
    /// On two different blocks, this should never return the same value, unless there is some
    /// underlying hash collision, which we assume will not occur.
    fn id(&self) -> &Self::Id;

    /// Get the ID of the predecessor to this block.
    fn predecessor(&self) -> &Self::Id;

    /// Get the absolute level of this block (height from the beginning of the history of the
    /// chain).
    fn level(&self) -> Level;
}

impl Block for BlockHead {
    type Id = BlockHash;

    fn id(&self) -> &Self::Id {
        &self.hash
    }

    fn predecessor(&self) -> &Self::Id {
        &self.predecessor
    }

    fn level(&self) -> Level {
        Level(self.level)
    }
}

/// An abstraction of what it means to fetch a block from a trusted source of information about the
/// state of the blockchain.
///
/// Instantiate this trait for each different potential backend that might interact with the
/// blockchain.
pub trait Fetch {
    type Block: Block;
    type Error;
    type Future: Future<Output = Result<Self::Block, Self::Error>> + Send;

    fn fetch_level(&mut self, level: Level) -> Self::Future;

    fn fetch_head(&mut self) -> Self::Future;
}
