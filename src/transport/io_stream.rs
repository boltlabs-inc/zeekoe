use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::server::TlsStream;

pub enum IoStream {
    Tcp(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl AsyncRead for IoStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            IoStream::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            IoStream::Tls(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for IoStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        match self.get_mut() {
            IoStream::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            IoStream::Tls(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            IoStream::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            IoStream::Tls(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match self.get_mut() {
            IoStream::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            IoStream::Tls(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

impl From<TcpStream> for IoStream {
    fn from(stream: TcpStream) -> Self {
        IoStream::Tcp(stream)
    }
}

impl From<TlsStream<TcpStream>> for IoStream {
    fn from(stream: TlsStream<TcpStream>) -> Self {
        IoStream::Tls(Box::new(stream))
    }
}
