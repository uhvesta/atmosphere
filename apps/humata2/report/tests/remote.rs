use humata2_report::remote::{ask, Listener, Refusal, RefusalKind, RemoteError, Reply, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

// Unix socket paths are limited to about 104 bytes on macOS, so the
// folder is made directly under /tmp with a short name.
fn short_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("hr")
        .tempdir_in("/tmp")
        .unwrap()
}

/// Serves exactly one request with `answer`, giving back the listener and
/// what it was asked.
fn serve_one(
    mut listener: Listener,
    answer: Result<Reply, Refusal>,
) -> tokio::task::JoinHandle<(Listener, Request)> {
    tokio::spawn(async move {
        let (request, responder) = listener.next().await.unwrap();
        responder.reply(answer).await.unwrap();
        (listener, request)
    })
}

fn reply(folder: Option<&str>, recording: bool) -> Reply {
    Reply {
        folder: folder.map(str::to_string),
        recording,
    }
}

#[tokio::test]
async fn each_request_kind_round_trips() {
    let dir = short_dir();
    let path = dir.path().join("a.sock");
    let mut listener = Listener::bind(&path).unwrap();
    let requests = [
        Request::Record {
            title: Some("it broke".into()),
        },
        Request::Record { title: None },
        Request::Stop,
        Request::Snapshot {
            title: Some("here".into()),
        },
        Request::Snapshot { title: None },
    ];
    for request in requests {
        let answer = Ok(reply(Some("/x/y"), true));
        let server = serve_one(listener, answer.clone());
        let got = ask(&path, &request).await.unwrap();
        let (back, seen) = server.await.unwrap();
        listener = back;
        assert_eq!(seen, request);
        assert_eq!(got, answer);
    }
}

#[tokio::test]
async fn a_refusal_reaches_the_asker() {
    let dir = short_dir();
    let path = dir.path().join("a.sock");
    let listener = Listener::bind(&path).unwrap();
    let refusal = Refusal {
        kind: RefusalKind::AlreadyRecording,
        message: "busy".into(),
    };
    let server = serve_one(listener, Err(refusal.clone()));
    let got = ask(&path, &Request::Stop).await.unwrap();
    server.await.unwrap();
    assert_eq!(got, Err(refusal));
}

#[tokio::test]
async fn asking_with_no_app_says_not_running() {
    let dir = short_dir();
    let path = dir.path().join("none.sock");
    let err = ask(&path, &Request::Stop).await.unwrap_err();
    assert!(matches!(err, RemoteError::NotRunning { .. }));
}

#[tokio::test]
async fn a_second_bind_on_a_live_socket_is_in_use_and_leaves_the_first_working() {
    let dir = short_dir();
    let path = dir.path().join("a.sock");
    let listener = Listener::bind(&path).unwrap();
    let second = Listener::bind(&path);
    assert!(matches!(second, Err(RemoteError::InUse { .. })));
    let server = serve_one(listener, Ok(reply(None, false)));
    let got = ask(&path, &Request::Stop).await.unwrap();
    server.await.unwrap();
    assert_eq!(got, Ok(reply(None, false)));
}

#[tokio::test]
async fn a_leftover_socket_file_is_replaced() {
    let dir = short_dir();
    let path = dir.path().join("a.sock");
    // A crashed app leaves its socket file behind.
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    let listener = Listener::bind(&path).unwrap();
    let server = serve_one(listener, Ok(reply(None, true)));
    let got = ask(&path, &Request::Record { title: None }).await.unwrap();
    server.await.unwrap();
    assert_eq!(got, Ok(reply(None, true)));
}

#[tokio::test]
async fn dropping_the_listener_removes_the_file() {
    let dir = short_dir();
    let path = dir.path().join("a.sock");
    let listener = Listener::bind(&path).unwrap();
    assert!(path.exists());
    drop(listener);
    assert!(!path.exists());
}

#[tokio::test]
async fn the_socket_is_readable_only_by_its_user() {
    use std::os::unix::fs::PermissionsExt;
    let dir = short_dir();
    let path = dir.path().join("sub").join("a.sock");
    let listener = Listener::bind(&path).unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(listener.path()), 0o600);
    assert_eq!(mode(path.parent().unwrap()), 0o700);
}

#[tokio::test]
async fn a_bad_line_is_refused_and_the_next_request_is_still_served() {
    let dir = short_dir();
    let path = dir.path().join("a.sock");
    let listener = Listener::bind(&path).unwrap();
    let server = serve_one(listener, Ok(reply(Some("/r"), false)));

    let stream = UnixStream::connect(&path).await.unwrap();
    let mut reader = BufReader::new(stream);
    reader.write_all(b"this is not json\n").await.unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).await.unwrap();
    let value: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["refused"]["kind"], "failed");

    let got = ask(&path, &Request::Snapshot { title: None }).await.unwrap();
    let (_listener, seen) = server.await.unwrap();
    assert_eq!(seen, Request::Snapshot { title: None });
    assert_eq!(got, Ok(reply(Some("/r"), false)));
}
