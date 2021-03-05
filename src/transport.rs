use {
    dialectic::{
        backend::serde::{
            codec::LengthDelimitedCodec,
            format::{length_delimited_bincode, Bincode},
            SymmetricalChan,
        },
        prelude::*,
    },
    futures::Future,
    std::{
        io::{self, BufRead, Seek},
        marker::{self, PhantomData},
        net::SocketAddr,
        sync::Arc,
    },
    tokio::{
        io::{AsyncRead, AsyncWrite, ReadHalf, WriteHalf},
        net::{TcpListener, TcpStream},
    },
    tokio_rustls::{
        rustls::{self, Certificate, PrivateKey},
        webpki::DNSName,
        TlsAcceptor, TlsConnector,
    },
    x509_parser::{
        error::PEMError,
        pem::{self, Pem},
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

pub fn read_certificates(mut r: impl BufRead + Seek) -> Result<Vec<Certificate>, PEMError> {
    let mut certs = Vec::new();
    loop {
        match Pem::read(&mut r) {
            Ok((Pem { label, contents }, _)) if label == "CERTIFICATE" => {
                certs.push(Certificate(contents))
            }
            Ok(_) => {}
            Err(PEMError::IOError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(pem_error) => return Err(pem_error),
        }
    }
    Ok(certs)
}

pub fn read_private_key(mut r: impl BufRead + Seek) -> Result<PrivateKey, PEMError> {
    let mut private_key = None;
    loop {
        match Pem::read(&mut r) {
            Ok((Pem { label, contents }, _)) if label == "PRIVATE KEY" && private_key.is_none() => {
                private_key = Some(PrivateKey(contents));
            }
            Ok(_) => {}
            Err(PEMError::IOError(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
                if let Some(private_key) = private_key {
                    return Ok(private_key);
                } else {
                    return Err(PEMError::IOError(e));
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Wrap a raw TCP socket in a given session type, using the length delimited bincode transport
/// format/encoding.
fn wrap_split<S: Session, T: AsyncRead + AsyncWrite + marker::Send>(
    stream: T,
    max_length: usize,
) -> SymmetricalChan<S, Bincode, LengthDelimitedCodec, WriteHalf<T>, ReadHalf<T>> {
    let (rx, tx) = tokio::io::split(stream);
    let (tx, rx) = length_delimited_bincode(tx, rx, 4, max_length);
    S::wrap(tx, rx)
}

pub struct ServerConfig<Protocol: Session> {
    /// The address on which to run the server.
    pub address: SocketAddr,
    /// The maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    pub max_length: usize,
    /// The server's session type.
    pub protocol: PhantomData<Protocol>,
    /// The server's TLS certificate.
    pub certificate_chain: Vec<Certificate>,
    /// The server's TLS private key.
    pub private_key: PrivateKey,
}

pub async fn serve_forever<Protocol, Interaction, Fut>(
    config: ServerConfig<Protocol>,
    mut interact: Interaction,
) -> Result<(), io::Error>
where
    Protocol: Session,
    Interaction: FnMut(TlsServerChan<Protocol>) -> Fut,
    Fut: Future + marker::Send + 'static,
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

    // Loop over incoming TCP connections forever
    let listener = TcpListener::bind(config.address).await?;
    loop {
        let (tcp_stream, _address) = listener.accept().await?;
        let tls_stream = acceptor.accept(tcp_stream).await?;
        let interaction = interact(wrap_split::<Protocol, _>(tls_stream, config.max_length));
        tokio::spawn(async move {
            interaction.await;
        });
    }
}

#[cfg(all(not(debug_assertions), feature = "allow_explicit_certificate_trust"))]
compile_error!(
    "crate cannot be built for release with the `allow_explicit_certificate_trust` feature enabled"
);

pub struct ClientConfig<Protocol: Session> {
    /// The domain name of the server to which to connect.
    pub domain: DNSName,
    /// The port on the server to which to connect.
    pub port: u16,
    /// The maximum length, in bytes, of messages to permit in serialization/deserialization.
    /// Receiving or sending any larger messages will result in an error.
    pub max_length: usize,
    /// The server's session type.
    pub protocol: PhantomData<Protocol>,

    #[cfg(feature = "allow_explicit_certificate_trust")]
    /// Also trust this certificate (FOR TESTING ONLY!). This field is only available in test
    /// builds, and should never be made to be available for a release build, because it adds the
    /// possibility of client misconfiguration to trust an arbitrary certificate.
    pub trust_explicit_certificate: Option<Certificate>,
}

pub async fn connect<Protocol, Interaction, Fut>(
    config: ClientConfig<Protocol>,
) -> Result<TlsClientChan<Protocol>, io::Error>
where
    Protocol: Session,
{
    // Configure client-side TLS
    let mut tls_config = rustls::ClientConfig::new();
    tls_config
        .root_store
        .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);

    #[cfg(feature = "allow_explicit_certificate_trust")]
    // Only non-release builds that explicitly request this capability via the feature, add the
    // auxiliary trusted certificate to the set of trusted certificates. In release builds, it is
    // not possible for the client to trust anyone other than the `webpki_roots::TLS_SERVER_ROOTS`
    // above.
    if let Some(certificate) = config.trust_explicit_certificate {
        tls_config.root_store.add(&certificate).map_err(|_error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid explicitly specified certificate",
            )
        })?;
    };

    // Resolve the domain name we wish to connect to, and set the returned SocketAddr's port
    let address_str: &str = config.domain.as_ref().into();
    let mut address: SocketAddr = tokio::net::lookup_host(address_str)
        .await?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not resolve domain name"))?;
    address.set_port(config.port);

    // Wrap a TCP stream in a TLS connection, then wrap that in a Dialectic channel
    let connector = TlsConnector::from(Arc::new(tls_config));
    let tcp_stream = TcpStream::connect(address).await?;
    let tls_stream = connector
        .connect(config.domain.as_ref(), tcp_stream)
        .await?;
    Ok(wrap_split::<Protocol, _>(tls_stream, config.max_length))
}
