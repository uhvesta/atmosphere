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
}

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
                pb::request::Kind::SendMessage(message) if message.session_id == "known" => {
                    Reply::Response(pb::response::Kind::MessageSent(pb::MessageSent {}))
                }
                pb::request::Kind::SendMessage(_) => {
                    Reply::Response(error(pb::ErrorCode::UnknownSession, "no such session"))
                }
                pb::request::Kind::WatchSession(watch) => {
                    let (sender, items) = mpsc::channel(8);
                    let cancelled = self.cancelled.lock().await.take();
                    tokio::spawn(async move {
                        match watch.session_id.as_str() {
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
        .call(pb::request::Kind::ListSessions(pb::ListSessions {}))
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
async fn dropping_a_stream_stops_it_on_the_other_side() {
    let (cancelled, stopped) = oneshot::channel();
    let service = Arc::new(Echo {
        cancelled: Mutex::new(Some(cancelled)),
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
        };
        let envelope = pb::Envelope {
            id: 0,
            body: Some(pb::envelope::Body::Hello(hello)),
        };
        let _ = crate::write(&mut their_writer, &envelope).await;
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
            };
            let envelope = pb::Envelope {
                id: 0,
                body: Some(pb::envelope::Body::Hello(hello)),
            };
            let _ = crate::write(&mut theirs, &envelope).await;
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
            let _ = crate::write(&mut theirs, &envelope).await;
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
