//! The server side of Zeekoe's transport layer.

use {
    dialectic::prelude::*,
    dialectic_reconnect::resume,
    dialectic_tokio_serde_bincode::length_delimited,
    futures::{stream::FuturesUnordered, Future, StreamExt},
    std::{
        fmt::Display, io, marker::PhantomData, net::SocketAddr, path::Path, sync::Arc,
        time::Duration,
    },
    tokio::{net::TcpListener, select, sync::mpsc},
    tokio_rustls::{
        rustls::{self, Certificate, PrivateKey},
        TlsAcceptor,
    },
};

use super::{channel::TransportError, handshake, pem};

pub use super::channel::ServerChan as Chan;
pub use handshake::SessionKey;

/// The type of errors returned during sessions on a server-side channel.
pub type Error = resume::ResumeError<TransportError>;

/// A server for some `Protocol` which accepts resumable connections over TLS.
///
/// The session type parameter for this type is the session from **the client's perspective.**
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
    /// The maximum permissible number of pending retries.
    max_pending_retries: Option<usize>,
    /// The timeout after which broken connections will be garbage-collected.
    timeout: Option<Duration>,
    /// The session, from the *client's* perspective.
    client_session: PhantomData<fn() -> Protocol>,
}

impl<Protocol> Server<Protocol>
where
    Protocol: Session,
    <Protocol as Session>::Dual: Session,
{
    /// Create a new server using the given certificate chain and private key.
    pub fn new(
        certificate_chain: impl AsRef<Path>,
        private_key: impl AsRef<Path>,
    ) -> Result<Self, io::Error> {
        Ok(Server {
            max_length: usize::MAX,
            length_field_bytes: 4,
            certificate_chain: pem::read_certificates(certificate_chain)?,
            private_key: pem::read_private_key(private_key)?,
            max_pending_retries: None,
            timeout: None,
            client_session: PhantomData,
        })
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

    /// Set a timeout for recovery within all future [`Chan`]s handled by this [`Server`].
    ///
    /// When there is a timeout, an error will be thrown if recovery from a previous error takes
    /// longer than the given timeout, even if the error recovery strategy specifies trying again.
    pub fn timeout(&mut self, timeout: Option<Duration>) -> &mut Self {
        self.timeout = timeout;
        self
    }

    /// Set the maximum number of pending retries for all future [`Chan`]s handled by this
    /// [`Server`].
    ///
    /// Restricting this limit (the default is `None`) prevents a potential unbounded memory leak in
    /// the case where a mis-behaving client attempts to reconnect many times before either end of a
    /// channel encounters an error and attempts to reconnect.
    pub fn max_pending_retries(&mut self, max_pending_retries: Option<usize>) -> &mut Self {
        self.max_pending_retries = max_pending_retries;
        self
    }

    /// Accept connections on `address` in a loop, running the `initialize` function when accepting.
    /// If `initialize` returns `None`, stop; otherwise, concurrently serve each connection with
    /// `interact`.
    ///
    /// Note that `initialize` runs sequentially: it can pause the server if desired by
    /// `.await`-ing.
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
        Interaction:
            Fn(SessionKey, Input, Chan<Protocol>) -> InteractionFut + Send + Sync + 'static,
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
        let mut acceptor = resume::Acceptor::new(
            handshake::server::handshake::<_, _, TransportError>,
            <<Protocol as Session>::Dual>::default(),
        );
        acceptor
            .timeout(self.timeout)
            .max_pending_retries(self.max_pending_retries);
        let acceptor = Arc::new(acceptor);

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
                Ok((tcp_stream, address)) => {
                    // Session typed messages may be small; send them immediately
                    tcp_stream.set_nodelay(true)?;

                    match tls_acceptor.accept(tcp_stream).await {
                        Err(error) => result_tx
                            .send(Err((Some(address), error)))
                            .await
                            .unwrap_or(()),
                        Ok(tls_stream) => {
                            // Layer a length-delimmited bincode `Chan` over the TLS stream
                            let (rx, tx) = tokio::io::split(tls_stream);
                            let (tx, rx) =
                                length_delimited(tx, rx, self.length_field_bytes, self.max_length);

                            let acceptor = acceptor.clone();
                            let interact = interact.clone();

                            // Run the interaction concurrently, or resume it if it's resuming an
                            // existing one
                            let join_handle = tokio::spawn(async move {
                                match acceptor.accept(tx, rx).await {
                                    Ok((session_key, Some(chan))) => {
                                        let interaction = interact(session_key, input, chan);
                                        interaction.await?;
                                    }
                                    Ok((_session_key, None)) => {
                                        // reconnected existing channel, nothing more to do
                                    }
                                    Err(err) => {
                                        use resume::AcceptError::*;
                                        // TODO: log these errors?
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
                            });

                            // Keep track of pending server task
                            result_tx
                                .send(Ok((address, join_handle)))
                                .await
                                .unwrap_or(());
                        }
                    }
                }
            }
        }

        error_handler.await?;
        Ok(())
    }
}
