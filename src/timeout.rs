use async_trait::async_trait;
use futures::Future;
use std::time::Duration;
use tokio::time::Timeout;

#[async_trait]
pub trait WithTimeout {
    fn with_timeout(self, duration: Duration) -> Timeout<Self>
    where
        Self: Sized;
}

impl<T> WithTimeout for T
where
    T: Future + Sized,
{
    fn with_timeout(self, duration: Duration) -> Timeout<Self> {
        tokio::time::timeout(duration, self)
    }
}
