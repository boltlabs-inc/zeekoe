use {
    dialectic::prelude::*,
    dialectic_reconnect::resume,
    dialectic_tokio_serde_bincode::length_delimited,
    futures::{stream::FuturesUnordered, Future, StreamExt},
    std::{fmt::Display, io, marker::PhantomData, net::SocketAddr, sync::Arc},
    tokio::{net::TcpListener, select, sync::mpsc},
    tokio_rustls::{
        rustls::{self, Certificate, PrivateKey},
        TlsAcceptor,
    },
};

use super::channel::TransportError;
use super::handshake;

pub use super::channel::ServerChan as Chan;

#[derive(Debug, Clone)]
pub struct Server<Protocol: Session> {
    /// The maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    max_length: usize,
    /// The number of bytes used to represent the length in length-delimited encoding.
    length_field_bytes: usize,
    /// The server's TLS certificate.
    certificate_chain: Vec<Certificate>,
    /// The server's TLS private key.
    private_key: PrivateKey,
    /// The session, from the *client's* perspective.
    client_session: PhantomData<fn() -> Protocol>,
}

impl<Protocol> Server<Protocol>
where
    Protocol: Session,
    <Protocol as Session>::Dual: Session,
{
    pub fn new(certificate_chain: Vec<Certificate>, private_key: PrivateKey) -> Self {
        Server {
            max_length: usize::MAX,
            length_field_bytes: 4,
            certificate_chain,
            private_key,
            client_session: PhantomData,
        }
    }

    /// Set the number of bytes used to represent the length field in the length-delimited encoding.
    pub fn length_field_bytes(&mut self, length_field_bytes: usize) -> &mut Self {
        self.length_field_bytes = length_field_bytes;
        self
    }

    /// Set the maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    pub fn max_length(&mut self, max_length: usize) -> &mut Self {
        self.max_length = max_length;
        self
    }

    pub async fn serve_while<Input, Error, Init, InitFut, Interaction, InteractionFut>(
        &self,
        address: impl Into<SocketAddr>,
        mut initialize: Init,
        interact: Interaction,
    ) -> Result<(), io::Error>
    where
        Input: Send + 'static,
        Error: Send + Display + 'static,
        Init: FnMut() -> InitFut,
        InitFut: Future<Output = Option<Input>>,
        Interaction: Fn(Chan<Protocol>, Input) -> InteractionFut + Send + Sync + 'static,
        InteractionFut: Future<Output = Result<(), Error>> + Send + 'static,
    {
        // Configure server-side TLS
        let mut tls_config = rustls::ServerConfig::new(rustls::NoClientAuth::new());
        tls_config
            .set_single_cert(self.certificate_chain.clone(), self.private_key.clone())
            .map_err(|_error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid server certificate chain or private key",
                )
            })?;
        let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

        // Resume-handling acceptor to be shared between all connections
        let acceptor = Arc::new(resume::Acceptor::new(
            handshake::server::handshake::<_, _, TransportError>,
            <<Protocol as Session>::Dual>::default(),
        ));

        // Error handling task awaits the result of each spawned server task and logs any errors
        // that occur, as they occur
        let (result_tx, mut result_rx) = mpsc::channel(1024);
        let error_handler = tokio::spawn(async move {
            let mut results = FuturesUnordered::new();
            loop {
                select! {
                    Some(incoming) = result_rx.recv() => {
                        match incoming {
                            Ok((address, join_handle)) => results.push(async move { (address, join_handle.await) }),
                            Err((None, error)) => eprintln!("Server TCP initialization error: {}", error),
                            Err((Some(address), error)) => eprintln!("Server TLS initialization error [{}]: {}", address, error),
                        }
                    },
                    Some((address, result)) = results.next() => {
                        match result {
                            Ok(Ok(())) => {},
                            Ok(Err(error)) => eprintln!("Server task error [{}]: {}", address, error),
                            Err(join_error) => eprintln!("Server task panic [{}]: {}", address, join_error),
                        }
                    },
                    else => break,
                }
            }
        });

        // Wrap the server function in an `Arc` to share it between threads
        let interact = Arc::new(interact);

        let address = address.into();
        println!("serving on: {:?}", address);
        let listener = TcpListener::bind(address).await?;

        // Loop over incoming TCP connections until `initialize` returns `None`
        while let Some(input) = initialize().await {
            match listener.accept().await {
                Err(error) => result_tx.send(Err((None, error))).await.unwrap_or(()),
                Ok((tcp_stream, address)) => match tls_acceptor.accept(tcp_stream).await {
                    Err(error) => result_tx
                        .send(Err((Some(address), error)))
                        .await
                        .unwrap_or(()),
                    Ok(tls_stream) => {
                        let (rx, tx) = tokio::io::split(tls_stream);
                        let (tx, rx) =
                            length_delimited(tx, rx, self.length_field_bytes, self.max_length);
                        let acceptor = acceptor.clone();
                        let interact = interact.clone();
                        result_tx
                            .send(Ok((
                                address,
                                tokio::spawn(async move {
                                    match acceptor.accept(tx, rx).await {
                                        Ok((_key, Some(chan))) => {
                                            let interaction = interact(chan, input);
                                            interaction.await?;
                                        }
                                        Ok((_key, None)) => {
                                            // reconnected existing channel, nothing more to do
                                        }
                                        Err(err) => {
                                            use resume::AcceptError::*;
                                            match err {
                                                HandshakeError(_err) => {}
                                                HandshakeIncomplete => {}
                                                NoSuchSessionKey(_key) => {}
                                                SessionKeyAlreadyExists(_key) => {}
                                                NoCapacity => {}
                                            }
                                        }
                                    }
                                    Ok::<_, Error>(())
                                }),
                            )))
                            .await
                            .unwrap_or(());
                    }
                },
            }
        }

        error_handler.await?;
        Ok(())
    }
}
