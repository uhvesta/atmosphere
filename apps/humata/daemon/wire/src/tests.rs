use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncWriteExt, DuplexStream, duplex, split};
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::{Client, ClientError, Reply, Service, ServiceFuture, WireError, error, pb, serve};

/// Answers ListSessions with as many sessions as the last SendMessage's text
/// is long, refuses an unknown session, and streams WatchSession: three
/// events for "three", an error after one for "broken", and forever for
/// "forever" (reporting on `cancelled` once the stream is dropped).
#[derive(Default)]
struct Echo {
    cancelled: Mutex<Option<oneshot::Sender<()>>>,
    /// SendMessage to "gated" waits for this, then reports on `finished`.
    gate: Mutex<Option<oneshot::Receiver<()>>>,
    finished: Mutex<Option<oneshot::Sender<()>>>,
}

/// Events a "flood" watch sends: more than a stream holds unread.
const FLOOD: usize = 4096;

fn event(id: &str) -> pb::StreamItem {
    pb::StreamItem {
        kind: Some(pb::stream_item::Kind::SessionEvent(pb::SessionEvent {
            id: id.to_owned(),
            ..Default::default()
        })),
    }
}

impl Service for Echo {
    fn handle(self: Arc<Self>, request: pb::request::Kind) -> ServiceFuture {
        Box::pin(async move {
            match request {
                pb::request::Kind::SendMessage(message) if message.session_id == "slow" => {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    Reply::Response(pb::response::Kind::MessageSent(pb::MessageSent {}))
                }
                pb::request::Kind::SendMessage(message) if message.session_id == "gated" => {
                    let gate = self.gate.lock().await.take();
                    if let Some(gate) = gate {
                        let _ = gate.await;
                    }
                    if let Some(finished) = self.finished.lock().await.take() {
                        let _ = finished.send(());
                    }
                    Reply::Response(pb::response::Kind::MessageSent(pb::MessageSent {}))
                }
                pb::request::Kind::SendMessage(message) if message.session_id == "known" => {
                    Reply::Response(pb::response::Kind::MessageSent(pb::MessageSent {}))
                }
                pb::request::Kind::SendMessage(_) => {
                    Reply::Response(error(pb::ErrorCode::UnknownSession, "no such session"))
                }
                pb::request::Kind::WatchSession(watch) => {
                    let (sender, items) = mpsc::channel(8);
                    let cancelled = self.cancelled.lock().await.take();
                    let finished = self.finished.lock().await.take();
                    tokio::spawn(async move {
                        match watch.session_id.as_str() {
                            "flood" => {
                                for index in 0..FLOOD {
                                    let _ = sender.send(Ok(event(&index.to_string()))).await;
                                }
                                if let Some(finished) = finished {
                                    let _ = finished.send(());
                                }
                            }
                            "three" => {
                                for id in ["a", "b", "c"] {
                                    let _ = sender.send(Ok(event(id))).await;
                                }
                            }
                            "broken" => {
                                let _ = sender.send(Ok(event("a"))).await;
                                let _ = sender
                                    .send(Err(pb::Error {
                                        code: pb::ErrorCode::StorageFailed.into(),
                                        message: String::new(),
                                    }))
                                    .await;
                            }
                            _ => {
                                sender.closed().await;
                                if let Some(cancelled) = cancelled {
                                    let _ = cancelled.send(());
                                }
                            }
                        }
                    });
                    Reply::Stream {
                        response: pb::response::Kind::Watching(pb::Watching {}),
                        items,
                    }
                }
                _ => Reply::Response(pb::response::Kind::Sessions(pb::Sessions {
                    sessions: vec![],
                })),
            }
        })
    }
}

async fn connected(service: Arc<Echo>) -> Client {
    let (ours, theirs) = duplex(64 * 1024);
    let (reader, writer) = split(theirs);
    tokio::spawn(async move { serve(reader, writer, pb::Role::Host, "test", service).await });
    let (reader, writer) = split(ours);
    match Client::connect(reader, writer, pb::Role::Daemon, "test").await {
        Ok(client) => client,
        Err(error) => panic!("couldn't connect: {error}"),
    }
}

fn send(session: &str) -> pb::request::Kind {
    pb::request::Kind::SendMessage(pb::SendMessage {
        session_id: session.to_owned(),
        ..Default::default()
    })
}

fn watch(session: &str) -> pb::request::Kind {
    pb::request::Kind::WatchSession(pb::WatchSession {
        session_id: session.to_owned(),
        ..Default::default()
    })
}

fn event_id(item: Option<Result<pb::StreamItem, pb::Error>>) -> Option<String> {
    match item {
        Some(Ok(pb::StreamItem {
            kind: Some(pb::stream_item::Kind::SessionEvent(event)),
        })) => Some(event.id),
        _ => None,
    }
}

#[tokio::test]
async fn a_call_gets_its_response() {
    let client = connected(Arc::default()).await;
    let response = client.call(send("known")).await;
    assert!(
        matches!(response, Ok(pb::response::Kind::MessageSent(_))),
        "{response:?}"
    );
    assert_eq!(client.peer().role(), pb::Role::Host);
}

#[tokio::test]
async fn an_error_response_is_refused_with_its_code() {
    let client = connected(Arc::default()).await;
    let response = client.call(send("unknown")).await;
    assert!(
        matches!(
            response,
            Err(ClientError::Refused {
                code: pb::ErrorCode::UnknownSession,
                ..
            })
        ),
        "{response:?}"
    );
}

#[tokio::test]
async fn answers_find_their_calls_whatever_order_they_come_in() {
    let client = Arc::new(connected(Arc::default()).await);
    let slow = tokio::spawn({
        let client = client.clone();
        async move { client.call(send("slow")).await }
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    let fast = client
        .call(pb::request::Kind::ListSessions(pb::ListSessions::default()))
        .await;
    assert!(
        matches!(fast, Ok(pb::response::Kind::Sessions(_))),
        "{fast:?}"
    );
    let slow = slow.await.map_err(|error| error.to_string());
    assert!(
        matches!(slow, Ok(Ok(pb::response::Kind::MessageSent(_)))),
        "{slow:?}"
    );
}

#[tokio::test]
async fn a_stream_delivers_its_items_then_ends() {
    let client = connected(Arc::default()).await;
    let Ok((response, mut stream)) = client.stream(watch("three")).await else {
        panic!("the watch was refused")
    };
    assert!(matches!(response, pb::response::Kind::Watching(_)));
    assert_eq!(event_id(stream.next().await).as_deref(), Some("a"));
    assert_eq!(event_id(stream.next().await).as_deref(), Some("b"));
    assert_eq!(event_id(stream.next().await).as_deref(), Some("c"));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_stream_that_fails_ends_with_its_error() {
    let client = connected(Arc::default()).await;
    let Ok((_, mut stream)) = client.stream(watch("broken")).await else {
        panic!("the watch was refused")
    };
    assert_eq!(event_id(stream.next().await).as_deref(), Some("a"));
    let end = stream.next().await;
    assert!(
        matches!(&end, Some(Err(error)) if error.code() == pb::ErrorCode::StorageFailed),
        "{end:?}"
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn a_stream_that_ends_normally_did_not_fall_behind() {
    let client = connected(Arc::default()).await;
    let Ok((_, mut stream)) = client.stream(watch("three")).await else {
        panic!("the watch was refused")
    };
    while stream.next().await.is_some() {}
    assert!(!stream.fell_behind());
}

#[tokio::test]
async fn a_stream_whose_reader_falls_too_far_behind_says_so() {
    let (finished, sent) = oneshot::channel();
    let service = Arc::new(Echo {
        finished: Mutex::new(Some(finished)),
        ..Echo::default()
    });
    let client = connected(service).await;
    let Ok((_, mut stream)) = client.stream(watch("flood")).await else {
        panic!("the watch was refused")
    };
    let sent = tokio::time::timeout(Duration::from_secs(5), sent).await;
    assert!(matches!(sent, Ok(Ok(()))), "the flood wasn't sent");
    // A round trip after the flood, so its items have been read.
    assert!(client.call(send("known")).await.is_ok());
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut read = 0;
    while stream.next().await.is_some() {
        read += 1;
    }
    assert!(read < FLOOD);
    assert!(stream.fell_behind());
    // The connection still answers.
    assert!(client.call(send("known")).await.is_ok());
}

#[tokio::test]
async fn dropping_a_stream_stops_it_on_the_other_side() {
    let (cancelled, stopped) = oneshot::channel();
    let service = Arc::new(Echo {
        cancelled: Mutex::new(Some(cancelled)),
        ..Echo::default()
    });
    let client = connected(service).await;
    let Ok((_, stream)) = client.stream(watch("forever")).await else {
        panic!("the watch was refused")
    };
    drop(stream);
    let stopped = tokio::time::timeout(Duration::from_secs(5), stopped).await;
    assert!(
        matches!(stopped, Ok(Ok(()))),
        "the service's stream kept going"
    );
    // The connection still answers.
    assert!(client.call(send("known")).await.is_ok());
}

#[tokio::test]
async fn a_call_fails_when_the_other_side_goes_away() {
    let (ours, theirs) = duplex(64 * 1024);
    let (mut their_reader, mut their_writer) = split(theirs);
    let peer = tokio::spawn(async move {
        let hello = pb::Hello {
            protocol_version: pb::ProtocolVersion::Current.into(),
            role: pb::Role::Host.into(),
            program_version: "test".to_owned(),
            accepts_zstd: false,
            ..Default::default()
        };
        let envelope = pb::Envelope {
            id: 0,
            body: Some(pb::envelope::Body::Hello(hello)),
        };
        let _ = crate::write(&mut their_writer, &envelope, false).await;
        let _ = crate::read(&mut their_reader).await;
        // Reads the request, then hangs up without answering.
        let _ = crate::read(&mut their_reader).await;
    });
    let (reader, writer) = split(ours);
    let Ok(client) = Client::connect(reader, writer, pb::Role::Daemon, "test").await else {
        panic!("couldn't connect")
    };
    let response = client.call(send("known")).await;
    assert!(
        matches!(response, Err(ClientError::Disconnected)),
        "{response:?}"
    );
    assert!(peer.await.is_ok());
}

#[tokio::test]
async fn a_call_made_after_the_other_side_went_away_fails_rather_than_waits() {
    let (ours, theirs) = duplex(64 * 1024);
    let (mut their_reader, mut their_writer) = split(theirs);
    let peer = tokio::spawn(async move {
        let hello = pb::Hello {
            protocol_version: pb::ProtocolVersion::Current.into(),
            role: pb::Role::Host.into(),
            program_version: "test".to_owned(),
            accepts_zstd: false,
            ..Default::default()
        };
        let envelope = pb::Envelope {
            id: 0,
            body: Some(pb::envelope::Body::Hello(hello)),
        };
        let _ = crate::write(&mut their_writer, &envelope, false).await;
        // Reads our hello, then hangs up.
        let _ = crate::read(&mut their_reader).await;
    });
    let (reader, writer) = split(ours);
    let Ok(client) = Client::connect(reader, writer, pb::Role::Daemon, "test").await else {
        panic!("couldn't connect")
    };
    assert!(peer.await.is_ok());
    let noticed = tokio::time::timeout(Duration::from_secs(5), async {
        while client.is_connected() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(noticed.is_ok(), "the client never saw the connection end");

    let call = tokio::time::timeout(Duration::from_secs(5), client.call(send("known"))).await;
    assert!(
        matches!(call, Ok(Err(ClientError::Disconnected))),
        "{call:?}"
    );
    let stream = tokio::time::timeout(Duration::from_secs(5), client.stream(watch("three")))
        .await
        .map(|answer| answer.map(|(response, _)| response));
    assert!(
        matches!(stream, Ok(Err(ClientError::Disconnected))),
        "{stream:?}"
    );
}

#[tokio::test]
async fn a_request_is_answered_to_the_end_when_its_caller_goes_away() {
    let (open, gate) = oneshot::channel();
    let (finished, done) = oneshot::channel();
    let service = Arc::new(Echo {
        gate: Mutex::new(Some(gate)),
        finished: Mutex::new(Some(finished)),
        ..Echo::default()
    });
    let client = connected(service).await;
    let call = tokio::spawn(async move { client.call(send("gated")).await });
    // Let the request reach the service, then hang up.
    tokio::time::sleep(Duration::from_millis(50)).await;
    call.abort();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let _ = open.send(());
    let finished = tokio::time::timeout(Duration::from_secs(5), done).await;
    assert!(
        matches!(finished, Ok(Ok(()))),
        "the service was stopped half way"
    );
}

async fn connect_to(
    peer: impl FnOnce(DuplexStream) -> tokio::task::JoinHandle<()>,
) -> Result<Client, WireError> {
    let (ours, theirs) = duplex(64 * 1024);
    let _peer = peer(theirs);
    let (reader, writer) = split(ours);
    Client::connect(reader, writer, pb::Role::Daemon, "test").await
}

#[tokio::test]
async fn a_peer_on_another_protocol_version_is_refused() {
    let result = connect_to(|mut theirs| {
        tokio::spawn(async move {
            let hello = pb::Hello {
                protocol_version: 99,
                role: pb::Role::Host.into(),
                program_version: "9.0".to_owned(),
                accepts_zstd: false,
                ..Default::default()
            };
            let envelope = pb::Envelope {
                id: 0,
                body: Some(pb::envelope::Body::Hello(hello)),
            };
            let _ = crate::write(&mut theirs, &envelope, false).await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        })
    })
    .await;
    assert!(
        matches!(result, Err(WireError::VersionMismatch { theirs: 99, .. })),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
async fn a_peer_that_does_not_say_hello_is_refused() {
    let result = connect_to(|mut theirs| {
        tokio::spawn(async move {
            let envelope = pb::Envelope {
                id: 1,
                body: Some(pb::envelope::Body::Cancel(pb::Cancel {})),
            };
            let _ = crate::write(&mut theirs, &envelope, false).await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        })
    })
    .await;
    assert!(
        matches!(result, Err(WireError::NoHello)),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
async fn a_peer_that_prints_text_first_is_refused() {
    // What a chatty SSH login script does.
    let result = connect_to(|mut theirs| {
        tokio::spawn(async move {
            let _ = theirs.write_all(b"Welcome to the build machine!\n").await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        })
    })
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn a_peer_that_closes_before_hello_is_refused() {
    let result = connect_to(|theirs| tokio::spawn(async move { drop(theirs) })).await;
    assert!(
        matches!(result, Err(WireError::ClosedBeforeHello)),
        "{:?}",
        result.err()
    );
}

#[tokio::test]
async fn a_frame_over_the_limit_is_refused_before_it_is_read() {
    let (mut ours, mut theirs) = duplex(64);
    let length = u32::try_from(crate::MAX_FRAME + 1).unwrap_or(u32::MAX);
    let _ = theirs.write_all(&length.to_be_bytes()).await;
    let result = crate::read(&mut ours).await;
    assert!(
        matches!(result, Err(WireError::TooLarge { .. })),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_frame_cut_off_in_the_middle_is_truncated() {
    let (mut ours, mut theirs) = duplex(64);
    let _ = theirs.write_all(&10_u32.to_be_bytes()).await;
    let _ = theirs.write_all(b"abc").await;
    drop(theirs);
    let result = crate::read(&mut ours).await;
    assert!(
        matches!(result, Err(WireError::Truncated { length: 10, .. })),
        "{result:?}"
    );
}

fn long_history() -> pb::Envelope {
    let text = "the agent ran the tests and they passed. ".repeat(500);
    pb::Envelope {
        id: 7,
        body: Some(pb::envelope::Body::Request(pb::Request {
            kind: Some(pb::request::Kind::SendMessage(pb::SendMessage {
                session_id: "known".to_owned(),
                text,
                ..Default::default()
            })),
        })),
    }
}

#[tokio::test]
async fn a_long_envelope_for_a_peer_that_reads_zstd_is_smaller_and_reads_back_the_same() {
    let envelope = long_history();
    let mut plain = Vec::new();
    crate::write(&mut plain, &envelope, false)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let mut compressed = Vec::new();
    crate::write(&mut compressed, &envelope, true)
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(compressed.len() * 4 < plain.len());
    let read = crate::read(&mut compressed.as_slice())
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(read, Some(envelope));
}

/// A message of `text`, as long as a backup or a long history is.
fn long_message(text: String) -> pb::Envelope {
    pb::Envelope {
        id: 9,
        body: Some(pb::envelope::Body::Request(pb::Request {
            kind: Some(pb::request::Kind::SendMessage(pb::SendMessage {
                session_id: "known".to_owned(),
                text,
                ..Default::default()
            })),
        })),
    }
}

#[tokio::test]
async fn a_very_long_envelope_reads_back_the_same_compressed_or_not() {
    // One that shrinks to almost nothing, and one that hardly shrinks, so
    // what's sent is long too.
    let repeated = "the agent ran the tests and they passed. ".repeat(8_000);
    let mut state = 0x2545_f491_4f6c_dd1d_u64;
    let varied: String = (0..300_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            char::from(b'!' + (state % 90) as u8)
        })
        .collect();
    for text in [repeated, varied] {
        let envelope = long_message(text);
        for compress in [false, true] {
            let mut written = Vec::new();
            crate::write(&mut written, &envelope, compress)
                .await
                .unwrap_or_else(|error| panic!("{error}"));
            let read = crate::read(&mut written.as_slice())
                .await
                .unwrap_or_else(|error| panic!("{error}"));
            assert_eq!(read.as_ref(), Some(&envelope), "compressed: {compress}");
        }
    }
}

#[tokio::test]
async fn a_corrupt_compressed_envelope_is_refused() {
    let mut compressed = Vec::new();
    crate::write(&mut compressed, &long_history(), true)
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let middle = compressed.len() / 2;
    compressed.truncate(middle);
    let body = (middle - 4) as u32 | (1 << 31);
    compressed[..4].copy_from_slice(&body.to_be_bytes());

    let read = crate::read(&mut compressed.as_slice()).await;
    assert!(
        matches!(
            read,
            Err(WireError::Decompress { .. } | WireError::Decode { .. })
        ),
        "{read:?}"
    );
}

#[tokio::test]
async fn peers_that_read_zstd_send_long_messages_compressed_and_still_answer() {
    let (client, server) = duplex(1 << 20);
    let (server_read, server_write) = split(server);
    tokio::spawn(serve(
        server_read,
        server_write,
        pb::Role::Host,
        "test",
        Arc::new(Echo::default()),
    ));
    let (client_read, client_write) = split(client);
    let client = Client::connect(client_read, client_write, pb::Role::Daemon, "test")
        .await
        .unwrap_or_else(|error| panic!("{error}"));

    assert!(client.peer().accepts_zstd);
    let Some(pb::envelope::Body::Request(pb::Request {
        kind: Some(request),
    })) = long_history().body
    else {
        panic!("not a request");
    };
    let answer = client.call(request).await;
    assert!(
        matches!(answer, Ok(pb::response::Kind::MessageSent(_))),
        "{answer:?}"
    );
}
