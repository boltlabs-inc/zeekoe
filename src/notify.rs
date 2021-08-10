use {
    skiplist::SkipMap,
    std::{
        cmp::Reverse,
        collections::{BTreeMap, HashMap, VecDeque},
        future::Future,
        hash::Hash,
        ops::{Add, Sub},
        sync::Arc,
    },
    tokio::sync::{oneshot, RwLock},
};

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Height(usize);

impl From<usize> for Height {
    fn from(n: usize) -> Self {
        Self(n)
    }
}

impl From<Height> for usize {
    fn from(h: Height) -> Self {
        h.0
    }
}

impl Add<usize> for Height {
    type Output = Height;

    fn add(self, rhs: usize) -> Self::Output {
        Height(self.0 + rhs)
    }
}

impl Sub<usize> for Height {
    type Output = Height;

    fn sub(self, rhs: usize) -> Self::Output {
        Height(self.0 - rhs)
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

pub struct Cache<B: Block, F: Fetch> {
    blocks: Vec<B>,
    fetcher: F,
}

impl<B: Block, F: Fetch> Cache<B, F> where B::Id: Hash + Eq + Clone {}

pub trait Block {
    type Id;

    fn id(&self) -> &Self::Id;

    fn predecessor(&self) -> &Self::Id;

    fn height(&self) -> Height;
}

pub trait Fetch {
    type Block: Block;
    type Error;
    type Future: Future<Output = Result<Self::Block, Self::Error>>;

    fn fetch_id(&mut self, id: <Self::Block as Block>::Id) -> Self::Future;

    fn fetch_height(&mut self, height: usize) -> Self::Future;

    fn fetch_head(&mut self) -> Self::Future;
}
