use crate::{
    Error, Host,
    auth::{Token, allowed_origin},
};
use axum::{
    Router,
    body::Body,
    extract::{
        State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, Uri, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use std::{
    net::SocketAddr,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use terminal_wire::{ClientFrame, PROTOCOL_VERSION, ServerFrame};
const SEND_TIMEOUT: Duration = Duration::from_secs(15);
#[derive(Clone)]
struct Gateway {
    host: Host,
    token: Token,
    static_dir: Arc<PathBuf>,
    connections: Arc<tokio::sync::Semaphore>,
}
/// Serve only on loopback. SSH forwarding supplies cross-machine access without
/// giving the browser native access to the terminal host or its credentials.
pub async fn serve(
    host: Host,
    address: SocketAddr,
    static_dir: PathBuf,
    token_file: PathBuf,
) -> Result<(), Error> {
    if !address.ip().is_loopback() {
        return Err(Error::Invalid {
            reason: "the terminal gateway must bind a loopback address; use authenticated SSH forwarding for remote viewers",
        });
    }
    let static_dir = std::fs::canonicalize(&static_dir)
        .map_err(|e| crate::error::disk("locate browser assets", &static_dir, e))?;
    if !static_dir.is_dir() {
        return Err(Error::Invalid {
            reason: "--static-dir must name the built browser asset directory",
        });
    }
    let token = Token::load_or_create(&token_file)?;
    let shutdown_host = host.clone();
    let app = router(host, static_dir, token);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|source| Error::Listen { address, source })?;
    let address = listener
        .local_addr()
        .map_err(|source| Error::Listen { address, source })?;
    eprintln!(
        "Terminal host listening on http://{address}; authenticate using its private token file."
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            if let Err(error)=shutdown_host.shutdown().await{eprintln!("Terminal host shutdown left incomplete recovery: {error} Existing holders remain running; preserve the private state folder before retrying.");}
        })
        .await
        .map_err(|source| Error::Serve { source })
}
async fn shutdown_signal() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {_ = tokio::signal::ctrl_c()=>{},_ = terminate.recv()=>{}};
        return;
    }
    let _ = tokio::signal::ctrl_c().await;
}
pub(crate) fn router(host: Host, static_dir: PathBuf, token: Token) -> Router {
    let state = Gateway {
        host,
        token,
        static_dir: Arc::new(static_dir),
        connections: Arc::new(tokio::sync::Semaphore::new(16)),
    };
    Router::new()
        .route("/ws", get(upgrade))
        .route("/health", get(|| async { "ok" }))
        .fallback(static_asset)
        .with_state(state)
}
async fn upgrade(
    State(state): State<Gateway>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok());
    if !allowed_origin(host, origin) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(slot) = state.connections.clone().try_acquire_owned() else {
        return (StatusCode::TOO_MANY_REQUESTS,"This terminal host already has 16 open WebSocket connections. Close an unused viewer or wait for pending connections to close, then retry.").into_response();
    };
    ws.max_message_size(2 << 20)
        .max_frame_size(2 << 20)
        .on_upgrade(move |socket| async move {
            let _slot = slot;
            connection(state, socket).await;
        })
        .into_response()
}
async fn send(socket: &mut WebSocket, frame: &ServerFrame) -> bool {
    let Ok(text) = serde_json::to_string(frame) else {
        return false;
    };
    matches!(
        tokio::time::timeout(SEND_TIMEOUT, socket.send(Message::Text(text.into()))).await,
        Ok(Ok(()))
    )
}
async fn reject(socket: &mut WebSocket, code: &str, message: &str) {
    let _ = send(
        socket,
        &ServerFrame::Error {
            id: None,
            code: code.into(),
            message: message.into(),
        },
    )
    .await;
    let _ = tokio::time::timeout(Duration::from_secs(1), socket.send(Message::Close(None))).await;
}
fn parse_client_frame(text: &str) -> Result<ClientFrame, serde_json::Error> {
    let frame = serde_json::from_str(text)?;
    if matches!(frame, ClientFrame::List) {
        let value: serde_json::Map<String, serde_json::Value> = serde_json::from_str(text)?;
        if value.len() != 1 {
            return Err(<serde_json::Error as serde::de::Error>::custom(
                "list does not accept extra fields",
            ));
        }
    }
    Ok(frame)
}
fn request_bounds_error(frame: &ClientFrame) -> Option<ServerFrame> {
    let (id, code, message) = match frame {
        ClientFrame::Input { id, bytes, .. } if bytes.len() > crate::MAX_INPUT => (
            Some(*id),
            "input_too_large",
            "Paste at most 256 KiB per input frame; this payload was rejected before queueing.",
        ),
        ClientFrame::SetRecovery { id, hint }
            if crate::validate_recovery(hint.as_ref()).is_err() =>
        {
            (
                Some(*id),
                "invalid_recovery",
                "Recovery metadata needs bounded explicit identifiers and at most sixteen paths without control characters.",
            )
        }
        ClientFrame::SetHistory { id, policy } if crate::validate_history(policy).is_err() => (
            Some(*id),
            "invalid_history",
            "Choose 1–100000 history lines, 64 KiB–4 MiB of compressed storage, and a 1–3600 second interval.",
        ),
        ClientFrame::Create { profile } if crate::validate_name(profile).is_err() => (
            None,
            "unknown_profile",
            "Choose a trusted configured profile with a bounded name.",
        ),
        ClientFrame::Authenticate { .. } => (
            None,
            "already_authenticated",
            "This connection is already authenticated.",
        ),
        _ => return None,
    };
    Some(ServerFrame::Error {
        id,
        code: code.into(),
        message: message.into(),
    })
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthRejection {
    Deadline,
    Eof,
    WebSocketError,
    NonText,
    Oversize,
    InvalidJson,
    WrongFrameType,
    ProtocolMismatch,
    TokenMismatch,
}
impl AuthRejection {
    fn category(self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::Eof => "eof",
            Self::WebSocketError => "websocket_error",
            Self::NonText => "non_text",
            Self::Oversize => "oversize",
            Self::InvalidJson => "invalid_json",
            Self::WrongFrameType => "wrong_frame_type",
            Self::ProtocolMismatch => "protocol_mismatch",
            Self::TokenMismatch => "token_mismatch",
        }
    }
}
fn authenticate_first(
    first: Result<Option<Result<Message, axum::Error>>, tokio::time::error::Elapsed>,
    token: &Token,
) -> Result<(), AuthRejection> {
    let message = first
        .map_err(|_| AuthRejection::Deadline)?
        .ok_or(AuthRejection::Eof)?
        .map_err(|_| AuthRejection::WebSocketError)?;
    let Message::Text(text) = message else {
        return Err(AuthRejection::NonText);
    };
    if text.len() > 1024 {
        return Err(AuthRejection::Oversize);
    }
    let frame = parse_client_frame(&text).map_err(|_| AuthRejection::InvalidJson)?;
    let ClientFrame::Authenticate {
        protocol,
        token: candidate,
    } = frame
    else {
        return Err(AuthRejection::WrongFrameType);
    };
    if protocol != PROTOCOL_VERSION {
        return Err(AuthRejection::ProtocolMismatch);
    }
    if !token.matches(&candidate) {
        return Err(AuthRejection::TokenMismatch);
    }
    Ok(())
}
async fn connection(state: Gateway, mut socket: WebSocket) {
    let first = tokio::time::timeout(Duration::from_secs(5), socket.recv()).await;
    if let Err(reason) = authenticate_first(first, &state.token) {
        eprintln!(
            "terminal viewer rejected: authentication reason={}",
            reason.category()
        );
        reject(&mut socket,"authentication","Authenticate with this host's current private token and matching protocol version before opening terminals.").await;
        return;
    }
    let mut subscription = match state.host.subscribe().await {
        Ok(s) => s,
        Err(_) => {
            reject(
                &mut socket,
                "viewer_unavailable",
                "This host cannot accept another viewer. Close an unused viewer or reconnect after the host is available.",
            )
            .await;
            return;
        }
    };
    let viewer = subscription.id;
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
         frame=subscription.receiver.recv()=>{let Some(frame)=frame else{eprintln!("terminal viewer {viewer} disconnected: host outbound subscription closed");break;};let text=match serde_json::to_string(&frame.frame){Ok(text)=>text,Err(_)=>{let id=match &frame.frame{ServerFrame::Snapshot{id,..}=>Some(*id),_=>None};let error=ServerFrame::Error{id,code:"frame_encoding_limit".into(),message:"This terminal state exceeds the supported snapshot transport budget (16 MiB decoded, 4 MiB compressed), or could not be encoded. Reduce retained history and request a fresh snapshot; the original process is still running.".into()};let Ok(text)=serde_json::to_string(&error)else{break;};text}};if !matches!(tokio::time::timeout(SEND_TIMEOUT,sender.send(Message::Text(text.into()))).await,Ok(Ok(()))){eprintln!("terminal viewer {viewer} disconnected: WebSocket write failed or timed out");break;}},
         message=receiver.next()=>{match message{
          Some(Ok(Message::Text(text)))=>{let frame=match parse_client_frame(&text){Ok(frame)=>frame,Err(error)=>{eprintln!("terminal viewer {viewer} disconnected: invalid protocol frame={} JSON category={:?}",protocol_frame_kind(&text),error.classify());let frame=ServerFrame::Error{id:None,code:"invalid_frame".into(),message:"Send a valid terminal protocol message; malformed input was not forwarded.".into()};if let Ok(text)=serde_json::to_string(&frame){let _=tokio::time::timeout(SEND_TIMEOUT,sender.send(Message::Text(text.into()))).await;}break;}};if let Some(error)=request_bounds_error(&frame){drop(frame);let Ok(text)=serde_json::to_string(&error)else{break;};if !matches!(tokio::time::timeout(SEND_TIMEOUT,sender.send(Message::Text(text.into()))).await,Ok(Ok(()))){break;}continue;}if state.host.frame(viewer,frame).await.is_err(){break;}},
          Some(Ok(Message::Ping(bytes)))=>{if !matches!(tokio::time::timeout(SEND_TIMEOUT,sender.send(Message::Pong(bytes))).await,Ok(Ok(()))){break;}},
          Some(Ok(Message::Pong(_)))=>{},
          Some(Ok(Message::Close(frame)))=>{eprintln!("terminal viewer {viewer} disconnected: peer Close code={:?}",frame.map(|frame|frame.code));break;},
          None=>{eprintln!("terminal viewer {viewer} disconnected: peer transport EOF without Close");break;},
          Some(Err(error))=>{eprintln!("terminal viewer {viewer} disconnected: {}",read_failure_kind(&error));break;},
          Some(Ok(Message::Binary(_)))=>{eprintln!("terminal viewer {viewer} disconnected: unsupported binary protocol frame");break;},
         }}
        }
    }
    state.host.leave(viewer).await;
    let _ = tokio::time::timeout(Duration::from_secs(1), sender.send(Message::Close(None))).await;
}
fn protocol_frame_kind(text: &str) -> &'static str {
    #[derive(serde::Deserialize)]
    struct Kind<'a> {
        #[serde(borrow, rename = "type")]
        kind: &'a str,
    }
    match serde_json::from_str::<Kind<'_>>(text)
        .ok()
        .map(|kind| kind.kind)
    {
        Some("authenticate") => "authenticate",
        Some("list") => "list",
        Some("open") => "open",
        Some("detach") => "detach",
        Some("take_control") => "take_control",
        Some("snapshot_request") => "snapshot_request",
        Some("ack") => "ack",
        Some("input") => "input",
        Some("resize") => "resize",
        Some("stop") => "stop",
        Some("create") => "create",
        Some("set_recovery") => "set_recovery",
        Some("set_history") => "set_history",
        _ => "unknown",
    }
}
// Classify typed errors without formatting their payload-bearing descriptions.
fn read_failure_kind(error: &axum::Error) -> &'static str {
    use tokio_tungstenite::tungstenite::Error as WsError;
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = source {
        if let Some(error) = error.downcast_ref::<WsError>() {
            return match error {
                WsError::Capacity(_) => "WebSocket frame/message capacity limit",
                WsError::Protocol(_) => "WebSocket protocol violation",
                WsError::Utf8(_) => "WebSocket invalid text encoding",
                WsError::Io(_) => "WebSocket socket I/O failure",
                WsError::ConnectionClosed | WsError::AlreadyClosed => {
                    "WebSocket transport already closed"
                }
                WsError::WriteBufferFull(_) => "WebSocket write buffer exhausted",
                _ => "WebSocket receive transport error",
            };
        }
        source = error.source();
    }
    "unclassified WebSocket receive failure"
}
async fn static_asset(State(state): State<Gateway>, uri: Uri, headers: HeaderMap) -> Response {
    if !allowed_origin(
        headers.get(header::HOST).and_then(|v| v.to_str().ok()),
        None,
    ) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let name = if uri.path() == "/" {
        "index.html"
    } else {
        uri.path().trim_start_matches('/')
    };
    let relative = Path::new(name);
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = state.static_dir.join(relative);
    let canonical = match tokio::fs::canonicalize(&path).await {
        Ok(p) if p.starts_with(state.static_dir.as_path()) => p,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if tokio::fs::metadata(&canonical)
        .await
        .map_or(true, |m| !m.is_file() || m.len() > 64 * 1024 * 1024)
    {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let bytes = match tokio::fs::read(&canonical).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let content = match canonical.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        _ => "application/octet-stream",
    };
    (
        [
            (header::CONTENT_TYPE, content),
            (header::CACHE_CONTROL, "no-store"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        Body::from(bytes),
    )
        .into_response()
}

#[cfg(test)]
mod authentication_tests {
    use super::*;
    #[tokio::test]
    async fn first_authentication_result_distinguishes_transport_and_credential_failures() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token");
        let token = Token::load_or_create(&path).unwrap();
        let secret = std::fs::read_to_string(path).unwrap();
        let frame = |frame: ClientFrame| {
            Ok(Some(Ok(Message::Text(
                serde_json::to_string(&frame).unwrap().into(),
            ))))
        };
        assert_eq!(
            authenticate_first(
                frame(ClientFrame::Authenticate {
                    protocol: PROTOCOL_VERSION,
                    token: secret.clone()
                }),
                &token
            ),
            Ok(())
        );
        assert_eq!(
            authenticate_first(
                frame(ClientFrame::Authenticate {
                    protocol: PROTOCOL_VERSION + 1,
                    token: secret.clone()
                }),
                &token
            ),
            Err(AuthRejection::ProtocolMismatch)
        );
        assert_eq!(
            authenticate_first(
                frame(ClientFrame::Authenticate {
                    protocol: PROTOCOL_VERSION,
                    token: "not-the-token".into()
                }),
                &token
            ),
            Err(AuthRejection::TokenMismatch)
        );
        assert_eq!(
            authenticate_first(frame(ClientFrame::List), &token),
            Err(AuthRejection::WrongFrameType)
        );
        assert_eq!(
            authenticate_first(Ok(Some(Ok(Message::Text("{".into())))), &token),
            Err(AuthRejection::InvalidJson)
        );
        assert_eq!(
            authenticate_first(Ok(Some(Ok(Message::Text(" ".repeat(1025).into())))), &token),
            Err(AuthRejection::Oversize)
        );
        assert_eq!(
            authenticate_first(Ok(Some(Ok(Message::Binary(Vec::new().into())))), &token),
            Err(AuthRejection::NonText)
        );
        assert_eq!(
            authenticate_first(
                Ok(Some(Err(axum::Error::new(std::io::Error::from(
                    std::io::ErrorKind::ConnectionReset
                ))))),
                &token
            ),
            Err(AuthRejection::WebSocketError)
        );
        assert_eq!(
            authenticate_first(Ok(None), &token),
            Err(AuthRejection::Eof)
        );
        let first = tokio::time::timeout(Duration::ZERO, std::future::pending()).await;
        assert_eq!(
            authenticate_first(first, &token),
            Err(AuthRejection::Deadline)
        );
    }
}
