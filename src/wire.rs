use crate::wire::raw::*;
use futures::Future;
use serde::{Deserialize, Serialize};
use std::convert::TryInto;
use thiserror::Error;

mod raw {
    tonic::include_proto!("wire");
}

/// If something is [`Transmit`], we can `send` any [`Serialize`] (and [`Sync`]) message along its
/// outgoing stream.
#[tonic::async_trait]
pub trait Transmit {
    /// The type of possible errors when sending.
    type Error;

    /// Send a message.
    async fn send<T: Serialize + Sync>(&mut self, message: &T) -> Result<(), Self::Error>;
}

/// If something is [`Receive`], we can `recv` any [`Deserialize`] message along its incoming
/// stream.
#[tonic::async_trait]
pub trait Receive {
    /// The type of possible errors when receiving.
    type Error;

    /// Receive a message. This may require type annotations for disambiguation.
    async fn recv<T: for<'a> Deserialize<'a>>(&mut self) -> Result<T, Self::Error>;
}

/// If something is [`Bidirectional`], it is both [`Transmit`] and [`Receive`], and additionally can
/// be split into separate connections to concurrently send and receive.
pub trait Bidirectional
where
    Self: Transmit + Receive,
{
    /// The type of the transmitting channel.
    type Tx: Transmit<Error = <Self as Transmit>::Error>;

    /// The type of the receiving channel.
    type Rx: Receive<Error = <Self as Receive>::Error>;

    /// Split this bidirectional connection into separate [`Transmit`] and [`Receive`] ends.
    fn split(&mut self) -> (&mut Self::Tx, &mut Self::Rx);
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
    pub use crate::wire::raw::generic_server::GenericServer;

    /// A server-side view of a connection to a particular client.
    ///
    /// This implements [`Transmit`], [`Receive`], and [`Bidirectional`], which means it's a
    /// bidirectional streaming connection to the client which can optionally be split into separate
    /// receiving and sending ends.
    pub struct Connection {
        requests: FromClient,
        replies: ToClient,
    }

    /// A stream of messages coming into a server from a particular client connection.
    ///
    /// This implements [`Receive`], which means it's a unidirectional incoming stream.
    pub struct FromClient(tonic::Streaming<Request>);

    /// A sink for messages going from a server to a particular client connection.
    ///
    /// This implements [`Transmit`], which means it's a unidirectional outgoing stream.
    pub struct ToClient(tokio::sync::mpsc::Sender<Result<Reply, tonic::Status>>);

    #[tonic::async_trait]
    impl Transmit for ToClient {
        type Error = Error;

        async fn send<T: Serialize + Sync>(&mut self, message: &T) -> Result<(), Self::Error> {
            if self
                .0
                .send(Ok(Reply {
                    reply: bincode::serialize(message)?,
                }))
                .await
                .is_err()
            {
                Err(Error::Disconnected)
            } else {
                Ok(())
            }
        }
    }

    #[tonic::async_trait]
    impl Receive for FromClient {
        type Error = Error;

        async fn recv<T: for<'a> Deserialize<'a>>(&mut self) -> Result<T, Self::Error> {
            match self.0.message().await? {
                Some(Request { request }) => Ok(bincode::deserialize(&request)?),
                None => Err(Error::Disconnected),
            }
        }
    }

    impl Connection {
        /// Close the connection with a [`tonic::Status`] indicating the reason for the closure.
        ///
        /// Dropping the [`Connection`] struct will also drop the connection, but this allows the server
        /// to indicate why the connection was closed.
        pub async fn close_with_status(mut self, status: tonic::Status) -> Result<(), Error> {
            self.replies
                .0
                .send(Err(status))
                .await
                .map_err(|_| Error::Disconnected)
        }
    }

    #[tonic::async_trait]
    impl Transmit for Connection {
        type Error = <ToClient as Transmit>::Error;

        async fn send<T: Serialize + Sync>(&mut self, message: &T) -> Result<(), Self::Error> {
            self.replies.send(message).await
        }
    }

    #[tonic::async_trait]
    impl Receive for Connection {
        type Error = <FromClient as Receive>::Error;

        async fn recv<T: for<'a> Deserialize<'a>>(&mut self) -> Result<T, Self::Error> {
            self.requests.recv().await
        }
    }

    impl Bidirectional for Connection {
        type Tx = ToClient;
        type Rx = FromClient;

        fn split(&mut self) -> (&mut Self::Tx, &mut Self::Rx) {
            (&mut self.replies, &mut self.requests)
        }
    }

    #[tonic::async_trait]
    impl<F, R> generic_server::Generic for F
    where
        F: Fn(Connection) -> R + Sync + Send + 'static,
        R: Future<Output = Result<(), Error>> + Send + 'static,
    {
        type InvokeStream = tokio::sync::mpsc::Receiver<Result<Reply, tonic::Status>>;

        async fn invoke(
            &self,
            requests: tonic::Request<tonic::Streaming<Request>>,
        ) -> Result<tonic::Response<Self::InvokeStream>, tonic::Status> {
            let requests = requests.into_inner();
            let (replies, rx) = tokio::sync::mpsc::channel(1);
            let _ = tokio::spawn(self(Connection {
                requests: FromClient(requests),
                replies: ToClient(replies),
            })); // TODO: log errors here
            Ok(tonic::Response::new(rx))
        }
    }
}

pub mod client {
    //! A generic client that can talk to servers defined in [`crate::wire::server`] using ad-hoc
    //! [`Serialize`] messages.

    use super::*;

    /// A client-side view of a connection to a server.
    ///
    /// This implements [`Transmit`], [`Receive`], and [`Bidirectional`], which means it's a
    /// bidirectional streaming connection to the server which can optionally be split into separate
    /// receiving and sending ends.
    pub struct Client {
        requests: ToServer,
        replies: FromServer,
    }

    /// A sink for messages going from the client to the server.
    ///
    /// This implements [`Transmit`], which means it's a unidirectional outgoing stream.
    pub struct ToServer(tokio::sync::mpsc::Sender<Request>);

    /// A stream of messages coming from the server to this client.
    ///
    /// This implements [`Receive`], which means it's a unidirectional incoming stream.
    pub struct FromServer(tonic::Streaming<Reply>);

    impl Client {
        pub async fn connect<D>(dst: D) -> Result<Self, Error>
        where
            D: TryInto<tonic::transport::Endpoint>,
            D::Error: Into<tonic::codegen::StdError>,
        {
            let mut client = generic_client::GenericClient::connect(dst).await?;
            let (requests, rx) = tokio::sync::mpsc::channel(1);
            let replies = client.invoke(rx).await?.into_inner();
            Ok(Client {
                requests: ToServer(requests),
                replies: FromServer(replies),
            })
        }
    }

    #[tonic::async_trait]
    impl Transmit for ToServer {
        type Error = Error;

        async fn send<T: Serialize + Sync>(&mut self, message: &T) -> Result<(), Self::Error> {
            if self
                .0
                .send(Request {
                    request: bincode::serialize(&message)?,
                })
                .await
                .is_err()
            {
                Err(Error::Disconnected)
            } else {
                Ok(())
            }
        }
    }

    #[tonic::async_trait]
    impl Receive for FromServer {
        type Error = Error;

        async fn recv<T: for<'a> Deserialize<'a>>(&mut self) -> Result<T, Self::Error> {
            match self.0.message().await? {
                Some(Reply { reply }) => Ok(bincode::deserialize(&reply)?),
                None => Err(Error::Disconnected),
            }
        }
    }

    #[tonic::async_trait]
    impl Transmit for Client {
        type Error = <ToServer as Transmit>::Error;

        async fn send<T: Serialize + Sync>(&mut self, message: &T) -> Result<(), Self::Error> {
            self.requests.send(&message).await
        }
    }

    #[tonic::async_trait]
    impl Receive for Client {
        type Error = <FromServer as Receive>::Error;

        async fn recv<T: for<'a> Deserialize<'a>>(&mut self) -> Result<T, Self::Error> {
            self.replies.recv().await
        }
    }

    impl Bidirectional for Client {
        type Tx = ToServer;
        type Rx = FromServer;

        fn split(&mut self) -> (&mut Self::Tx, &mut Self::Rx) {
            (&mut self.requests, &mut self.replies)
        }
    }
}
