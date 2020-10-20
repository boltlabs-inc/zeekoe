use crate::wire::{self, activate, establish, pay};
use futures::Future;
use generic_array::GenericArray;
use ring::rand::SecureRandom;
use sha2::{Digest, Sha256};
use std::convert::{TryFrom, TryInto};
use tokio::sync::mpsc;
use tonic::transport::{self, Channel, Endpoint};
use tonic::{Request, Response};

#[derive(Debug, Clone, Copy)]
pub struct Balance(u64); // FIXME: use a proper currency type here

#[derive(Debug, Clone)]
pub struct Nonce([u8; 32]);

impl Nonce {
    pub fn new(rng: &dyn SecureRandom) -> Nonce {
        Nonce(
            ring::rand::generate(rng)
                .expect("Random nonce generation failed.")
                .expose(),
        )
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

    #[derive(Debug, Clone)]
    pub struct State {
        channel_id: Id,
        nonce: Nonce,
        revocation_lock: RevocationLock,
        merchant_balance: Balance,
        customer_balance: Balance,
    }

    #[derive(Debug, Clone)]
    pub struct RevocationLock; // FIXME: what is the shape of this data?

    #[derive(Debug, Clone)]
    pub struct RevocationSecret; // FIXME: what is the shape of this data?

    #[derive(Debug, Clone)]
    pub struct ClosingAuthorization; // FIXME:: what is the shape of this data?

    #[derive(Debug, Clone)]
    pub struct InvalidClosingAuthorization;

    impl TryFrom<Vec<u8>> for ClosingAuthorization {
        type Error = InvalidClosingAuthorization;
        fn try_from(bytes: Vec<u8>) -> Result<ClosingAuthorization, InvalidClosingAuthorization> {
            Ok(ClosingAuthorization) // FIXME: actually parse/validate closing auth here
        }
    }

    fn revocation(rng: &dyn SecureRandom) -> (RevocationLock, RevocationSecret) {
        (RevocationLock, RevocationSecret) // FIXME: actually generate a real lock and secret
    }

    #[derive(Debug, Clone)]
    pub struct PublicKey; // FIXME: have this contain key data

    #[derive(Debug, Clone)]
    pub struct PrivateKey; // FIXME: have this contain key data

    fn keypair(rng: &dyn SecureRandom) -> (PublicKey, PrivateKey) {
        (PublicKey, PrivateKey) // FIXME: actually generate a real keypair
    }

    #[derive(Debug, Clone)]
    pub struct Id(GenericArray<u8, <Sha256 as Digest>::OutputSize>);

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
        pub async fn initialize(
            mut self,
            customer_escrow: Balance,
        ) -> Result<Initialized<'random>, Error> {
            // Generate the initial cryptographic material for the channel
            let nonce = Nonce::new(self.rng);
            let (public_key, private_key) = keypair(self.rng);
            let (revocation_lock, revocation_secret) = revocation(self.rng);

            // Set up bidirectional channels over gRPC to the merchant for the Establish protocol
            let (mut requests, rx) = mpsc::channel(1);
            let mut replies = self.merchant.establish(rx).await?.into_inner();

            // Request an initialized channel from the merchant
            if requests
                .send(establish::Request {
                    request: Some(establish::request::Request::Initialize(
                        establish::request::Initialize {
                            channel_public_key: vec![], // FIXME: actually transcribe key data here
                            revocation_lock: vec![],    // FIXME: actually transcribe key data here
                            nonce: nonce.0.to_vec(),
                            customer_balance: customer_escrow.0,
                        },
                    )),
                })
                .await
                .is_err()
            {
                Err(Error::ConnectionLost)?;
            }

            // Wait for the merchant to respond with their desired initial balance
            let merchant_balance = Balance(match replies.message().await? {
                Some(establish::Reply {
                    reply:
                        Some(establish::reply::Reply::Initialize(establish::reply::Initialize {
                            merchant_balance,
                        })),
                }) => merchant_balance,
                Some(_) => Err(Error::UnexpectedResponse)?,
                None => Err(Error::ConnectionLost)?,
            });

            Ok(Initialized {
                rng: self.rng,
                merchant: self.merchant,
                requests,
                replies,
                customer_balance: customer_escrow,
                merchant_balance,
                nonce,
                public_key,
                private_key,
                revocation_lock,
                revocation_secret,
            })
        }
    }

    pub struct Initialized<'random> {
        rng: &'random dyn SecureRandom,
        merchant: MerchantClient,
        requests: tokio::sync::mpsc::Sender<establish::Request>,
        replies: tonic::Streaming<establish::Reply>,
        customer_balance: Balance,
        merchant_balance: Balance,
        nonce: Nonce,
        public_key: PublicKey,
        private_key: PrivateKey,
        revocation_lock: RevocationLock,
        revocation_secret: RevocationSecret,
    }

    impl<'random> Initialized<'random> {
        pub async fn escrow(mut self) -> Result<Escrowed<'random>, Error> {
            // Request that the merchant begin the escrow protocol
            if self
                .requests
                .send(establish::Request {
                    request: Some(establish::request::Request::StartEscrow(
                        establish::request::StartEscrow {
                            customer_auxiliary_data: vec![], // FIXME: put actual data here
                        },
                    )),
                })
                .await
                .is_err()
            {
                Err(Error::ConnectionLost)?;
            }

            // Get the merchant's auxiliary data and closing authorization
            let establish::reply::StartEscrow {
                merchant_auxiliary_data,
                closing_authorization,
            } = match self.replies.message().await? {
                Some(establish::Reply {
                    reply: Some(establish::reply::Reply::StartEscrow(reply)),
                }) => reply,
                Some(_) => Err(Error::UnexpectedResponse)?,
                None => Err(Error::ConnectionLost)?,
            };

            let closing_authorization = closing_authorization
                .try_into()
                .or(Err(Error::UnexpectedResponse))?;

            // Send the completed escrow and expiry authorizations back to the merchant
            if self
                .requests
                .send(establish::Request {
                    request: Some(establish::request::Request::CompleteEscrow(
                        establish::request::CompleteEscrow {
                            escrow_authorization: vec![], // FIXME: actual cryptography here
                            expiry_authorization: vec![], // FIXME: actual cryptography here
                        },
                    )),
                })
                .await
                .is_err()
            {
                Err(Error::ConnectionLost)?;
            }

            // Compute the initial state
            let state = State {
                channel_id: Id(Sha256::digest(&[])), // FIXME: compute the channel id here
                nonce: self.nonce,
                revocation_lock: self.revocation_lock,
                customer_balance: self.customer_balance,
                merchant_balance: self.merchant_balance,
            };

            Ok(Escrowed {
                rng: self.rng,
                merchant: self.merchant,
                state,
                private_key: self.private_key,
                revocation_secret: self.revocation_secret,
                closing_authorization,
            })
        }
    }

    pub struct Escrowed<'random> {
        rng: &'random dyn SecureRandom,
        merchant: MerchantClient,
        state: State,
        private_key: PrivateKey,
        revocation_secret: RevocationSecret,
        closing_authorization: ClosingAuthorization,
    }
}
