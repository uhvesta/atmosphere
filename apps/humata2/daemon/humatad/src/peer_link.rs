//! `humatad2 serve --stdio`: the peer link (GRPC.md §2, §8). The Mac starts
//! it over SSH or the machine's connect command (MP-7, MP-15) and speaks
//! gRPC on its stdin and stdout, one HTTP/2 connection for the link's life.
//!
//! PeerService is served here and on no socket, so nothing local to the
//! machine reaches it by connecting somewhere. Every call on the link is
//! the peer's (`Caller::Peer`): never the user or an agent, admin build or
//! not, and whatever metadata a request carries. Running this binary isn't
//! enough to be the peer, though: the host must prove it holds the
//! machine's enrollment key first (`peer`), and until it has, the link
//! answers only Hello, ProveHost and WhoAmI.
//!
//! Once the link is up, stderr goes to `logs/peer-link.log` in the state
//! folder: in a terminal Humata took over, stderr shares the stream, so a
//! late error line would corrupt it.
//!
//! This process opens no socket and no store: the socket's daemon keeps
//! the terminals and the store (PER-7). Until the two share them, the
//! services the Mac relays, and PeerService past the handshake, answer
//! "not built".

use std::fs::{File, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use humata2_grpc::humata::v1 as pb;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::oneshot;
use tokio_stream::StreamExt;
use tonic::service::InterceptorLayer;
use tonic::transport::Server;
use tonic::transport::server::Connected;
use tower::util::MapRequestLayer;

use crate::compression::Compression;
use crate::daemon::Daemon;
use crate::error::DaemonError;
use crate::link_proof;
use crate::not_built;
use crate::peer::{self, Gate, Handshake, Peer};
use crate::serve;
use crate::sync;

/// The link's log, in the state folder's `logs`.
const LOG_FILE: &str = "peer-link.log";

/// An HTTP/2 PING after the link has been this quiet (TR-9): the only
/// timer an idle link runs, and how a dead link is noticed (GRPC.md §2).
const KEEPALIVE: Duration = Duration::from_secs(30);

/// A PING unanswered this long ends the link.
const KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(20);

/// Prints the sync marker if asked, points stderr at the link's log, then
/// serves the peer link on stdio until it closes, a stop signal comes, or
/// the host's proof is wrong. `state` is the state folder, with a private
/// `logs` folder in it.
pub(crate) async fn run(
    state: &Path,
    machine: String,
    sync_nonce: Option<&str>,
) -> Result<(), DaemonError> {
    // Opened before the marker, so that failing to open it still reaches
    // whoever reads this terminal, before the stream starts.
    let log_path = state.join("logs").join(LOG_FILE);
    let log = open_log(&log_path)?;

    let mut output = tokio::io::stdout();
    if let Some(nonce) = sync_nonce
        && let Err(error) = sync::write_marker(&mut output, nonce).await
    {
        // The Mac hung up before the link came up: nothing to serve.
        return match error.kind() {
            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset => Ok(()),
            _ => Err(DaemonError::PeerLinkMarker { source: error }),
        };
    }
    rustix::stdio::dup2_stderr(&log).map_err(|source| DaemonError::PeerLinkStderr {
        path: log_path.clone(),
        source: source.into(),
    })?;
    drop(log);
    crate::log::say!("humatad2: peer link up (pid {})", std::process::id());

    let handshake = Arc::new(Handshake::new(
        state.join(link_proof::KEY_FILE),
        machine.clone(),
        Arc::new(Compression::new(state)),
    ));
    let stop = serve::stop_signal()?;
    let (input, output) = stdio(output);
    serve_link(input, output, handshake.clone(), machine, stop).await?;
    if handshake.host_proof_was_wrong() {
        return Err(DaemonError::PeerHostProofWrong);
    }
    Ok(())
}

type Input = Box<dyn AsyncRead + Unpin + Send>;
type Output = Box<dyn AsyncWrite + Unpin + Send>;

/// The link's stdin and stdout. When they're pipes, as SSH and most
/// connect commands give, they're read and written as the runtime's own
/// non-blocking fds, woken by the kernel; otherwise, such as on a terminal
/// or a socket, through tokio's stdio, which uses blocking threads.
fn stdio(stdout: tokio::io::Stdout) -> (Input, Output) {
    let input: Input = match pipe_end(rustix::stdio::stdin(), |fd| {
        tokio::net::unix::pipe::Receiver::from_owned_fd(fd)
    }) {
        Some(pipe) => Box::new(pipe),
        None => Box::new(tokio::io::stdin()),
    };
    let output: Output = match pipe_end(rustix::stdio::stdout(), |fd| {
        tokio::net::unix::pipe::Sender::from_owned_fd(fd)
    }) {
        Some(pipe) => Box::new(pipe),
        None => Box::new(stdout),
    };
    (input, output)
}

/// `open` on a copy of `fd`, or None when it isn't a pipe tokio can take.
fn pipe_end<T>(
    fd: std::os::fd::BorrowedFd<'_>,
    open: impl FnOnce(std::os::fd::OwnedFd) -> std::io::Result<T>,
) -> Option<T> {
    let copy = rustix::io::dup(fd).ok()?;
    open(copy).ok()
}

/// Serves the link on `input` and `output` until the connection ends,
/// `stop` resolves, or the host's proof is wrong.
async fn serve_link<R, W>(
    input: R,
    output: W,
    handshake: Arc<Handshake>,
    machine: String,
    stop: impl Future<Output = ()>,
) -> Result<(), DaemonError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (closed_tx, closed_rx) = oneshot::channel();
    let link = Link {
        input,
        output,
        _closed: closed_tx,
    };
    // One connection, then an incoming stream that never ends: the server
    // stops through the shutdown future below, once that connection is
    // gone, rather than at once when the stream runs out.
    let incoming =
        tokio_stream::once(Ok::<_, std::io::Error>(link)).chain(tokio_stream::pending());
    let done = {
        let handshake = handshake.clone();
        async move {
            tokio::select! {
                // Resolves when the connection drops the link: stdin
                // closed, the HTTP/2 connection ended, or a keepalive
                // PING went unanswered.
                _ = closed_rx => {}
                () = stop => {}
                // The answer to the wrong proof still goes out: the
                // server lets calls in flight finish before it closes.
                () = handshake.closing() => {}
            }
        }
    };

    // The first layer is the outer one: it marks the caller and the
    // methods open before the proof, which the gate can't see the path of.
    let router = Server::builder()
        .http2_keepalive_interval(Some(KEEPALIVE))
        .http2_keepalive_timeout(Some(KEEPALIVE_TIMEOUT))
        .layer(MapRequestLayer::new(peer::mark))
        .layer(InterceptorLayer::new(Gate::new(handshake.clone())))
        .add_service(pb::daemon_service_server::DaemonServiceServer::new(Daemon::new(
            machine,
            pb::DaemonRole::Machine,
        )))
        .add_service(pb::peer_service_server::PeerServiceServer::new(Peer::new(handshake)));
    let router = not_built::register_peer_link(router);
    router
        .serve_with_incoming_shutdown(incoming, done)
        .await
        .map_err(|source| DaemonError::Serve { source })
}

/// The link's log, appended to and 0600.
fn open_log(path: &Path) -> Result<File, DaemonError> {
    let failed = |source| DaemonError::PeerLinkLog {
        path: path.to_owned(),
        source,
    };
    let log = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)
        .map_err(failed)?;
    log.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(failed)?;
    Ok(log)
}

/// stdin and stdout as one connection. Dropping it, which the server does
/// when the connection ends, drops `_closed` and so ends the server.
struct Link<R, W> {
    input: R,
    output: W,
    _closed: oneshot::Sender<()>,
}

impl<R, W> Connected for Link<R, W> {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl<R: AsyncRead + Unpin, W: Unpin> AsyncRead for Link<R, W> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.input).poll_read(context, buf)
    }
}

impl<R: Unpin, W: AsyncWrite + Unpin> AsyncWrite for Link<R, W> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.output).poll_write(context, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.output).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.output).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    //! The link over an in-memory pipe, on tokio's paused clock: time
    //! moves only when every task waits, so these count what an idle link
    //! does in an hour without taking one (TR-9).

    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;
    use tokio::time::Instant;

    use super::serve_link;
    use crate::compression::Compression;
    use crate::error::DaemonError;
    use crate::link_proof;
    use crate::peer::Handshake;

    const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
    const DATA: u8 = 0;
    const HEADERS: u8 = 1;
    const SETTINGS: u8 = 4;
    const PING: u8 = 6;
    const WINDOW_UPDATE: u8 = 8;
    const ACK: u8 = 1;
    const HOUR: Duration = Duration::from_secs(3600);

    struct Frame {
        kind: u8,
        flags: u8,
        payload: Vec<u8>,
    }

    fn frame(kind: u8, flags: u8, payload: &[u8]) -> Vec<u8> {
        let length = u32::try_from(payload.len()).unwrap().to_be_bytes();
        let mut bytes = vec![length[1], length[2], length[3], kind, flags, 0, 0, 0, 0];
        bytes.extend_from_slice(payload);
        bytes
    }

    async fn read_frame(from: &mut ReadHalf<DuplexStream>) -> Option<Frame> {
        let mut head = [0u8; 9];
        from.read_exact(&mut head).await.ok()?;
        let length = u32::from_be_bytes([0, head[0], head[1], head[2]]) as usize;
        let mut payload = vec![0; length];
        from.read_exact(&mut payload).await.ok()?;
        Some(Frame {
            kind: head[3],
            flags: head[4],
            payload,
        })
    }

    struct Peer {
        from: ReadHalf<DuplexStream>,
        to: WriteHalf<DuplexStream>,
        served: JoinHandle<Result<(), DaemonError>>,
        _stop: oneshot::Sender<()>,
    }

    /// A link served on a pipe, and the host's end of it after the
    /// HTTP/2 preface: a bare client that answers what it's told to.
    async fn link(dir: &Path) -> Peer {
        let (host, machine) = tokio::io::duplex(64 * 1024);
        let (input, output) = tokio::io::split(machine);
        let handshake = Arc::new(Handshake::new(
            dir.join(link_proof::KEY_FILE),
            "ma_test".to_owned(),
            Arc::new(Compression::new(dir)),
        ));
        let (stop, stopped) = oneshot::channel::<()>();
        let served = tokio::spawn(serve_link(input, output, handshake, "ma_test".to_owned(), async {
            let _ = stopped.await;
        }));
        let (from, mut to) = tokio::io::split(host);
        to.write_all(PREFACE).await.unwrap();
        to.write_all(&frame(SETTINGS, 0, &[])).await.unwrap();
        Peer {
            from,
            to,
            served,
            _stop: stop,
        }
    }

    /// What the machine sent in `span`, beyond the HTTP/2 handshake: PINGs
    /// (answered when `answer_pings`) and every other frame's type.
    async fn watch(peer: &mut Peer, span: Duration, answer_pings: bool) -> (usize, Vec<u8>) {
        let until = Instant::now() + span;
        let mut pings = 0;
        let mut others = Vec::new();
        while let Ok(Some(next)) = tokio::time::timeout_at(until, read_frame(&mut peer.from)).await {
            match (next.kind, next.flags & ACK) {
                (SETTINGS, 0) => peer.to.write_all(&frame(SETTINGS, ACK, &[])).await.unwrap(),
                (SETTINGS | WINDOW_UPDATE, _) => {}
                (PING, 0) => {
                    pings += 1;
                    if answer_pings {
                        peer.to.write_all(&frame(PING, ACK, &next.payload)).await.unwrap();
                    }
                }
                (kind, _) => others.push(kind),
            }
        }
        (pings, others)
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_link_sends_only_a_keepalive_ping_at_most_every_30_seconds() {
        let dir = tempfile::tempdir().unwrap();
        let mut peer = link(dir.path()).await;

        let (pings, others) = watch(&mut peer, HOUR, true).await;

        assert!(others.is_empty(), "frames other than PING: {others:?}");
        assert!(pings >= 1, "the link sends keepalive PINGs");
        assert!(pings <= 120, "{pings} PINGs in an hour");
        assert!(!peer.served.is_finished(), "an answered PING keeps the link up");
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_whose_pings_go_unanswered_ends() {
        let dir = tempfile::tempdir().unwrap();
        let mut peer = link(dir.path()).await;

        let (_, others) = watch(&mut peer, Duration::from_secs(120), false).await;

        assert!(
            others.iter().all(|kind| ![DATA, HEADERS].contains(kind)),
            "the link sent {others:?}"
        );
        let served = tokio::time::timeout(Duration::from_secs(1), peer.served).await;
        assert!(matches!(served, Ok(Ok(Ok(())))), "the link ended");
    }
}
