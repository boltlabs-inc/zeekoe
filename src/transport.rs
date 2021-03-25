use {
    dialectic::prelude::*,
    dialectic_tokio_serde::{codec::LengthDelimitedCodec, SymmetricalChan},
    dialectic_tokio_serde_bincode::{length_delimited, Bincode},
    futures::{stream::FuturesUnordered, Future, StreamExt},
    std::{fmt::Display, fs::File, io, io::Read, net::SocketAddr, path::Path, sync::Arc},
    tokio::{
        io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf},
        net::{TcpListener, TcpStream},
        sync::mpsc,
    },
    tokio_rustls::{
        rustls::{self, Certificate, PrivateKey},
        webpki::DNSName,
        TlsAcceptor, TlsConnector,
    },
};

/// A *server-side* session-typed channel over TCP using length-delimited bincode encoding for
/// serialization.
pub type TlsServerChan<S> = SymmetricalChan<
    S,
    Bincode,
    LengthDelimitedCodec,
    WriteHalf<tokio_rustls::server::TlsStream<TcpStream>>,
    ReadHalf<tokio_rustls::server::TlsStream<TcpStream>>,
>;

/// A *client-side* session-typed channel over TCP using length-delimited bincode encoding for
/// serialization.
pub type TlsClientChan<S> = SymmetricalChan<
    S,
    Bincode,
    LengthDelimitedCodec,
    WriteHalf<tokio_rustls::client::TlsStream<TcpStream>>,
    ReadHalf<tokio_rustls::client::TlsStream<TcpStream>>,
>;

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

/// Wrap a raw TCP socket in a given session type, using the length delimited bincode transport
/// format/encoding.
fn wrap_split<S: Session, T: AsyncRead + AsyncWrite + Send>(
    stream: T,
    max_length: usize,
) -> SymmetricalChan<S, Bincode, LengthDelimitedCodec, WriteHalf<T>, ReadHalf<T>> {
    let (rx, tx) = tokio::io::split(stream);
    let (tx, rx) = length_delimited(tx, rx, 4, max_length);
    S::wrap(tx, rx)
}

pub struct ServerConfig {
    /// The address on which to run the server.
    pub address: SocketAddr,
    /// The maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    pub max_length: usize,
    /// The server's TLS certificate.
    pub certificate_chain: Vec<Certificate>,
    /// The server's TLS private key.
    pub private_key: PrivateKey,
}

pub async fn serve_while<Protocol, Input, Error, Interaction, Init, InteractionFut, InitFut>(
    config: ServerConfig,
    mut initialize: Init,
    mut interact: Interaction,
) -> Result<(), io::Error>
where
    Protocol: Session,
    Init: FnMut() -> InitFut,
    InitFut: Future<Output = Option<Input>>,
    Interaction: FnMut(TlsServerChan<Protocol>, Input) -> InteractionFut,
    InteractionFut: Future<Output = Result<(), Error>> + Send + 'static,
    Error: Send + Display + 'static,
{
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
    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    // Error handling task awaits the result of each spawned server task and logs any errors that
    // occur, as they occur
    let (result_tx, mut result_rx) = mpsc::channel(1024);
    let error_handler = tokio::spawn(async move {
        let mut results = FuturesUnordered::new();
        loop {
            let next_future = result_rx.recv();
            let next_result = results.next();
            tokio::pin!(next_future, next_result);
            tokio::select! {
                Some(incoming) = next_future => {
                    match incoming {
                        Ok((address, join_handle)) => results.push(async move { (address, join_handle.await) }),
                        Err((None, error)) => eprintln!("Server TCP initialization error: {}", error),
                        Err((Some(address), error)) => eprintln!("Server TLS initialization error [{}]: {}", address, error),
                    }
                },
                Some((address, result)) = next_result => {
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

    // Loop over incoming TCP connections until `continue_while()` returns `false`
    println!("serving on: {:?}", config.address);
    let listener = TcpListener::bind(config.address).await?;
    while let Some(input) = initialize().await {
        match listener.accept().await {
            Err(error) => result_tx.send(Err((None, error))).await.unwrap_or(()),
            Ok((tcp_stream, address)) => match acceptor.accept(tcp_stream).await {
                Err(error) => result_tx
                    .send(Err((Some(address), error)))
                    .await
                    .unwrap_or(()),
                Ok(tls_stream) => {
                    let interaction = interact(
                        wrap_split::<Protocol, _>(tls_stream, config.max_length),
                        input,
                    );
                    result_tx
                        .send(Ok((address, tokio::spawn(interaction))))
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
) -> Result<TlsClientChan<Protocol>, io::Error> {
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
        tls_config.root_store.add(&certificate).map_err(|_error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid explicitly specified certificate",
            )
        })?;
    };

    // Resolve the domain name we wish to connect to
    let address_str: &str = config.domain.as_ref().into();
    let mut addresses = tokio::net::lookup_host((address_str, config.port)).await?;

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
    let connector = TlsConnector::from(Arc::new(tls_config));
    let tls_stream = connector
        .connect(config.domain.as_ref(), tcp_stream)
        .await?;
    Ok(wrap_split::<Protocol, _>(tls_stream, config.max_length))
}
