use crate::wire::dynamic::raw::*;
use futures::Future;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use std::pin::Pin;
use thiserror::Error;

use dialectic::{Receive, Ref, Transmit, Val};

mod raw {
    tonic::include_proto!("wire");
}

/// Errors that can occur during communication between clients and servers.
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum Error {
    /// A message failed to serialize or deserialize appropriately.
    #[error("{0}")]
    Serialization(#[from] Box<bincode::ErrorKind>),
    /// There was an issue in the gRPC transport layer.
    #[error("{0}")]
    Transport(#[from] tonic::transport::Error),
    /// The remote peer returned a status code instead of a message.
    #[error("{0}")]
    Status(#[from] tonic::Status),
    /// The remote peer disconnected without sending a final status code.
    #[error("remote peer disconnected")]
    Disconnected,
}

pub mod server {
    //! Constructing a generic server based on an `async` function from streaming inputs to
    //! streaming outputs, of arbitrary [`Serialize`] types.

    use super::*;

    /// A gRPC server whose behavior is defined by an `async` function passed in.
    pub use crate::wire::dynamic::raw::dynamic_server::DynamicServer;

    /// A stream of messages coming into a server from a particular client connection.
    ///
    /// This implements [`Receive`], which means it's a unidirectional incoming stream.
    #[derive(Debug)]
    pub struct FromClient(tonic::Streaming<Request>);

    /// A sink for messages going from a server to a particular client connection.
    ///
    /// This implements [`Transmit`], which means it's a unidirectional outgoing stream.
    #[derive(Debug)]
    pub struct ToClient(tokio::sync::mpsc::Sender<Result<Reply, tonic::Status>>);

    // The primary way a `ToClient` sends things is by serializing a reference.
    impl<'a, T: Serialize + Sync + 'a> Transmit<'a, T, Ref> for ToClient {
        type Error = Error;
        type Future = Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'static>>;

        fn send(&mut self, message: &'a T) -> Self::Future {
            match bincode::serialize(message) {
                Err(err) => Box::pin(async { Err(err.into()) }),
                Ok(reply) => {
                    let mut tx = self.0.clone();
                    Box::pin(async move {
                        if tx.send(Ok(Reply { reply })).await.is_err() {
                            Err(Error::Disconnected)
                        } else {
                            Ok(())
                        }
                    })
                }
            }
        }
    }

    // Sending a value delegates to the impl for sending a reference.
    impl<'a, T: Send + Sync + Serialize + 'a> Transmit<'a, T, Val> for ToClient {
        type Error = Error;
        type Future = <ToClient as Transmit<'a, T, Ref>>::Future;

        fn send(&mut self, message: T) -> Self::Future {
            <ToClient as Transmit<'_, T, Ref>>::send(self, &message)
        }
    }

    #[tonic::async_trait]
    impl<T: for<'a> Deserialize<'a> + 'static> Receive<T> for FromClient {
        type Error = Error;

        async fn recv(&mut self) -> Result<T, Self::Error> {
            match self.0.message().await? {
                Some(Request { request }) => Ok(bincode::deserialize(&request)?),
                None => Err(Error::Disconnected),
            }
        }
    }

    impl ToClient {
        /// Close the connection with a [`tonic::Status`] indicating the reason for the closure.
        pub async fn close_with_status(mut self, status: tonic::Status) -> Result<(), Error> {
            self.0
                .send(Err(status))
                .await
                .map_err(|_| Error::Disconnected)
        }
    }

    #[tonic::async_trait]
    impl<F, R> dynamic_server::Dynamic for F
    where
        F: Fn(ToClient, FromClient) -> R + Sync + Send + 'static,
        R: Future<Output = Result<(), Error>> + Send + 'static,
    {
        type InvokeStream = tokio::sync::mpsc::Receiver<Result<Reply, tonic::Status>>;

        async fn invoke(
            &self,
            requests: tonic::Request<tonic::Streaming<Request>>,
        ) -> Result<tonic::Response<Self::InvokeStream>, tonic::Status> {
            let requests = requests.into_inner();
            let (replies, rx) = tokio::sync::mpsc::channel(1);
            let _ = tokio::spawn(self(ToClient(replies), FromClient(requests))); // TODO: log errors here
            Ok(tonic::Response::new(rx))
        }
    }
}

pub mod client {
    //! A generic client that can talk to servers defined in [`crate::wire::dynamic::server`] using
    //! ad-hoc [`Serialize`] messages.

    use super::*;

    /// A sink for messages going from the client to the server.
    ///
    /// This implements [`Transmit`], which means it's a unidirectional outgoing stream.
    #[derive(Debug)]
    pub struct ToServer(tokio::sync::mpsc::Sender<Request>);

    /// A stream of messages coming from the server to this client.
    ///
    /// This implements [`Receive`], which means it's a unidirectional incoming stream.
    #[derive(Debug)]
    pub struct FromServer(tonic::Streaming<Reply>);

    pub async fn connect<D>(dst: D) -> Result<(ToServer, FromServer), Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        let mut client = dynamic_client::DynamicClient::connect(dst).await?;
        let (requests, rx) = tokio::sync::mpsc::channel(1);
        let replies = client.invoke(rx).await?.into_inner();
        Ok((ToServer(requests), FromServer(replies)))
    }

    // The primary way a `ToClient` sends things is by serializing a reference.
    impl<'a, T: Serialize + Sync + 'a> Transmit<'a, T, Ref> for ToServer {
        type Error = Error;
        type Future = Pin<Box<dyn Future<Output = Result<(), Self::Error>> + Send + 'static>>;

        fn send(&mut self, message: &'a T) -> Self::Future {
            match bincode::serialize(message) {
                Err(err) => Box::pin(async { Err(err.into()) }),
                Ok(request) => {
                    let mut tx = self.0.clone();
                    Box::pin(async move {
                        if tx.send(Request { request }).await.is_err() {
                            Err(Error::Disconnected)
                        } else {
                            Ok(())
                        }
                    })
                }
            }
        }
    }

    // Sending a value delegates to the impl for sending a reference.
    impl<'a, T: Send + Sync + Serialize + 'a> Transmit<'a, T, Val> for ToServer {
        type Error = Error;
        type Future = <ToServer as Transmit<'a, T, Ref>>::Future;

        fn send(&mut self, message: T) -> Self::Future {
            <ToServer as Transmit<'_, T, Ref>>::send(self, &message)
        }
    }

    #[tonic::async_trait]
    impl<T: for<'a> Deserialize<'a> + 'static> Receive<T> for FromServer {
        type Error = Error;

        async fn recv(&mut self) -> Result<T, Self::Error> {
            match self.0.message().await? {
                Some(Reply { reply }) => Ok(bincode::deserialize(&reply)?),
                None => Err(Error::Disconnected),
            }
        }
    }
}
