use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{UnixListener, UnixStream};
use tokio_stream::Stream;
use tonic::transport::server::{Connected, UdsConnectInfo};

use crate::sys::{check, AuditToken, Requirement};

/// The id the next accepted connection gets.
static NEXT_CONN: AtomicU64 = AtomicU64::new(1);

/// What the daemon learns about a connection when it is accepted.
#[derive(Clone, Debug)]
pub struct PeerInfo {
    /// What tonic reports for any Unix socket peer.
    pub uds: UdsConnectInfo,
    /// True when the peer is the signed Humata app.
    pub app: bool,
    /// Why the peer is not the app, for the daemon's log. `None` when it is the
    /// app, or when the build has no requirement to check against.
    pub not_app: Option<String>,
    /// Which accepted connection this is: unique within the process, never
    /// reused. Per-connection state (D144) is kept by it, since tonic clones
    /// this value for every request.
    pub conn: u64,
}

/// An accepted connection that carries its [`PeerInfo`].
pub struct CheckedStream {
    inner: UnixStream,
    info: PeerInfo,
}

impl Connected for CheckedStream {
    type ConnectInfo = PeerInfo;

    fn connect_info(&self) -> PeerInfo {
        self.info.clone()
    }
}

impl AsyncRead for CheckedStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for CheckedStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }
}

struct Incoming {
    listener: UnixListener,
    requirement: Option<Requirement>,
}

impl Stream for Incoming {
    type Item = io::Result<CheckedStream>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let (stream, _) = match this.listener.poll_accept(cx) {
            Poll::Ready(Ok(accepted)) => accepted,
            Poll::Ready(Err(e)) => return Poll::Ready(Some(Err(e))),
            Poll::Pending => return Poll::Pending,
        };
        let (app, not_app) = verdict(&stream, this.requirement.as_ref());
        let uds = <UnixStream as Connected>::connect_info(&stream);
        Poll::Ready(Some(Ok(CheckedStream {
            inner: stream,
            info: PeerInfo {
                uds,
                app,
                not_app,
                conn: NEXT_CONN.fetch_add(1, Ordering::Relaxed),
            },
        })))
    }
}

fn verdict(stream: &UnixStream, requirement: Option<&Requirement>) -> (bool, Option<String>) {
    use std::os::fd::AsFd;
    let Some(requirement) = requirement else {
        return (false, None);
    };
    let token = match AuditToken::of_peer(stream.as_fd()) {
        Ok(token) => token,
        Err(e) => return (false, Some(format!("couldn't read the peer's audit token: {e}"))),
    };
    match check(&token, requirement) {
        Ok(()) => (true, None),
        Err(e) => (false, Some(e.to_string())),
    }
}

/// Accepts like `UnixListenerStream`, and decides once per connection whether the
/// peer is the app. A failed check does not fail the stream: the connection is
/// served, just not as the app. With no `requirement` nothing is the app.
pub fn incoming(
    listener: UnixListener,
    requirement: Option<Requirement>,
) -> impl Stream<Item = io::Result<CheckedStream>> + Send + 'static {
    Incoming {
        listener,
        requirement,
    }
}
