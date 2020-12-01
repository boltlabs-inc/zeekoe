use crate::wire;
use futures::Future;
use generic_array::{ArrayLength, GenericArray};
use ring::rand::SecureRandom;
use sha2::{Digest, Sha256};
use std::convert::{TryFrom, TryInto};
use tokio::sync::mpsc;
use tonic::transport::{self, Channel, Endpoint};
use tonic::{Request, Response};

use crate::amount::Amount;
use crate::chain::{Arbiter, SignatureScheme};
use crate::revocation::{Revocation, RevocationLock, RevocationSecret};
use crate::util::*;

pub struct Nonce<Length: ArrayLength<u8>>(GenericArray<u8, Length>);

impl<Length> Nonce<Length>
where
    Length: ArrayLength<u8>,
{
    /// Create a new random nonce with `SecurityParameter` bytes of entropy.
    pub fn new(rng: &dyn SecureRandom) -> Nonce<Length> {
        Nonce(random_bytes(rng))
    }

    /// Reveal the random nonce, consuming `self` to prevent accidental re-use once revealed.
    pub fn reveal(self) -> GenericArray<u8, Length> {
        self.0
    }
}

// pub mod channel {
//     use super::*;

//     pub enum Error {
//         TransportError(tonic::transport::Error),
//         StatusError(tonic::Status),
//         ConnectionLost,
//         UnexpectedResponse,
//     }

//     impl From<tonic::transport::Error> for Error {
//         fn from(err: tonic::transport::Error) -> Error {
//             Error::TransportError(err)
//         }
//     }

//     impl From<tonic::Status> for Error {
//         fn from(status: tonic::Status) -> Error {
//             Error::StatusError(status)
//         }
//     }

//     pub struct State<NonceLength: ArrayLength<u8>, J: Arbiter> {
//         channel_id: J::ChannelId,
//         nonce: Nonce<NonceLength>,
//         revocation_lock: RevocationLock<J>,
//         merchant_balance: Amount<J::TransactionCurrency>,
//         customer_balance: Amount<J::TransactionCurrency>,
//     }

//     pub async fn connect<'random, D>(
//         rng: &'random dyn SecureRandom,
//         dst: D,
//     ) -> Result<Connected<'random>, tonic::transport::Error>
//     where
//         D: TryInto<tonic::transport::Endpoint>,
//         D::Error: Into<tonic::codegen::StdError>,
//     {
//         Ok(Connected {
//             rng,
//             merchant: crate::wire::merchant_client::MerchantClient::connect(dst).await?,
//         })
//     }

//     pub struct Connected<'random> {
//         rng: &'random dyn SecureRandom,
//         merchant: MerchantClient,
//     }

//     impl<'random> Connected<'random> {
//         pub async fn initialize<NonceLength: ArrayLength<u8>, J: Arbiter>(
//             mut self,
//             customer_escrow: Amount<J::TransactionCurrency>,
//         ) -> Result<Initialized<'random, NonceLength, J>, Error> {
//             // Generate the initial cryptographic material for the channel
//             let nonce = Nonce::new(self.rng);
//             let (public_key, private_key) = J::ClosingSignatureScheme::key_pair(self.rng);
//             let revocation = Revocation::new(self.rng);

//             // Set up bidirectional channels over gRPC to the merchant for the Establish protocol
//             let mut establish = StreamingMethod::connect(|rx| self.merchant.establish(rx)).await?;

//             // Request an initialized channel from the merchant
//             establish
//                 .send(establish::Request {
//                     request: Some(establish::request::Request::Initialize(
//                         establish::request::Initialize {
//                             channel_public_key: public_key.clone().into(),
//                             revocation_lock: revocation.lock().as_slice().into(),
//                             nonce: nonce.0.to_vec(),
//                             customer_balance: customer_escrow.into(),
//                         },
//                     )),
//                 })
//                 .await?;

//             // Wait for the merchant to respond with their desired initial balance
//             let merchant_balance = match establish.recv().await? {
//                 Some(establish::Reply {
//                     reply:
//                         Some(establish::reply::Reply::Initialize(establish::reply::Initialize {
//                             merchant_balance,
//                         })),
//                 }) => merchant_balance
//                     .try_into()
//                     .map_err(|_| Error::UnexpectedResponse)?,
//                 Some(_) => Err(Error::UnexpectedResponse)?,
//                 None => Err(Error::ConnectionLost)?,
//             };

//             Ok(Initialized {
//                 rng: self.rng,
//                 merchant: self.merchant,
//                 establish,
//                 customer_balance: customer_escrow,
//                 merchant_balance,
//                 nonce,
//                 public_key,
//                 private_key,
//                 revocation,
//             })
//         }
//     }

//     /// An active connection to a bidirectionally streaming method. This wraps a call to a
//     /// gRPC method with an input and output stream so that one can `send` and `recv` from
//     /// it like any other bidirectional streaming connection.
//     #[derive(Debug)]
//     pub struct StreamingMethod<Request, Reply> {
//         requests: tokio::sync::mpsc::Sender<Request>,
//         replies: tonic::Streaming<Reply>,
//     }

//     impl<Request: Send, Reply> StreamingMethod<Request, Reply> {
//         /// Create a new streaming connection by invoking the given method and setting up
//         /// input and output streams for it.
//         pub async fn connect<M, F>(method: M) -> Result<Self, tonic::Status>
//         where
//             M: FnOnce(mpsc::Receiver<Request>) -> F,
//             F: Future<Output = Result<Response<tonic::Streaming<Reply>>, tonic::Status>>,
//         {
//             let (requests, rx) = mpsc::channel(1);
//             let replies = method(rx).await?.into_inner();
//             Ok(StreamingMethod { requests, replies })
//         }

//         /// Send a request to the streaming method, not waiting for a reply.
//         pub async fn send(&mut self, request: impl Into<Request>) -> Result<(), Error> {
//             if self.requests.send(request.into()).await.is_err() {
//                 Err(Error::ConnectionLost)
//             } else {
//                 Ok(())
//             }
//         }

//         /// Wait to receive a reply from the streaming method, attempting to transform
//         /// it into the given desired type.
//         pub async fn recv<T>(&mut self) -> Result<T, Error>
//         where
//             Reply: TryInto<T>,
//         {
//             match self.replies.message().await? {
//                 Some(reply) => reply.try_into().map_err(|_| Error::UnexpectedResponse),
//                 None => Err(Error::ConnectionLost),
//             }
//         }
//     }

//     pub struct Initialized<'random, NonceLength: ArrayLength<u8>, J: Arbiter> {
//         rng: &'random dyn SecureRandom,
//         merchant: MerchantClient,
//         establish: StreamingMethod<establish::Request, establish::Reply>,
//         customer_balance: Amount<J::TransactionCurrency>,
//         merchant_balance: Amount<J::TransactionCurrency>,
//         nonce: Nonce<NonceLength>,
//         public_key: <J::ClosingSignatureScheme as SignatureScheme<
//             J::SignatureSchemeSecurityParameter,
//             J::ClosingAuthorization,
//         >>::PublicKey,
//         private_key: <J::ClosingSignatureScheme as SignatureScheme<
//             J::SignatureSchemeSecurityParameter,
//             J::ClosingAuthorization,
//         >>::PrivateKey,
//         revocation: Revocation<J>,
//     }
// }
