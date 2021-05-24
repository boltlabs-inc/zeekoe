use {
    dialectic::prelude::*,
    dialectic_reconnect::{resume, retry},
    dialectic_tokio_serde_bincode::length_delimited,
    futures::{stream::FuturesUnordered, Future, StreamExt},
    std::{fmt::Display, io, net::SocketAddr, sync::Arc},
    tokio::{
        net::{TcpListener, TcpStream},
        select,
        sync::mpsc,
    },
    tokio_rustls::{
        rustls::{self, Certificate, PrivateKey},
        webpki::DNSName,
        TlsAcceptor, TlsConnector,
    },
};

pub use channel::{ClientChan, ServerChan, TransportError};
pub use dialectic_reconnect::Backoff;
use handshake::{Handshake, SessionKey};

mod channel;
mod handshake;
pub mod pem;

#[derive(Debug, Clone)]
pub struct Server {
    /// The maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    max_length: usize,
    /// The number of bytes used to represent the length in length-delimited encoding.
    length_field_bytes: usize,
    /// The server's TLS certificate.
    certificate_chain: Vec<Certificate>,
    /// The server's TLS private key.
    private_key: PrivateKey,
}

impl Server {
    pub fn new(certificate_chain: Vec<Certificate>, private_key: PrivateKey) -> Server {
        Server {
            max_length: usize::MAX,
            length_field_bytes: 4,
            certificate_chain,
            private_key,
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

    pub async fn serve_while<Protocol, Input, Error, Init, InitFut, Interaction, InteractionFut>(
        &self,
        address: impl Into<SocketAddr>,
        mut initialize: Init,
        interact: Interaction,
    ) -> Result<(), io::Error>
    where
        Protocol: Session,
        Input: Send + 'static,
        Error: Send + Display + 'static,
        Init: FnMut() -> InitFut,
        InitFut: Future<Output = Option<Input>>,
        Interaction: Fn(ServerChan<Protocol>, Input) -> InteractionFut + Send + Sync + 'static,
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
            Protocol::default(),
        ));

        // Error handling task awaits the result of each spawned server task and logs any errors that
        // occur, as they occur
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
                                        Err(_err) => {
                                            todo!("log handshake errors here")
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

#[cfg(all(not(debug_assertions), feature = "allow_explicit_certificate_trust"))]
compile_error!(
    "crate cannot be built for release with the `allow_explicit_certificate_trust` feature enabled"
);

#[derive(Clone)]
pub struct Client {
    /// The number of bytes used to represent the length field in the length-delimited encoding.
    length_field_bytes: usize,
    /// The maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    max_length: usize,
    /// The backoff strategy for reconnecting to the server in the event of a connection loss.
    backoff: Backoff,
    /// Client TLS configuration.
    tls_config: rustls::ClientConfig,
}

impl Client {
    pub fn new(backoff: Backoff) -> Client {
        let mut tls_config = rustls::ClientConfig::new();
        tls_config
            .root_store
            .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
        Client {
            length_field_bytes: 4,
            max_length: usize::MAX,
            backoff,
            tls_config,
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

    // Only non-release builds that explicitly request this capability via the feature, add the
    // auxiliary trusted certificate to the set of trusted certificates. In release builds, it
    // is not possible for the client to trust anyone other than the
    // `webpki_roots::TLS_SERVER_ROOTS`.
    #[cfg(feature = "allow_explicit_certificate_trust")]
    pub fn trust_explicit_certificate(
        &mut self,
        trust_explicit_certificate: &Certificate,
    ) -> Result<&mut Self, webpki::Error> {
        self.tls_config.root_store.add(trust_explicit_certificate)?;
        Ok(self)
    }

    pub async fn connect<Protocol: Session>(
        &self,
        domain: DNSName,
        port: u16,
    ) -> Result<
        ClientChan<Protocol>,
        retry::RetryError<std::convert::Infallible, io::Error, TransportError>,
    > {
        // Share the TLS config between all times we connect
        let tls_config = Arc::new(self.tls_config.clone());

        // Address configuration
        let length_field_bytes = self.length_field_bytes;
        let max_length = self.max_length;

        // A closure that connects to the server we want to connect to
        let connect = move |(domain, port): (DNSName, u16)| {
            let tls_config = tls_config.clone();
            async move {
                // Resolve the domain name we wish to connect to
                let address_str: &str = AsRef::as_ref(&domain);
                let mut addresses = tokio::net::lookup_host((address_str, port)).await?;

                // Attempt to connect to any of the socket addresses, succeeding on the first
                let mut connection_error = None;
                let tcp_stream = loop {
                    if let Some(address) = addresses.next() {
                        match TcpStream::connect(address).await {
                            Ok(tcp_stream) => break tcp_stream,
                            Err(e) => connection_error = Some(e),
                        }
                    } else {
                        return Err(connection_error.unwrap_or_else(|| {
                            io::Error::new(
                                io::ErrorKind::NotFound,
                                format!("unknown domain: {}", address_str),
                            )
                        }));
                    }
                };

                // Wrap a TCP stream in a TLS connection, then wrap that in a Dialectic channel
                let tls_connector = TlsConnector::from(tls_config);
                let tls_stream = tls_connector.connect(domain.as_ref(), tcp_stream).await?;
                let (rx, tx) = tokio::io::split(tls_stream);
                let (tx, rx) = length_delimited(tx, rx, length_field_bytes, max_length);
                Ok((tx, rx))
            }
        };

        let (_key, chan) = retry::Connector::new(
            connect,
            handshake::client::init::<_, _, TransportError>,
            handshake::client::retry::<_, _, TransportError>,
            Protocol::default(),
        )
        .recover_rx(self.backoff.build(retry::Recovery::ReconnectAfter))
        .recover_tx(self.backoff.build(retry::Recovery::ReconnectAfter))
        .recover_connect(self.backoff.build(retry::Recovery::ReconnectAfter))
        .recover_handshake(self.backoff.build(retry::Recovery::ReconnectAfter))
        .connect((domain, port))
        .await?;

        Ok(chan)
    }
}
