//! The HTTP side of the link against a loopback fake that answers as
//! OpenCode v2.3.0 did (testdata/): every call carries the password, lists
//! and the event stream read back as the link needs them, and a reply the
//! TUI beat is told apart from a server that refuses.

use std::path::Path;

use humata2_opencode::{Event, Http, Server, ServerError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// One request as the fake read it.
#[derive(Debug)]
struct Seen {
    method: String,
    path: String,
    /// Header lines, lowercased names.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(have, _)| have == name)
            .map(|(_, value)| value.as_str())
    }
}

/// A server that reads one request, answers it with `response` and keeps
/// the connection open until the client goes. Gives its URL and the
/// request it read.
async fn answering(response: Vec<u8>) -> (String, oneshot::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let url = format!("http://{}", listener.local_addr().expect("an address"));
    let (seen_tx, seen) = oneshot::channel();
    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut read = Vec::new();
        let mut buffer = [0u8; 4096];
        let head_end = loop {
            if let Some(at) = read.windows(4).position(|w| w == b"\r\n\r\n") {
                break at;
            }
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(n) => read.extend_from_slice(&buffer[..n]),
            }
        };
        let head = String::from_utf8_lossy(&read[..head_end]).into_owned();
        let mut lines = head.split("\r\n");
        let request_line = lines.next().unwrap_or_default().to_owned();
        let mut parts = request_line.split(' ');
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let length = headers
            .iter()
            .find(|(name, _)| name == "content-length")
            .and_then(|(_, value)| value.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = read[head_end + 4..].to_vec();
        while body.len() < length {
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(n) => body.extend_from_slice(&buffer[..n]),
            }
        }
        let _ = seen_tx.send(Seen {
            method,
            path,
            headers,
            body,
        });
        let _ = socket.write_all(&response).await;
        // Held open, as an event stream is, until the client drops it.
        let _ = socket.read(&mut buffer).await;
    });
    (url, seen)
}

fn json_response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// Fails the test instead of hanging it when `future` never finishes.
async fn within<F: std::future::Future>(future: F) -> F::Output {
    match tokio::time::timeout(std::time::Duration::from_secs(10), future).await {
        Ok(output) => output,
        Err(_) => panic!("no answer from the fake within 10 s"),
    }
}

/// `opencode:pw` in base64, as Basic auth sends it.
const AUTH: &str = "Basic b3BlbmNvZGU6cHc=";

#[tokio::test]
async fn the_event_stream_gives_parsed_events_and_carries_the_password() {
    let sse = include_str!("../testdata/events-session-created.sse");
    let mut response =
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n"
            .to_vec();
    // The frames cut mid-frame across two chunks, then a heartbeat between
    // frames, then the end of the body, so the stream ends instead of
    // waiting for more.
    let (first, second) = sse.as_bytes().split_at(sse.len() / 3);
    for chunk in [first, second, b": heartbeat\n\n".as_slice()] {
        response.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        response.extend_from_slice(chunk);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");
    let (url, seen) = answering(response).await;
    let http = Http::new(&url, "pw").expect("a loopback URL");
    let mut events = within(http.events()).await.expect("the stream opens");
    assert_eq!(within(events.recv()).await, Some(Event::Connected));
    assert!(
        within(events.recv()).await.is_some(),
        "the recorded session.created"
    );
    assert_eq!(
        within(events.recv()).await,
        None,
        "the heartbeat is no event"
    );
    let seen = within(seen).await.expect("the fake read the request");
    assert_eq!(seen.method, "GET");
    assert_eq!(seen.path, "/api/event");
    assert_eq!(seen.header("authorization"), Some(AUTH));
}

#[tokio::test]
async fn the_running_sessions_and_open_asks_are_listed() {
    let running = include_str!("../testdata/session-active-running.json");
    let (url, seen) = answering(json_response("200 OK", running)).await;
    let http = Http::new(&url, "pw").expect("a loopback URL");
    assert_eq!(
        within(http.active()).await.expect("listed"),
        vec!["ses_e1c52e7e9ffesHyRT9YEoxd0RG".to_owned()]
    );
    assert_eq!(
        within(seen).await.expect("read").path,
        "/api/session/active"
    );

    let asks = include_str!("../testdata/permission-list.json");
    let (url, seen) = answering(json_response("200 OK", asks)).await;
    let http = Http::new(&url, "pw").expect("a loopback URL");
    let pending = within(http.pending()).await.expect("listed");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, "per_1e3ac9e95001Sgm3zNE4P6lF7q");
    assert_eq!(pending[0].session, "ses_e1c537029ffejTdBfVrnGEG0jm");
    assert_eq!(
        within(seen).await.expect("read").path,
        "/api/permission/request"
    );
}

#[tokio::test]
async fn a_reply_posts_its_body_to_the_asks_session() {
    let (url, seen) = answering(b"HTTP/1.1 204 No Content\r\n\r\n".to_vec()).await;
    let http = Http::new(&url, "pw").expect("a loopback URL");
    let body = br#"{"decision":"once"}"#.to_vec();
    within(http.reply("ses_1", "per_2", body.clone()))
        .await
        .expect("replied");
    let seen = within(seen).await.expect("read");
    assert_eq!(seen.method, "POST");
    assert_eq!(seen.path, "/api/session/ses_1/permission/per_2/reply");
    assert_eq!(seen.body, body);
    assert_eq!(seen.header("authorization"), Some(AUTH));
    assert!(
        seen.header("content-type")
            .is_some_and(|kind| kind.contains("json"))
    );
}

#[tokio::test]
async fn a_reply_the_tui_beat_is_gone_and_a_refusal_is_not() {
    let errors: serde_json::Value =
        serde_json::from_str(include_str!("../testdata/errors.json")).expect("JSON");
    let stale = errors["stale_permission_reply"]["body"].to_string();
    let (url, _seen) = answering(json_response("404 Not Found", &stale)).await;
    let http = Http::new(&url, "pw").expect("a loopback URL");
    let replied = within(http.reply("ses_1", "per_2", b"{}".to_vec())).await;
    assert!(matches!(replied, Err(ServerError::Gone)), "{replied:?}");

    let (url, _seen) = answering(json_response("401 Unauthorized", "{}")).await;
    let http = Http::new(&url, "wrong").expect("a loopback URL");
    let replied = within(http.reply("ses_1", "per_2", b"{}".to_vec())).await;
    assert!(
        matches!(replied, Err(ServerError::Status { code: 401, .. })),
        "{replied:?}"
    );
}

#[tokio::test]
async fn creating_the_session_names_its_id_and_folder() {
    let created = r#"{"data":{"id":"ses_0f8a1c2e7b3d4e5f9a6b1c2d3e"}}"#;
    let (url, seen) = answering(json_response("200 OK", created)).await;
    let http = Http::new(&url, "pw").expect("a loopback URL");
    within(http.create_session("ses_0f8a1c2e7b3d4e5f9a6b1c2d3e", Path::new("/work/proj")))
        .await
        .expect("created");
    let seen = within(seen).await.expect("read");
    assert_eq!(
        (seen.method.as_str(), seen.path.as_str()),
        ("POST", "/api/session")
    );
    let body: serde_json::Value = serde_json::from_slice(&seen.body).expect("JSON");
    assert_eq!(body["id"], "ses_0f8a1c2e7b3d4e5f9a6b1c2d3e");
    assert_eq!(body["location"]["directory"], "/work/proj");
}

#[tokio::test]
async fn nothing_listening_and_a_non_loopback_url_are_errors() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let url = format!("http://{}", listener.local_addr().expect("an address"));
    drop(listener);
    let http = Http::new(&url, "pw").expect("a loopback URL");
    assert!(within(http.active()).await.is_err());
    assert!(Http::new("http://10.0.0.5:4096", "pw").is_err());
    assert!(Http::new("not a url", "pw").is_err());
}
