//! `humatad2 serve --stdio`: the peer link (GRPC.md §2, §8). The Mac starts
//! it over SSH or the machine's connect command (MP-7, MP-15) and speaks
//! gRPC on its stdin and stdout, one HTTP/2 connection for the link's life.
//!
//! PeerService is served here and on no socket, so nothing local to the
//! machine reaches it by connecting somewhere. Every call on the link is
//! the peer's (`Caller::Peer`): never the user or an agent, admin build or
//! not, and whatever metadata a request carries.
//!
//! This process opens no socket and no store: the socket's daemon keeps
//! the terminals and the store (PER-7). Until the two share them, the
//! services the Mac relays, and all of PeerService, answer "not built".

use std::pin::Pin;
use std::task::{Context, Poll};

use humata2_grpc::humata::v1 as pb;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf, Stdin, Stdout};
use tokio::sync::oneshot;
use tokio_stream::StreamExt;
use tonic::transport::Server;
use tonic::transport::server::Connected;
use tower::util::MapRequestLayer;

use crate::daemon::Daemon;
use crate::error::DaemonError;
use crate::identity;
use crate::not_built;
use crate::serve;
use crate::sync;

/// Prints the sync marker if asked, then serves the peer link on stdio
/// until it closes or a stop signal comes.
pub(crate) async fn run(machine: String, sync_nonce: Option<&str>) -> Result<(), DaemonError> {
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
    let stop = serve::stop_signal()?;
    let done = async move {
        tokio::select! {
            // Resolves when the connection drops the link: stdin closed, or
            // the HTTP/2 connection ended.
            _ = closed_rx => {}
            _ = stop => {}
        }
    };

    let router = Server::builder()
        .layer(MapRequestLayer::new(identity::mark_peer))
        .add_service(pb::daemon_service_server::DaemonServiceServer::new(Daemon::new(
            machine,
            pb::DaemonRole::Machine,
        )));
    let router = not_built::register_peer_link(router);
    router
        .serve_with_incoming_shutdown(incoming, done)
        .await
        .map_err(|source| DaemonError::Serve { source })
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
