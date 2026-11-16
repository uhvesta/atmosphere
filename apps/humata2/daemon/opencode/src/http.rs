//! The real [`Server`]: OpenCode's loopback HTTP API, one connection per call.

use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::event::{Asked, Event};
use crate::link::{Server, ServerError};
use crate::serve::reply_path;
use crate::sse::Frames;

/// The only URL start Humata talks to.
const LOOPBACK: &str = "http://127.0.0.1:";
/// How many parsed events wait for the link.
const BACKLOG: usize = 64;

/// OpenCode's server at a loopback URL, with the password its calls carry.
pub struct Http {
    /// `127.0.0.1:<port>`, for connecting and for the `Host` header.
    addr: String,
    /// The `Authorization` header's value.
    auth: String,
}

impl Http {
    /// A client for the server at `url`, which must be `http://127.0.0.1:<port>`.
    pub fn new(url: &str, password: &str) -> Result<Self, ServerError> {
        let bad = || ServerError::BadUrl {
            url: url.to_owned(),
        };
        let port = url
            .trim_end_matches('/')
            .strip_prefix(LOOPBACK)
            .ok_or_else(bad)?;
        port.parse::<u16>().map_err(|_| bad())?;
        Ok(Self {
            addr: format!("127.0.0.1:{port}"),
            auth: format!("Basic {}", STANDARD.encode(format!("opencode:{password}"))),
        })
    }

    /// Creates the session `id` in `directory`.
    pub async fn create_session(&self, id: &str, directory: &Path) -> Result<(), ServerError> {
        let body = json!({ "id": id, "location": { "directory": directory } });
        let (status, text) = self
            .call(
                Method::POST,
                "/api/session",
                Some(body.to_string().into_bytes()),
            )
            .await?;
        ok_or_status(status, text)
    }

    /// Sends one request on a fresh connection and returns the response head and body.
    async fn send(
        &self,
        method: Method,
        path: &str,
        accept: &str,
        body: Option<Vec<u8>>,
    ) -> Result<
        (
            Response<Incoming>,
            hyper::client::conn::http1::SendRequest<Full<Bytes>>,
        ),
        ServerError,
    > {
        let stream =
            TcpStream::connect(&self.addr)
                .await
                .map_err(|source| ServerError::Connect {
                    addr: self.addr.clone(),
                    source,
                })?;
        let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
            .await
            .map_err(|source| ServerError::Request { source })?;
        tokio::spawn(async move {
            let _ = connection.await;
        });
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header("host", &self.addr)
            .header("authorization", &self.auth)
            .header("accept", accept);
        let payload = match body {
            Some(bytes) => {
                builder = builder.header("content-type", "application/json");
                Bytes::from(bytes)
            }
            None => Bytes::new(),
        };
        let request = builder
            .body(Full::new(payload))
            .map_err(|source| ServerError::Build { source })?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|source| ServerError::Request { source })?;
        Ok((response, sender))
    }

    /// A call whose whole response is read: its status and body text.
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<(u16, String), ServerError> {
        let (response, _sender) = self.send(method, path, "application/json", body).await?;
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|source| ServerError::Body { source })?
            .to_bytes();
        Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
    }

    /// A GET whose 2xx body is JSON with a `data` member, which is returned.
    async fn data(&self, path: &str) -> Result<Value, ServerError> {
        let (status, text) = self.call(Method::GET, path, None).await?;
        if !(200..300).contains(&status) {
            return Err(ServerError::Status {
                code: status,
                body: text,
            });
        }
        let mut value: Value =
            serde_json::from_str(&text).map_err(|source| ServerError::BadJson { source })?;
        Ok(value
            .get_mut("data")
            .map(Value::take)
            .unwrap_or(Value::Null))
    }
}

fn ok_or_status(code: u16, body: String) -> Result<(), ServerError> {
    if (200..300).contains(&code) {
        Ok(())
    } else {
        Err(ServerError::Status { code, body })
    }
}

impl Server for Http {
    async fn events(&self) -> Result<mpsc::Receiver<Event>, ServerError> {
        let (response, sender) = self
            .send(Method::GET, "/api/event", "text/event-stream", None)
            .await?;
        let status = response.status().as_u16();
        let mut body = response.into_body();
        if !(200..300).contains(&status) {
            let text = match body.collect().await {
                Ok(collected) => String::from_utf8_lossy(&collected.to_bytes()).into_owned(),
                Err(_) => String::new(),
            };
            return Err(ServerError::Status {
                code: status,
                body: text,
            });
        }
        let (events, receiver) = mpsc::channel(BACKLOG);
        tokio::spawn(async move {
            // Holding the sender keeps the connection open while the body streams.
            let sender = sender;
            let mut frames = Frames::default();
            while let Some(Ok(frame)) = body.frame().await {
                let Ok(data) = frame.into_data() else {
                    continue;
                };
                for text in frames.push(&data) {
                    if let Ok(event) = Event::parse(&text)
                        && events.send(event).await.is_err()
                    {
                        return;
                    }
                }
            }
        });
        Ok(receiver)
    }

    async fn active(&self) -> Result<Vec<String>, ServerError> {
        match self.data("/api/session/active").await? {
            Value::Object(sessions) => Ok(sessions.keys().cloned().collect()),
            Value::Null => Ok(Vec::new()),
            _ => Err(ServerError::Unexpected {
                what: "object of running sessions",
            }),
        }
    }

    async fn pending(&self) -> Result<Vec<Asked>, ServerError> {
        match self.data("/api/permission/request").await? {
            Value::Array(items) => items.iter().map(asked).collect(),
            Value::Null => Ok(Vec::new()),
            _ => Err(ServerError::Unexpected {
                what: "list of open asks",
            }),
        }
    }

    async fn reply(&self, session: &str, request: &str, body: Vec<u8>) -> Result<(), ServerError> {
        let (status, text) = self
            .call(Method::POST, &reply_path(session, request), Some(body))
            .await?;
        if status == 404 {
            return Err(ServerError::Gone);
        }
        ok_or_status(status, text)
    }
}

/// One ask from the open-asks list.
fn asked(item: &Value) -> Result<Asked, ServerError> {
    let text = |key: &str, what: &'static str| {
        item.get(key)
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(ServerError::Unexpected { what })
    };
    let resources = item
        .get("resources")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    Ok(Asked {
        id: text("id", "ask `id`")?,
        session: text("sessionID", "ask `sessionID`")?,
        action: text("action", "ask `action`")?,
        resources,
    })
}
