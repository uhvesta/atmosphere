//! The local socket `humata report record|stop|snapshot` talks to the
//! running app through (D133).
//!
//! One request per connection: the client sends a JSON line, the app
//! answers with a JSON line (`{"ok": Reply}` or `{"refused": Refusal}`)
//! and closes.

use std::io::ErrorKind;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub use crate::error::RemoteError;

/// The longest request line the app reads; a request is a verb and a title.
const MAX_LINE: u64 = 64 * 1024;
/// How long the app waits for a client to send its line before dropping it.
const LINE_TIMEOUT: Duration = Duration::from_secs(5);

/// What `humata report` asks the app to do.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum Request {
    Record { title: Option<String> },
    Stop,
    Snapshot { title: Option<String> },
}

/// What the app did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    /// The saved report's absolute path, once saved.
    pub folder: Option<String>,
    pub recording: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    AlreadyRecording,
    NotRecording,
    Failed,
}

/// Why the app didn't do what was asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct Refusal {
    pub kind: RefusalKind,
    pub message: String,
}

pub type Answer = Result<Reply, Refusal>;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Wire {
    Ok(Reply),
    Refused(Refusal),
}

impl From<Answer> for Wire {
    fn from(answer: Answer) -> Self {
        match answer {
            Ok(reply) => Wire::Ok(reply),
            Err(refusal) => Wire::Refused(refusal),
        }
    }
}

impl From<Wire> for Answer {
    fn from(wire: Wire) -> Self {
        match wire {
            Wire::Ok(reply) => Ok(reply),
            Wire::Refused(refusal) => Err(refusal),
        }
    }
}

/// `HUMATA_APP_SOCKET` if set and non-empty, else `$HOME/.humata2/app.sock`.
pub fn socket_path() -> Result<PathBuf, RemoteError> {
    if let Some(path) = std::env::var_os("HUMATA_APP_SOCKET").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .ok_or(RemoteError::NoHome)?;
    Ok(PathBuf::from(home).join(".humata2").join("app.sock"))
}

/// The app's end: accepts `humata report` requests. Removes its socket
/// file when dropped.
pub struct Listener {
    inner: UnixListener,
    path: PathBuf,
}

impl Listener {
    /// Listens at `path`. Must be called inside a tokio runtime. Creates
    /// the parent folder (0700) if missing. A file already at `path` that
    /// answers means another app is running there; one that doesn't is a
    /// leftover and is replaced. The socket is readable only by this user
    /// (0600).
    pub fn bind(path: &Path) -> Result<Self, RemoteError> {
        let bind_err = |source| RemoteError::Bind {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(bind_err)?;
        }
        if std::fs::symlink_metadata(path).is_ok() {
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(RemoteError::InUse {
                    path: path.to_path_buf(),
                });
            }
            std::fs::remove_file(path).map_err(bind_err)?;
        }
        let inner = UnixListener::bind(path).map_err(bind_err)?;
        let listener = Listener {
            inner,
            path: path.to_path_buf(),
        };
        // Dropping `listener` on failure removes the file again.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(bind_err)?;
        Ok(listener)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Waits for the next request. A connection that sends something
    /// that isn't a request is told so and skipped.
    pub async fn next(&mut self) -> Result<(Request, Responder), RemoteError> {
        loop {
            let (stream, _) = self.inner.accept().await.map_err(|source| RemoteError::Io {
                path: self.path.clone(),
                source,
            })?;
            let mut reader = BufReader::new(stream.take(MAX_LINE));
            let mut line = String::new();
            let read = tokio::time::timeout(LINE_TIMEOUT, reader.read_line(&mut line)).await;
            let mut stream = reader.into_inner().into_inner();
            let Ok(Ok(n)) = read else {
                continue; // too slow, or hung up
            };
            if n == 0 {
                continue;
            }
            match serde_json::from_str::<Request>(line.trim_end()) {
                Ok(request) => {
                    return Ok((
                        request,
                        Responder {
                            stream,
                            path: self.path.clone(),
                        },
                    ));
                }
                Err(_) => {
                    let refusal = Refusal {
                        kind: RefusalKind::Failed,
                        message: "that isn't a request Humata 2 understands; make sure \
                                  `humata` and the app are the same version"
                            .to_string(),
                    };
                    if let Ok(mut bytes) = serde_json::to_vec(&Wire::Refused(refusal)) {
                        bytes.push(b'\n');
                        // The client may have gone; nothing to do then.
                        let _ = stream.write_all(&bytes).await;
                        let _ = stream.shutdown().await;
                    }
                }
            }
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        // Already gone is fine.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// The way to answer one request.
pub struct Responder {
    stream: UnixStream,
    path: PathBuf,
}

impl Responder {
    /// Sends the answer as one JSON line, then closes the connection.
    pub async fn reply(mut self, answer: Answer) -> Result<(), RemoteError> {
        let mut bytes = serde_json::to_vec(&Wire::from(answer))
            .map_err(|source| RemoteError::Encode { source })?;
        bytes.push(b'\n');
        let io = |source| RemoteError::Io {
            path: self.path.clone(),
            source,
        };
        self.stream.write_all(&bytes).await.map_err(io)?;
        self.stream.shutdown().await.map_err(io)
    }
}

/// Sends `request` to the app at `path` and waits for its answer.
pub async fn ask(path: &Path, request: &Request) -> Result<Answer, RemoteError> {
    let io = |source| RemoteError::Io {
        path: path.to_path_buf(),
        source,
    };
    let stream = match UnixStream::connect(path).await {
        Ok(stream) => stream,
        Err(e) if matches!(e.kind(), ErrorKind::NotFound | ErrorKind::ConnectionRefused) => {
            return Err(RemoteError::NotRunning {
                path: path.to_path_buf(),
            });
        }
        Err(e) => return Err(io(e)),
    };
    let mut bytes =
        serde_json::to_vec(request).map_err(|source| RemoteError::Encode { source })?;
    bytes.push(b'\n');
    let mut reader = BufReader::new(stream);
    reader.write_all(&bytes).await.map_err(io)?;
    reader.flush().await.map_err(io)?;
    let mut line = String::new();
    let n = reader.read_line(&mut line).await.map_err(io)?;
    if n == 0 {
        return Err(io(ErrorKind::UnexpectedEof.into()));
    }
    let wire: Wire =
        serde_json::from_str(line.trim_end()).map_err(|source| RemoteError::Decode { source })?;
    Ok(wire.into())
}
