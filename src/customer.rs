use crate::wire::{self, activate, establish, pay};
use futures::Future;
use generic_array::{ArrayLength, GenericArray};
use ring::rand::SecureRandom;
use sha2::{Digest, Sha256};
use std::convert::{TryFrom, TryInto};
use tokio::sync::mpsc;
use tonic::transport::{self, Channel, Endpoint};
use tonic::{Request, Response};

use crate::chain;
use crate::revocation::{Revocation, RevocationLock, RevocationSecret};

pub struct Nonce<SecurityParameter: ArrayLength<u8>>(GenericArray<u8, SecurityParameter>);

impl<SecurityParameter> Nonce<SecurityParameter>
where
    SecurityParameter::ArrayType: ring::rand::RandomlyConstructable,
    SecurityParameter: ArrayLength<u8, ArrayType = [u8]>,
{
    /// Create a new random nonce with `SecurityParameter` bytes of entropy.
    pub fn new(rng: &dyn SecureRandom) -> Nonce<SecurityParameter> {
        let nonce: SecurityParameter::ArrayType = ring::rand::generate(rng)
            .expect("Random generation failed.")
            .expose();
        Nonce(GenericArray::from_slice(&nonce).clone())
    }

    /// Reveal the random nonce, consuming `self` to prevent accidental re-use once revealed.
    pub fn reveal(self) -> GenericArray<u8, SecurityParameter> {
        self.0
    }
}

pub mod channel {
    use super::*;
    use crate::wire::{self, activate, establish, pay};
    type MerchantClient = wire::merchant_client::MerchantClient<tonic::transport::Channel>;

    pub enum Error {
        TransportError(tonic::transport::Error),
        StatusError(tonic::Status),
        ConnectionLost,
        UnexpectedResponse,
    }

    impl From<tonic::transport::Error> for Error {
        fn from(err: tonic::transport::Error) -> Error {
            Error::TransportError(err)
        }
    }

    impl From<tonic::Status> for Error {
        fn from(status: tonic::Status) -> Error {
            Error::StatusError(status)
        }
    }

    pub struct State<SecurityParameter: ArrayLength<u8>, Hash: Digest, Chain: chain::Chain> {
        channel_id: Chain::ChannelId,
        nonce: Nonce<SecurityParameter>,
        revocation_lock: RevocationLock<Hash>,
        merchant_balance: Chain::Currency,
        customer_balance: Chain::Currency,
    }

    pub async fn connect<'random, D>(
        rng: &'random dyn SecureRandom,
        dst: D,
    ) -> Result<Connected<'random>, tonic::transport::Error>
    where
        D: TryInto<tonic::transport::Endpoint>,
        D::Error: Into<tonic::codegen::StdError>,
    {
        Ok(Connected {
            rng,
            merchant: crate::wire::merchant_client::MerchantClient::connect(dst).await?,
        })
    }

    pub struct Connected<'random> {
        rng: &'random dyn SecureRandom,
        merchant: MerchantClient,
    }

    impl<'random> Connected<'random> {
        pub async fn initialize<SecurityParameter, Hash: Digest, Chain: chain::Chain>(
            mut self,
            customer_escrow: Chain::Currency,
        ) -> Result<Initialized<'random, SecurityParameter, Hash, Chain>, Error>
        where
            SecurityParameter: ArrayLength<u8, ArrayType = [u8]>,
            SecurityParameter::ArrayType: ring::rand::RandomlyConstructable,
        {
            // Generate the initial cryptographic material for the channel
            let nonce = Nonce::new(self.rng);
            let (public_key, private_key) = Chain::channel_keypair(self.rng);
            let revocation = Revocation::new(self.rng);

            // Set up bidirectional channels over gRPC to the merchant for the Establish protocol
            let mut establish = StreamingMethod::connect(|rx| self.merchant.establish(rx)).await?;

            // Request an initialized channel from the merchant
            establish
                .send(establish::Request {
                    request: Some(establish::request::Request::Initialize(
                        establish::request::Initialize {
                            channel_public_key: public_key.clone().into(),
                            revocation_lock: revocation.lock().as_slice().into(),
                            nonce: nonce.0.to_vec(),
                            customer_balance: customer_escrow.clone().into(),
                        },
                    )),
                })
                .await?;

            // Wait for the merchant to respond with their desired initial balance
            let merchant_balance = match establish.recv().await? {
                Some(establish::Reply {
                    reply:
                        Some(establish::reply::Reply::Initialize(establish::reply::Initialize {
                            merchant_balance,
                        })),
                }) => merchant_balance
                    .try_into()
                    .map_err(|_| Error::UnexpectedResponse)?,
                Some(_) => Err(Error::UnexpectedResponse)?,
                None => Err(Error::ConnectionLost)?,
            };

            Ok(Initialized {
                rng: self.rng,
                merchant: self.merchant,
                establish,
                customer_balance: customer_escrow,
                merchant_balance,
                nonce,
                public_key,
                private_key,
                revocation,
            })
        }
    }

    /// An active connection to a bidirectionally streaming method. This wraps a call to a
    /// gRPC method with an input and output stream so that one can `send` and `recv` from
    /// it like any other bidirectional streaming connection.  
    pub struct StreamingMethod<Request, Reply> {
        requests: tokio::sync::mpsc::Sender<Request>,
        replies: tonic::Streaming<Reply>,
    }

    impl<Request: Send, Reply> StreamingMethod<Request, Reply> {
        /// Create a new streaming connection by invoking the given method and setting up
        /// input and output streams for it.
        pub async fn connect<M, F>(method: M) -> Result<Self, tonic::Status>
        where
            M: FnOnce(mpsc::Receiver<Request>) -> F,
            F: Future<Output = Result<Response<tonic::Streaming<Reply>>, tonic::Status>>,
        {
            let (requests, rx) = mpsc::channel(1);
            let replies = method(rx).await?.into_inner();
            Ok(StreamingMethod { requests, replies })
        }

        /// Send a request to the streaming method, not waiting for a reply.
        pub async fn send(&mut self, request: impl Into<Request>) -> Result<(), Error> {
            if self.requests.send(request.into()).await.is_err() {
                Err(Error::ConnectionLost)
            } else {
                Ok(())
            }
        }

        /// Wait to receive a reply from the streaming method, attempting to transform
        /// it into the given desired type.
        pub async fn recv<T>(&mut self) -> Result<T, Error>
        where
            Reply: TryInto<T>,
        {
            match self.replies.message().await? {
                Some(reply) => reply.try_into().map_err(|_| Error::UnexpectedResponse),
                None => Err(Error::ConnectionLost),
            }
        }
    }

    pub struct Initialized<
        'random,
        SecurityParameter: ArrayLength<u8>,
        Hash: Digest,
        Chain: chain::Chain,
    > {
        rng: &'random dyn SecureRandom,
        merchant: MerchantClient,
        establish: StreamingMethod<establish::Request, establish::Reply>,
        customer_balance: Chain::Currency,
        merchant_balance: Chain::Currency,
        nonce: Nonce<SecurityParameter>,
        public_key: Chain::ChannelPublicKey,
        private_key: Chain::ChannelPrivateKey,
        revocation: Revocation<SecurityParameter, Hash>,
    }

    // impl<'random, SecurityParameter, Hash, Chain: chain::Chain> Initialized<'random, SecurityParameter, Hash, Chain> {
    //     pub async fn escrow(mut self) -> Result<Escrowed<'random>, Error> {
    //         // Request that the merchant begin the escrow protocol
    //         if self
    //             .requests
    //             .send(establish::Request {
    //                 request: Some(establish::request::Request::StartEscrow(
    //                     establish::request::StartEscrow {
    //                         customer_auxiliary_data: vec![], // FIXME: put actual data here
    //                     },
    //                 )),
    //             })
    //             .await
    //             .is_err()
    //         {
    //             Err(Error::ConnectionLost)?;
    //         }

    //         // Get the merchant's auxiliary data and closing authorization
    //         let establish::reply::StartEscrow {
    //             merchant_auxiliary_data,
    //             closing_authorization,
    //         } = match self.replies.message().await? {
    //             Some(establish::Reply {
    //                 reply: Some(establish::reply::Reply::StartEscrow(reply)),
    //             }) => reply,
    //             Some(_) => Err(Error::UnexpectedResponse)?,
    //             None => Err(Error::ConnectionLost)?,
    //         };

    //         let closing_authorization = closing_authorization
    //             .try_into()
    //             .or(Err(Error::UnexpectedResponse))?;

    //         // Send the completed escrow and expiry authorizations back to the merchant
    //         if self
    //             .requests
    //             .send(establish::Request {
    //                 request: Some(establish::request::Request::CompleteEscrow(
    //                     establish::request::CompleteEscrow {
    //                         escrow_authorization: vec![], // FIXME: actual cryptography here
    //                         expiry_authorization: vec![], // FIXME: actual cryptography here
    //                     },
    //                 )),
    //             })
    //             .await
    //             .is_err()
    //         {
    //             Err(Error::ConnectionLost)?;
    //         }

    //         // Compute the initial state
    //         let state = State {
    //             channel_id: Id(Sha256::digest(&[])), // FIXME: compute the channel id here
    //             nonce: self.nonce,
    //             revocation_lock: self.revocation_lock,
    //             customer_balance: self.customer_balance,
    //             merchant_balance: self.merchant_balance,
    //         };

    //         Ok(Escrowed {
    //             rng: self.rng,
    //             merchant: self.merchant,
    //             state,
    //             private_key: self.private_key,
    //             revocation_secret: self.revocation_secret,
    //             closing_authorization,
    //         })
    //     }
    // }

    // pub struct Escrowed<'random> {
    //     rng: &'random dyn SecureRandom,
    //     merchant: MerchantClient,
    //     state: State,
    //     private_key: PrivateKey,
    //     revocation_secret: RevocationSecret,
    //     closing_authorization: ClosingAuthorization,
    // }
}
