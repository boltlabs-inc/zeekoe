//! The server side of Zeekoe's transport layer.

use tracing::{error, info};
use {
    dialectic::prelude::*,
    dialectic_reconnect::resume,
    dialectic_tokio_serde::codec::LengthDelimitedCodec,
    dialectic_tokio_serde_bincode::{length_delimited, Bincode},
    futures::{stream::FuturesUnordered, Future, StreamExt},
    std::{
        fmt::Debug, io, marker::PhantomData, net::SocketAddr, path::Path, sync::Arc, time::Duration,
    },
    thiserror::Error,
    tokio::{net::TcpListener, select, sync::mpsc},
    tokio_rustls::{rustls, TlsAcceptor},
};

use super::{channel::TransportError, handshake, io_stream::IoStream, pem};

pub use super::channel::ServerChan as Chan;
pub use handshake::SessionKey;

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
    /// The maximum permissible number of pending retries.
    max_pending_retries: Option<usize>,
    /// The timeout after which broken connections will be garbage-collected.
    timeout: Option<Duration>,
    /// The session, from the *client's* perspective.
    client_session: PhantomData<fn() -> Protocol>,
}

type AcceptError = dialectic_reconnect::resume::AcceptError<
    SessionKey,
    dialectic_tokio_serde::Error<Bincode, Bincode, LengthDelimitedCodec, LengthDelimitedCodec>,
>;

#[derive(Debug, Error)]
pub enum ServerError<TaskError: 'static + Debug> {
    #[error(transparent)]
    Tcp(#[from] io::Error),
    #[error("{0:?}")]
    Task(TaskError),
    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),
    #[error("{0:?}")]
    Accept(AcceptError),
}

impl<Protocol> Default for Server<Protocol>
where
    Protocol: Session,
    <Protocol as Session>::Dual: Session,
{
    fn default() -> Self {
        Self {
            max_length: usize::MAX,
            length_field_bytes: 4,
            max_pending_retries: None,
            timeout: None,
            client_session: PhantomData,
        }
    }
}

impl<Protocol> Server<Protocol>
where
    Protocol: Session,
    <Protocol as Session>::Dual: Session,
{
    /// Create a new server using the given certificate chain and private key.
    pub fn new() -> Self {
        Self::default()
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
    pub async fn serve_while<
        Input,
        Error,
        Init,
        InitFut,
        Interaction,
        InteractionFut,
        TerminateFut,
    >(
        &self,
        address: impl Into<SocketAddr>,
        tls_config: Option<(&Path, &Path)>,
        mut initialize: Init,
        interact: Interaction,
        terminate: TerminateFut,
    ) -> Result<(), io::Error>
    where
        Input: Send + 'static,
        Error: Send + Debug + 'static,
        Init: FnMut() -> InitFut,
        InitFut: Future<Output = Option<Input>>,
        Interaction:
            Fn(SessionKey, Input, Chan<Protocol>) -> InteractionFut + Send + Sync + 'static,
        InteractionFut: Future<Output = Result<(), Error>> + Send + 'static,
        TerminateFut: Future<Output = ()> + Send + 'static,
    {
        let mut server_config = rustls::ServerConfig::new(rustls::NoClientAuth::new());

        // Optionally configure server-side TLS
        let tls_acceptor = match tls_config {
            None => None,
            Some((certificate_chain_path, private_key_path)) => {
                let certificate_chain = pem::read_certificates(certificate_chain_path)?;
                let private_key = pem::read_private_key(private_key_path)?;

                server_config
                    .set_single_cert(certificate_chain, private_key)
                    .map_err(|_error| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid server certificate chain or private key",
                        )
                    })?;
                Some(TlsAcceptor::from(Arc::new(server_config)))
            }
        };

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
        let (result_tx, result_rx) = mpsc::unbounded_channel();
        let error_join_handle = tokio::spawn(error_handler(result_rx));

        // Listen for the termination event and forward it to stop the server
        let (stop_server, mut recv_stop_server) = mpsc::channel(1);
        tokio::spawn(async move {
            terminate.await;
            stop_server.send(()).await.unwrap_or(());
        });

        // Wrap the server function in an `Arc` to share it between threads
        let interact = Arc::new(interact);

        // Bind to the address and serve
        let address = address.into();
        info!("serving on: {:?}", address);
        let listener = TcpListener::bind(address).await?;

        // Loop over incoming TCP connections until `initialize` returns `None`
        while let Some(input) = initialize().await {
            // If the termination future returns before a new connection, stop
            let accept_result = tokio::select! {
                result = listener.accept() => result,
                () = async { recv_stop_server.recv().await.unwrap_or(()) } => break,
            };

            match accept_result {
                Err(err) => result_tx.send(Err(err.into())).unwrap_or(()),
                Ok((tcp_stream, addr)) => {
                    tcp_stream.set_nodelay(true)?;

                    let io_stream = match tls_acceptor {
                        None => IoStream::from(tcp_stream),
                        Some(ref acceptor) => match acceptor.accept(tcp_stream).await {
                            Ok(tls_stream) => IoStream::from(tls_stream),
                            Err(e) => {
                                error!("Server TLS initialization error [{}]: {}", addr, e);
                                continue;
                            }
                        },
                    };

                    // Layer a length-delimmited bincode `Chan` over the TLS stream
                    let (rx, tx) = tokio::io::split(io_stream);
                    let (tx, rx) =
                        length_delimited(tx, rx, self.length_field_bytes, self.max_length);

                    let acceptor = acceptor.clone();
                    let interact = interact.clone();

                    // Run the interaction concurrently, or resume it if it's resuming an
                    // existing one
                    let join_handle = tokio::spawn(async move {
                        let result = acceptor.accept(tx, rx).await;
                        run_interaction::<Protocol, _, _, _, _>(result, input, interact).await
                    });

                    // Keep track of pending server task
                    result_tx.send(Ok(join_handle)).unwrap_or(());
                }
            }
        }

        error_join_handle.await?;
        Ok(())
    }
}

type JoinHandle<T> = tokio::task::JoinHandle<Result<(), ServerError<T>>>;

/// Run the interaction on a single connection.
async fn run_interaction<Protocol, Interaction, InteractionFut, Error, Input>(
    result: Result<(SessionKey, Option<Chan<Protocol>>), AcceptError>,
    input: Input,
    interact: Arc<Interaction>,
) -> Result<(), ServerError<Error>>
where
    Protocol: Session,
    <Protocol as Session>::Dual: Session,
    InteractionFut: Future<Output = Result<(), Error>> + Send + 'static,
    Interaction: Fn(SessionKey, Input, Chan<Protocol>) -> InteractionFut + Send + Sync + 'static,
    Error: Debug + 'static,
{
    match result.map_err(ServerError::Accept)? {
        (session_key, Some(chan)) => interact(session_key, input, chan)
            .await
            .map_err(ServerError::Task)?,
        (_session_key, None) => {
            // reconnected existing channel, nothing more to do
        }
    }
    Ok::<_, ServerError<Error>>(())
}

/// Handle errors on the provided `Receiver`.
async fn error_handler<Error: Debug>(
    mut result_rx: mpsc::UnboundedReceiver<Result<JoinHandle<Error>, ServerError<Error>>>,
) {
    let mut results = FuturesUnordered::new();
    loop {
        select! {
            Some(incoming) = result_rx.recv() => {
                match incoming {
                    Ok(join_handle) => results.push(async move {
                        let join_handle: JoinHandle<Error> = join_handle;
                        join_handle.await.map_err(ServerError::Join).and_then(|r| r)
                    }),
                    Err(err) => error!("{}", err),
                }
            },
            Some(result) = results.next() => {
                match result {
                    Ok(()) => {},
                    Err(err) => error!("{}", err),
                }
            },
            else => break,
        }
    }
}
