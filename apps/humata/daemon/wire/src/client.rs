use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    waiting: Arc<Mutex<Waiting>>,
    reading: JoinHandle<()>,
    writing: JoinHandle<()>,
}

/// The requests still owed an answer, and whether any more can be.
#[derive(Default)]
struct Waiting {
    pending: HashMap<u64, Pending>,
    /// Set when reading or writing ended. A request made after that would
    /// never be answered, so it is refused instead of kept.
    closed: bool,
}

impl Waiting {
    /// Ends the connection for its callers: dropping what is pending tells
    /// each it was disconnected, and nothing more is taken.
    fn close(waiting: &Mutex<Self>) {
        let mut waiting = waiting.lock().unwrap_or_else(PoisonError::into_inner);
        waiting.closed = true;
        waiting.pending.clear();
    }
}

enum Pending {
    Call(oneshot::Sender<pb::response::Kind>),
    Stream {
        response: Option<oneshot::Sender<pb::response::Kind>>,
        items: mpsc::Sender<Result<pb::StreamItem, pb::Error>>,
        /// Set when the reader fell too far behind and the stream was let go.
        lagged: Arc<AtomicBool>,
    },
}

impl Client {
    /// Says hello on the connection and starts reading answers.
    pub async fn connect<R, W>(
        reader: R,
        writer: W,
        role: pb::Role,
        program_version: &str,
    ) -> Result<Self, WireError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Self::connect_as(
            reader,
            writer,
            role,
            program_version,
            pb::ClientKind::Unspecified,
        )
        .await
    }

    /// As [`Client::connect`], saying in the hello which client of the
    /// daemon this is: the command line or an agent's MCP server.
    pub async fn connect_as<R, W>(
        mut reader: R,
        mut writer: W,
        role: pb::Role,
        program_version: &str,
        client_kind: pb::ClientKind,
    ) -> Result<Self, WireError>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let peer = handshake(&mut reader, &mut writer, role, program_version, client_kind).await?;
        let compress = peer.accepts_zstd;
        let (outgoing, mut queue) = mpsc::channel::<pb::Envelope>(STREAM_BUFFER);
        let waiting: Arc<Mutex<Waiting>> = Arc::default();
        let written = waiting.clone();
        let writing = tokio::spawn(async move {
            while let Some(envelope) = queue.recv().await {
                if frame::write(&mut writer, &envelope, compress)
                    .await
                    .is_err()
                {
                    Waiting::close(&written);
                    return;
                }
            }
        });
        let reading = tokio::spawn(read_answers(reader, waiting.clone()));
        Ok(Self {
            peer,
            next_id: AtomicU64::new(1),
            outgoing,
            waiting,
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
        !self
            .waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .closed
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
        let lagged = Arc::new(AtomicBool::new(false));
        let id = self
            .send(
                request,
                Pending::Stream {
                    response: Some(answer),
                    items,
                    lagged: lagged.clone(),
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
                lagged,
            },
        ))
    }

    async fn send(&self, request: pb::request::Kind, pending: Pending) -> Result<u64, ClientError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut waiting = self.waiting.lock().unwrap_or_else(PoisonError::into_inner);
            if waiting.closed {
                return Err(ClientError::Disconnected);
            }
            waiting.pending.insert(id, pending);
        }
        let envelope = pb::Envelope {
            id,
            body: Some(pb::envelope::Body::Request(pb::Request {
                kind: Some(request),
            })),
        };
        if self.outgoing.send(envelope).await.is_err() {
            self.waiting
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pending
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

async fn read_answers<R: AsyncRead + Unpin>(mut reader: R, waiting: Arc<Mutex<Waiting>>) {
    // Ends when the connection does; dropping what is pending tells each
    // caller it was disconnected.
    while let Ok(Some(envelope)) = frame::read(&mut reader).await {
        let id = envelope.id;
        let mut waiting = waiting.lock().unwrap_or_else(PoisonError::into_inner);
        let pending = &mut waiting.pending;
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
                    lagged,
                }) => {
                    let failed = matches!(response, pb::response::Kind::Error(_));
                    let _ = answer.send(response);
                    if !failed {
                        pending.insert(
                            id,
                            Pending::Stream {
                                response: None,
                                items,
                                lagged,
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
                if let Some(Pending::Stream { items, lagged, .. }) = pending.get(&id) {
                    // A full stream would hold up every other answer, so a
                    // reader that falls this far behind loses the stream,
                    // and is told it fell behind rather than was cut off.
                    match items.try_send(Ok(item)) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            lagged.store(true, Ordering::Relaxed);
                            pending.remove(&id);
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            pending.remove(&id);
                        }
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
    Waiting::close(&waiting);
}

/// The items of a streamed answer. Dropping it tells the other side to stop.
pub struct Stream {
    id: u64,
    items: mpsc::Receiver<Result<pb::StreamItem, pb::Error>>,
    outgoing: mpsc::Sender<pb::Envelope>,
    lagged: Arc<AtomicBool>,
}

impl Stream {
    /// The next item; None when the stream has ended normally or the
    /// connection closed.
    pub async fn next(&mut self) -> Option<Result<pb::StreamItem, pb::Error>> {
        self.items.recv().await
    }

    /// Whether the stream ended because its reader fell too far behind,
    /// rather than because it ended or the connection closed. Asking again
    /// starts it over, which for a terminal replays what it missed.
    pub fn fell_behind(&self) -> bool {
        self.lagged.load(Ordering::Relaxed)
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
