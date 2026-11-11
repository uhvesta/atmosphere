use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::error::{ClientError, WireError};
use crate::{frame, handshake, pb};

/// Items a stream can hold before the connection waits for its reader.
const STREAM_BUFFER: usize = 1024;

/// Sends requests on a connection and matches the answers to them.
pub struct Client {
    peer: pb::Hello,
    next_id: AtomicU64,
    outgoing: mpsc::Sender<pb::Envelope>,
    pending: Arc<Mutex<HashMap<u64, Pending>>>,
    reading: JoinHandle<()>,
    writing: JoinHandle<()>,
}

enum Pending {
    Call(oneshot::Sender<pb::response::Kind>),
    Stream {
        response: Option<oneshot::Sender<pb::response::Kind>>,
        items: mpsc::Sender<Result<pb::StreamItem, pb::Error>>,
    },
}

impl Client {
    /// Says hello on the connection and starts reading answers.
    pub async fn connect<R, W>(
        mut reader: R,
        mut writer: W,
        role: pb::Role,
        program_version: &str,
    ) -> Result<Self, WireError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let peer = handshake(&mut reader, &mut writer, role, program_version).await?;
        let compress = peer.accepts_zstd;
        let (outgoing, mut queue) = mpsc::channel::<pb::Envelope>(STREAM_BUFFER);
        let writing = tokio::spawn(async move {
            while let Some(envelope) = queue.recv().await {
                if frame::write(&mut writer, &envelope, compress)
                    .await
                    .is_err()
                {
                    return;
                }
            }
        });
        let pending: Arc<Mutex<HashMap<u64, Pending>>> = Arc::default();
        let reading = tokio::spawn(read_answers(reader, pending.clone()));
        Ok(Self {
            peer,
            next_id: AtomicU64::new(1),
            outgoing,
            pending,
            reading,
            writing,
        })
    }

    /// The other side's hello.
    pub fn peer(&self) -> &pb::Hello {
        &self.peer
    }

    /// Whether the connection is still open.
    pub fn is_connected(&self) -> bool {
        !self.reading.is_finished()
    }

    /// Sends a request that is answered with one response. An error response
    /// is returned as [`ClientError::Refused`].
    pub async fn call(
        &self,
        request: pb::request::Kind,
    ) -> Result<pb::response::Kind, ClientError> {
        let (answer, answered) = oneshot::channel();
        self.send(request, Pending::Call(answer)).await?;
        refused(answered.await.map_err(|_| ClientError::Disconnected)?)
    }

    /// Sends a request that is answered with a response and a stream.
    pub async fn stream(
        &self,
        request: pb::request::Kind,
    ) -> Result<(pb::response::Kind, Stream), ClientError> {
        let (answer, answered) = oneshot::channel();
        let (items, receiver) = mpsc::channel(STREAM_BUFFER);
        let id = self
            .send(
                request,
                Pending::Stream {
                    response: Some(answer),
                    items,
                },
            )
            .await?;
        let response = refused(answered.await.map_err(|_| ClientError::Disconnected)?)?;
        Ok((
            response,
            Stream {
                id,
                items: receiver,
                outgoing: self.outgoing.clone(),
            },
        ))
    }

    async fn send(&self, request: pb::request::Kind, pending: Pending) -> Result<u64, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, pending);
        let envelope = pb::Envelope {
            id,
            body: Some(pb::envelope::Body::Request(pb::Request {
                kind: Some(request),
            })),
        };
        if self.outgoing.send(envelope).await.is_err() {
            self.pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            return Err(ClientError::Disconnected);
        }
        Ok(id)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.reading.abort();
        self.writing.abort();
    }
}

fn refused(response: pb::response::Kind) -> Result<pb::response::Kind, ClientError> {
    match response {
        pb::response::Kind::Error(error) => Err(ClientError::Refused {
            code: error.code(),
            message: error.message,
        }),
        response => Ok(response),
    }
}

async fn read_answers<R: AsyncRead + Unpin>(
    mut reader: R,
    pending: Arc<Mutex<HashMap<u64, Pending>>>,
) {
    // Ends when the connection does; dropping what is pending tells each
    // caller it was disconnected.
    while let Ok(Some(envelope)) = frame::read(&mut reader).await {
        let id = envelope.id;
        let mut pending = pending.lock().unwrap_or_else(PoisonError::into_inner);
        match envelope.body {
            Some(pb::envelope::Body::Response(pb::Response {
                kind: Some(response),
            })) => match pending.remove(&id) {
                Some(Pending::Call(answer)) => {
                    let _ = answer.send(response);
                }
                Some(Pending::Stream {
                    response: Some(answer),
                    items,
                }) => {
                    let failed = matches!(response, pb::response::Kind::Error(_));
                    let _ = answer.send(response);
                    if !failed {
                        pending.insert(
                            id,
                            Pending::Stream {
                                response: None,
                                items,
                            },
                        );
                    }
                }
                Some(other) => {
                    pending.insert(id, other);
                }
                None => {}
            },
            Some(pb::envelope::Body::Item(item)) => {
                if let Some(Pending::Stream { items, .. }) = pending.get(&id) {
                    // A full stream would hold up every other answer, so a
                    // reader that falls this far behind loses the stream.
                    if items.try_send(Ok(item)).is_err() {
                        pending.remove(&id);
                    }
                }
            }
            Some(pb::envelope::Body::End(end)) => {
                if let Some(Pending::Stream { items, .. }) = pending.remove(&id)
                    && let Some(error) = end.error
                {
                    let _ = items.try_send(Err(error));
                }
            }
            _ => {}
        }
    }
    pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

/// The items of a streamed answer. Dropping it tells the other side to stop.
pub struct Stream {
    id: u64,
    items: mpsc::Receiver<Result<pb::StreamItem, pb::Error>>,
    outgoing: mpsc::Sender<pb::Envelope>,
}

impl Stream {
    /// The next item; None when the stream has ended normally or the
    /// connection closed.
    pub async fn next(&mut self) -> Option<Result<pb::StreamItem, pb::Error>> {
        self.items.recv().await
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let cancel = pb::Envelope {
            id: self.id,
            body: Some(pb::envelope::Body::Cancel(pb::Cancel {})),
        };
        // Best effort: if the connection is gone, so is the stream.
        let _ = self.outgoing.try_send(cancel);
    }
}
