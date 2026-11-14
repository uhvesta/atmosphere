use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use crate::error::WireError;
use crate::{error, frame, handshake, pb};

/// How the service answers a request.
pub enum Reply {
    Response(pb::response::Kind),
    /// A response, then items until the channel closes (the stream ended) or
    /// sends an error (it ended with that error).
    Stream {
        response: pb::response::Kind,
        items: mpsc::Receiver<Result<pb::StreamItem, pb::Error>>,
    },
}

pub type ServiceFuture = Pin<Box<dyn Future<Output = Reply> + Send>>;

/// Answers requests. Each request is handled on its own task.
pub trait Service: Send + Sync + 'static {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture;

    /// Told the other side's hello, once, before any of its requests is
    /// handled. Does nothing unless a service wants what it says.
    fn greeted(&self, _peer: &pb::Hello) {}
}

/// Envelopes waiting for the writer.
const OUTGOING: usize = 1024;

/// Serves `service` on a connection until the other side closes it. A request
/// still being answered then is answered to the end, so a step that changes
/// state isn't cut off half way; only its reply and any stream are dropped.
/// A request that only reads ([`abandonable`]) is stopped instead, as it is
/// when the other side cancels it: nobody waits for what it would find.
pub async fn serve<R, W, S>(
    mut reader: R,
    mut writer: W,
    role: pb::Role,
    program_version: &str,
    service: Arc<S>,
) -> Result<pb::Hello, WireError>
where
    R: AsyncRead + Unpin + Send,
    W: AsyncWrite + Unpin + Send + 'static,
    S: Service + ?Sized,
{
    let peer = handshake(
        &mut reader,
        &mut writer,
        role,
        program_version,
        pb::ClientKind::Unspecified,
    )
    .await?;
    service.greeted(&peer);
    let compress = peer.accepts_zstd;
    let (outgoing, mut queue) = mpsc::channel::<pb::Envelope>(OUTGOING);
    let writing = tokio::spawn(async move {
        while let Some(envelope) = queue.recv().await {
            frame::write(&mut writer, &envelope, compress).await?;
        }
        Ok::<_, WireError>(())
    });

    let tasks: Arc<Mutex<HashMap<u64, AbortHandle>>> = Arc::default();
    let result = loop {
        let envelope = match frame::read(&mut reader).await {
            Ok(Some(envelope)) => envelope,
            Ok(None) => break Ok(peer),
            Err(error) => break Err(error),
        };
        let id = envelope.id;
        match envelope.body {
            Some(pb::envelope::Body::Request(pb::Request { kind: Some(kind) })) => {
                // Locked first, so a task that finishes at once can't remove
                // itself before it is added.
                let mut running = tasks.lock().unwrap_or_else(PoisonError::into_inner);
                let task = tokio::spawn(answer(
                    id,
                    kind,
                    service.clone(),
                    outgoing.clone(),
                    tasks.clone(),
                ));
                running.insert(id, task.abort_handle());
            }
            Some(pb::envelope::Body::Request(pb::Request { kind: None })) => {
                let response = error(
                    pb::ErrorCode::InvalidRequest,
                    "the request names nothing to do; update Humata so both sides are the same version",
                );
                let _ = outgoing.send(respond(id, response)).await;
            }
            Some(pb::envelope::Body::Cancel(_)) => {
                if let Some(task) = tasks
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .remove(&id)
                {
                    task.abort();
                }
            }
            // Hellos after the first, and anything only a client receives.
            _ => {}
        }
    };
    for (_, task) in tasks.lock().unwrap_or_else(PoisonError::into_inner).drain() {
        task.abort();
    }
    drop(outgoing);
    // The writer fails if the other side has gone, which is how it ends.
    let _ = writing.await;
    result
}

async fn answer<S: Service + ?Sized>(
    id: u64,
    kind: pb::request::Kind,
    service: Arc<S>,
    outgoing: mpsc::Sender<pb::Envelope>,
    tasks: Arc<Mutex<HashMap<u64, AbortHandle>>>,
) {
    // On its own task, so aborting this one (the connection closed, or the
    // request or its stream was cancelled) never stops the service half way
    // through a change; a request that only reads is stopped along with it.
    let abandonable = abandonable(&kind);
    let handling = tokio::spawn(service.handle(kind));
    let _stop = abandonable.then(|| StopOnDrop(handling.abort_handle()));
    let reply = match handling.await {
        Ok(reply) => reply,
        Err(_) => Reply::Response(error(
            pb::ErrorCode::BackendFailed,
            "Humata stopped while answering; try again, and if it keeps happening, report it with the daemon's log",
        )),
    };
    match reply {
        Reply::Response(response) => {
            let _ = outgoing.send(respond(id, response)).await;
        }
        Reply::Stream {
            response,
            mut items,
        } => {
            if outgoing.send(respond(id, response)).await.is_err() {
                return;
            }
            let mut end = pb::StreamEnd { error: None };
            while let Some(item) = items.recv().await {
                match item {
                    Ok(item) => {
                        let envelope = pb::Envelope {
                            id,
                            body: Some(pb::envelope::Body::Item(item)),
                        };
                        if outgoing.send(envelope).await.is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        end.error = Some(error);
                        break;
                    }
                }
            }
            let _ = outgoing
                .send(pb::Envelope {
                    id,
                    body: Some(pb::envelope::Body::End(end)),
                })
                .await;
        }
    }
    tasks
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&id);
}

fn respond(id: u64, kind: pb::response::Kind) -> pb::Envelope {
    pb::Envelope {
        id,
        body: Some(pb::envelope::Body::Response(pb::Response {
            kind: Some(kind),
        })),
    }
}

/// Aborts a task when dropped: when the task waiting on it is aborted.
struct StopOnDrop(AbortHandle);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Whether a request's handler may be stopped part way when nobody waits for
/// its answer any more: it only reads, so stopping it leaves nothing half
/// changed. Every other request, including one not listed here, runs to the
/// end. The app's own list of what it stops waiting for (`onlyReads` in
/// clients/daemon_live) matches this one, apart from refreshing pull
/// requests, which the app stops waiting for but which runs to the end
/// here, as it records what it read and cleans up merged sessions.
fn abandonable(request: &pb::request::Kind) -> bool {
    use pb::request::Kind;
    match request {
        Kind::ListPullRequests(list) => !list.refresh,
        Kind::ListProjects(_)
        | Kind::ListSessions(_)
        | Kind::ListImportable(_)
        | Kind::ListModels(_)
        | Kind::ListSkipped(_)
        | Kind::ReadHistory(_)
        | Kind::ReadImage(_)
        | Kind::GetChatCommands(_)
        | Kind::ListProposals(_)
        | Kind::ReadChanges(_)
        | Kind::ListThreads(_)
        | Kind::ListFiles(_)
        | Kind::ReadFile(_)
        | Kind::ListFileVersions(_)
        | Kind::ListFolder(_)
        | Kind::ListItems(_)
        | Kind::ReadBoardLog(_)
        | Kind::ListInbox(_)
        | Kind::ListApprovals(_)
        | Kind::GetStorage(_)
        | Kind::ListSkills(_)
        | Kind::ListPlugins(_)
        | Kind::GetSessionDefaults(_)
        | Kind::ListHosts(_)
        | Kind::ListSshHosts(_)
        | Kind::ListRecoveries(_)
        | Kind::ListTerminals(_)
        | Kind::ListForwards(_)
        | Kind::Search(_)
        | Kind::GetPullRefresh(_)
        | Kind::GetAgentAccess(_)
        | Kind::ListSavedRepositories(_)
        | Kind::ListRepositoryOwners(_) => true,
        _ => false,
    }
}
