use {
    dialectic::prelude::*,
    dialectic_reconnect::{resume, retry},
    dialectic_tokio_serde::{codec::LengthDelimitedCodec, SymmetricalError},
    dialectic_tokio_serde_bincode::{length_delimited, Bincode},
    futures::{stream::FuturesUnordered, Future, StreamExt},
    serde::{Deserialize, Serialize},
    std::{fmt::Display, fs::File, io, io::Read, net::SocketAddr, path::Path, sync::Arc},
    tokio::{
        net::{TcpListener, TcpStream},
        sync::mpsc,
    },
    tokio_rustls::{
        rustls::{self, Certificate, PrivateKey},
        webpki::DNSName,
        TlsAcceptor, TlsConnector,
    },
    uuid::Uuid,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SessionKey {
    client_key: Uuid,
    server_key: Uuid,
}

pub type Handshake = Session! {
    choose {
        0 => {
            send Uuid;
            recv Uuid;
        },
        1 => {
            send SessionKey;
        }
    }
};

type HandshakeErr = SymmetricalError<Bincode, LengthDelimitedCodec>;

pub use channel::{ClientChan, ServerChan};

mod channel {
    use {
        super::{Handshake, SessionKey},
        dialectic::Chan,
        dialectic_reconnect::{resume, retry},
        dialectic_tokio_serde::{codec::LengthDelimitedCodec, Receiver, Sender},
        dialectic_tokio_serde_bincode::Bincode,
        std::io,
        tokio::{
            io::{ReadHalf, WriteHalf},
            net::TcpStream,
        },
    };

    /// A *server-side* session-typed channel over TCP using length-delimited bincode encoding for
    /// serialization.
    pub type ServerChan<S> = ResumeSplitChan<
        S,
        SessionKey,
        Bincode,
        LengthDelimitedCodec,
        tokio_rustls::server::TlsStream<TcpStream>,
    >;

    /// A *client-side* session-typed channel over TCP using length-delimited bincode encoding for
    /// serialization.
    pub type ClientChan<S> = RetrySplitChan<
        S,
        SessionKey,
        Handshake,
        (),
        io::Error,
        dialectic_tokio_serde::SymmetricalError<Bincode, LengthDelimitedCodec>,
        Bincode,
        LengthDelimitedCodec,
        tokio_rustls::client::TlsStream<TcpStream>,
    >;

    type ResumeSplitChan<S, K, F, E, T> =
        Chan<S, ResumeSplitSender<K, F, E, T>, ResumeSplitReceiver<K, F, E, T>>;

    type RetrySplitChan<S, K, H, A, CErr, HErr, F, E, T> = Chan<
        S,
        RetrySplitSender<K, H, A, CErr, HErr, F, E, T>,
        RetrySplitReceiver<K, H, A, CErr, HErr, F, E, T>,
    >;

    type ResumeSplitSender<K, F, E, T> =
        resume::Sender<K, SplitSender<F, E, T>, SplitReceiver<F, E, T>>;
    type ResumeSplitReceiver<K, F, E, T> =
        resume::Receiver<K, SplitSender<F, E, T>, SplitReceiver<F, E, T>>;

    type RetrySplitSender<K, H, A, CErr, HErr, F, E, T> =
        retry::Sender<H, A, K, CErr, HErr, SplitSender<F, E, T>, SplitReceiver<F, E, T>>;
    type RetrySplitReceiver<K, H, A, CErr, HErr, F, E, T> =
        retry::Receiver<H, A, K, CErr, HErr, SplitSender<F, E, T>, SplitReceiver<F, E, T>>;

    type SplitSender<F, E, T> = Sender<F, E, WriteHalf<T>>;
    type SplitReceiver<F, E, T> = Receiver<F, E, ReadHalf<T>>;
}

pub fn read_certificates(path: impl AsRef<Path>) -> Result<Vec<Certificate>, io::Error> {
    let mut file = File::open(&path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    let mut certificates = Vec::new();
    for pem::Pem { contents, .. } in pem::parse_many(contents)
        .into_iter()
        .filter(|p| p.tag == "CERTIFICATE")
    {
        certificates.push(Certificate(contents));
    }
    Ok(certificates)
}

pub fn read_single_certificate(path: impl AsRef<Path>) -> Result<Certificate, io::Error> {
    let mut file = File::open(&path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    let pem = pem::parse(contents).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid PEM encoding in certificate: {}", e),
        )
    })?;
    if pem.tag == "CERTIFICATE" {
        Ok(Certificate(pem.contents))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("not labeled as a certificate: '{}'", pem.tag),
        ))
    }
}

pub fn read_private_key(path: impl AsRef<Path>) -> Result<PrivateKey, io::Error> {
    let mut file = File::open(&path)?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;

    let pem = pem::parse(contents).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid PEM encoding in private key: {}", e),
        )
    })?;
    if pem.tag == "PRIVATE KEY" {
        Ok(PrivateKey(pem.contents))
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("not labeled as a private key: '{}'", pem.tag),
        ))
    }
}

pub struct ServerConfig {
    /// The address on which to run the server.
    pub address: SocketAddr,
    /// The maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    pub max_length: usize,
    /// The number of bytes used to represent the length in length-delimited encoding.
    pub length_field_bytes: usize,
    /// The server's TLS certificate.
    pub certificate_chain: Vec<Certificate>,
    /// The server's TLS private key.
    pub private_key: PrivateKey,
}

pub async fn serve_while<Protocol, Input, Error, Interaction, Init, InteractionFut, InitFut>(
    config: ServerConfig,
    mut initialize: Init,
    interact: Interaction,
) -> Result<(), io::Error>
where
    Protocol: Session,
    Input: Send + 'static,
    Init: FnMut() -> InitFut,
    InitFut: Future<Output = Option<Input>>,
    Interaction: Fn(ServerChan<Protocol>, Input) -> InteractionFut + Send + Sync + 'static,
    InteractionFut: Future<Output = Result<(), Error>> + Send + 'static,
    Error: Send + Display + 'static,
{
    // Error handling task awaits the result of each spawned server task and logs any errors that
    // occur, as they occur
    let (result_tx, mut result_rx) = mpsc::channel(1024);
    let error_handler = tokio::spawn(async move {
        let mut results = FuturesUnordered::new();
        loop {
            tokio::select! {
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

    // Configure server-side TLS
    let mut tls_config = rustls::ServerConfig::new(rustls::NoClientAuth::new());
    tls_config
        .set_single_cert(config.certificate_chain, config.private_key)
        .map_err(|_error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid server certificate chain or private key",
            )
        })?;
    let tls_acceptor = TlsAcceptor::from(Arc::new(tls_config));

    println!("serving on: {:?}", config.address);
    let listener = TcpListener::bind(config.address).await?;

    // Make an acceptor for resumable connections
    let handshake = |chan: Chan<<Handshake as Session>::Dual, _, _>| async move {
        offer!(in chan {
            0 => {
                let (client_key, chan) = chan.recv().await?;
                let server_key = Uuid::new_v4();
                chan.send(server_key).await?.close();
                Ok((resume::ResumeKind::New, SessionKey { client_key, server_key }))
            },
            1 => {
                let (session_key, chan) = chan.recv().await?;
                chan.close();
                Ok::<_, HandshakeErr>((resume::ResumeKind::Existing, session_key))
            }
        })?
    };

    // Resume-handling acceptor to be shared between all connections
    let acceptor = Arc::new(resume::Acceptor::new(handshake, Protocol::default()));

    // Wrap the server function in an `Arc` to share it between threads
    let interact = Arc::new(interact);

    // Loop over incoming TCP connections until `continue_while()` returns `false`
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
                        length_delimited(tx, rx, config.length_field_bytes, config.max_length);
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

#[cfg(all(not(debug_assertions), feature = "allow_explicit_certificate_trust"))]
compile_error!(
    "crate cannot be built for release with the `allow_explicit_certificate_trust` feature enabled"
);

pub struct ClientConfig {
    /// The domain name of the server to which to connect.
    pub domain: DNSName,
    /// The port on the server to which to connect.
    pub port: u16,
    /// The number of bytes used to represent the length field in the length-delimited encoding.
    pub length_field_bytes: usize,
    /// The maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    pub max_length: usize,

    /// Also trust this certificate (FOR TESTING ONLY!). This field is only available in test
    /// builds, and should never be made to be available for a release build, because it adds the
    /// possibility of client misconfiguration to trust an arbitrary certificate.
    #[cfg(feature = "allow_explicit_certificate_trust")]
    pub trust_explicit_certificate: Option<Certificate>,
}

pub async fn connect<Protocol: Session>(
    config: ClientConfig,
) -> Result<
    ClientChan<Protocol>,
    retry::RetryError<std::convert::Infallible, io::Error, HandshakeErr>,
> {
    // Configure client-side TLS
    let mut tls_config = rustls::ClientConfig::new();
    tls_config
        .root_store
        .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);

    // Only non-release builds that explicitly request this capability via the feature, add the
    // auxiliary trusted certificate to the set of trusted certificates. In release builds, it is
    // not possible for the client to trust anyone other than the `webpki_roots::TLS_SERVER_ROOTS`
    // above.
    #[cfg(feature = "allow_explicit_certificate_trust")]
    if let Some(certificate) = config.trust_explicit_certificate {
        tls_config
            .root_store
            .add(&certificate)
            .map_err(|_error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid explicitly specified certificate",
                )
            })
            .map_err(retry::RetryError::ConnectError)?;
    };

    // Make a TLS connector
    let tls_connector = Arc::new(TlsConnector::from(Arc::new(tls_config)));

    // Address configuration
    let domain = Arc::new(config.domain);
    let port = config.port;
    let length_field_bytes = config.length_field_bytes;
    let max_length = config.max_length;

    // A closure that connects to the server we want to connect to
    let connect = move |()| {
        let domain = domain.clone();
        let tls_connector = tls_connector.clone();
        async move {
            // Resolve the domain name we wish to connect to
            let address_str: &str = AsRef::as_ref(&*domain);
            let mut addresses = tokio::net::lookup_host((address_str, port)).await?;

            // Attempt to connect to any of the socket addresses, succeeding on the first to work
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
            let tls_stream = tls_connector
                .connect(domain.as_ref().as_ref(), tcp_stream)
                .await?;

            let (rx, tx) = tokio::io::split(tls_stream);
            let (tx, rx) = length_delimited(tx, rx, length_field_bytes, max_length);
            Ok((tx, rx))
        }
    };

    // A closure that initializes a new connection with a handshake
    let init = |chan: Chan<Handshake, _, _>| async move {
        let client_key = Uuid::new_v4();
        let chan = chan.choose::<0>().await?.send(client_key).await?;
        let (server_key, chan) = chan.recv().await?;
        chan.close();
        Ok(SessionKey {
            client_key,
            server_key,
        })
    };

    // A closure that retries an existing connection with a handshake
    let retry = |key: SessionKey, chan: Chan<Handshake, _, _>| async move {
        chan.choose::<1>().await?.send(key).await?.close();
        Ok(())
    };

    let (_key, chan) = retry::Connector::new(connect, init, retry, Protocol::default())
        .connect(())
        .await?;

    Ok(chan)
}
