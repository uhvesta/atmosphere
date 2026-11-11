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
}

/// Envelopes waiting for the writer.
const OUTGOING: usize = 1024;

/// Serves `service` on a connection until the other side closes it. Requests
/// still being answered then are dropped.
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
    let peer = handshake(&mut reader, &mut writer, role, program_version).await?;
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
    match service.handle(kind).await {
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
