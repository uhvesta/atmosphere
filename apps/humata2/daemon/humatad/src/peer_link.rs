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

use humata2_grpc::humata::v1 as pb;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, Stdin, Stdout};
use tokio::sync::oneshot;
use tokio_stream::StreamExt;
use tonic::service::InterceptorLayer;
use tonic::transport::Server;
use tonic::transport::server::Connected;
use tower::util::MapRequestLayer;

use crate::daemon::Daemon;
use crate::error::DaemonError;
use crate::link_proof;
use crate::not_built;
use crate::peer::{self, Gate, Handshake, Peer};
use crate::serve;
use crate::sync;

/// The link's log, in the state folder's `logs`.
const LOG_FILE: &str = "peer-link.log";

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
    eprintln!("humatad2: peer link up (pid {})", std::process::id());

    let (closed_tx, closed_rx) = oneshot::channel();
    let link = Link {
        input: tokio::io::stdin(),
        output,
        _closed: closed_tx,
    };
    // One connection, then an incoming stream that never ends: the server
    // stops through the shutdown future below, once that connection is
    // gone, rather than at once when the stream runs out.
    let incoming =
        tokio_stream::once(Ok::<_, std::io::Error>(link)).chain(tokio_stream::pending());
    let handshake = Arc::new(Handshake::new(
        state.join(link_proof::KEY_FILE),
        machine.clone(),
    ));
    let stop = serve::stop_signal()?;
    let done = {
        let handshake = handshake.clone();
        async move {
            tokio::select! {
                // Resolves when the connection drops the link: stdin
                // closed, or the HTTP/2 connection ended.
                _ = closed_rx => {}
                _ = stop => {}
                // The answer to the wrong proof still goes out: the
                // server lets calls in flight finish before it closes.
                _ = handshake.closing() => {}
            }
        }
    };

    // The first layer is the outer one: it marks the caller and the
    // methods open before the proof, which the gate can't see the path of.
    let router = Server::builder()
        .layer(MapRequestLayer::new(peer::mark))
        .layer(InterceptorLayer::new(Gate::new(handshake.clone())))
        .add_service(pb::daemon_service_server::DaemonServiceServer::new(Daemon::new(
            machine,
            pb::DaemonRole::Machine,
        )))
        .add_service(pb::peer_service_server::PeerServiceServer::new(Peer::new(
            handshake.clone(),
        )));
    let router = not_built::register_peer_link(router);
    router
        .serve_with_incoming_shutdown(incoming, done)
        .await
        .map_err(|source| DaemonError::Serve { source })?;
    if handshake.host_proof_was_wrong() {
        return Err(DaemonError::PeerHostProofWrong);
    }
    Ok(())
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
struct Link {
    input: Stdin,
    output: Stdout,
    _closed: oneshot::Sender<()>,
}

impl Connected for Link {
    type ConnectInfo = ();

    fn connect_info(&self) -> Self::ConnectInfo {}
}

impl AsyncRead for Link {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.input).poll_read(context, buf)
    }
}

impl AsyncWrite for Link {
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
