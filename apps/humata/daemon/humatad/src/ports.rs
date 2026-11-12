//! Connections to ports on this host, such as a dev server's, made for the
//! Mac: each one's bytes go back over the protocol, and the Mac's are
//! written to it, so nothing but the host's own connection is opened.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use humata_wire::pb;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::{Mutex, mpsc};

/// What a connection to a port can fail at.
#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error(
        "nothing answered on port {port} of this host; start the server there, and check that it listens on 127.0.0.1 or all addresses"
    )]
    Connect {
        port: u16,
        #[source]
        source: std::io::Error,
    },

    #[error("port {port} isn't a TCP port; give one from 1 to 65535")]
    Invalid { port: u32 },

    #[error("no connection {id} on this host; it closed already. Reload the page to connect again")]
    Unknown { id: String },

    #[error(
        "couldn't send to connection {id}; the server closed it. Reload the page to connect again"
    )]
    Write {
        id: String,
        #[source]
        source: std::io::Error,
    },
}

impl PortError {
    pub fn code(&self) -> pb::ErrorCode {
        match self {
            Self::Connect { .. } | Self::Write { .. } => pb::ErrorCode::HostUnavailable,
            Self::Invalid { .. } => pb::ErrorCode::InvalidRequest,
            Self::Unknown { .. } => pb::ErrorCode::UnknownConnection,
        }
    }
}

/// The open connections, by id, each one's writing half.
#[derive(Default)]
pub struct Ports {
    next: AtomicU64,
    open: Mutex<HashMap<String, Arc<Mutex<OwnedWriteHalf>>>>,
}

/// How much is read from a connection at a time.
const CHUNK: usize = 64 * 1024;

impl Ports {
    /// Connects to `port` on 127.0.0.1, and returns the connection's id and
    /// what it sends, a chunk at a time, until it closes.
    pub async fn connect(
        self: &Arc<Self>,
        port: u32,
    ) -> Result<(String, mpsc::Receiver<Vec<u8>>), PortError> {
        let port = u16::try_from(port)
            .ok()
            .filter(|port| *port != 0)
            .ok_or(PortError::Invalid { port })?;
        let stream = TcpStream::connect(("127.0.0.1", port))
            .await
            .map_err(|source| PortError::Connect { port, source })?;
        let _ = stream.set_nodelay(true);
        let (mut reader, writer) = stream.into_split();
        let id = format!("c{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        self.open
            .lock()
            .await
            .insert(id.clone(), Arc::new(Mutex::new(writer)));
        let (chunks, receiver) = mpsc::channel(16);
        let ports = Arc::clone(self);
        let reading = id.clone();
        tokio::spawn(async move {
            let mut buffer = vec![0; CHUNK];
            loop {
                let read = tokio::select! {
                    read = reader.read(&mut buffer) => read,
                    () = chunks.closed() => break,
                };
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(count) => {
                        if chunks.send(buffer[..count].to_vec()).await.is_err() {
                            break;
                        }
                    }
                }
            }
            ports.open.lock().await.remove(&reading);
        });
        Ok((id, receiver))
    }

    /// Writes `data` to the connection; empty data closes its writing half,
    /// as the Mac's side did.
    pub async fn write(&self, id: &str, data: &[u8]) -> Result<(), PortError> {
        let writer = self
            .open
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| PortError::Unknown { id: id.to_owned() })?;
        let mut writer = writer.lock().await;
        let written = if data.is_empty() {
            writer.shutdown().await
        } else {
            writer.write_all(data).await
        };
        written.map_err(|source| PortError::Write {
            id: id.to_owned(),
            source,
        })
    }

    /// Ends the connection; one already closed is no error.
    pub async fn close(&self, id: &str) {
        let writer = self.open.lock().await.remove(id);
        if let Some(writer) = writer {
            let _ = writer.lock().await.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn a_connection_carries_bytes_both_ways_until_the_server_closes() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut asked = vec![0; 4];
            socket.read_exact(&mut asked).await.unwrap();
            socket.write_all(b"pong").await.unwrap();
            asked
        });

        let ports = Arc::new(Ports::default());
        let (id, mut chunks) = ports.connect(u32::from(port)).await.unwrap();
        ports.write(&id, b"ping").await.unwrap();
        assert_eq!(server.await.unwrap(), b"ping");
        let mut got = Vec::new();
        while let Some(chunk) = chunks.recv().await {
            got.extend(chunk);
        }
        assert_eq!(got, b"pong");
        // Once the server closed, the connection is gone.
        assert!(matches!(
            ports.write(&id, b"more").await,
            Err(PortError::Unknown { .. })
        ));
    }

    #[tokio::test]
    async fn a_port_nothing_listens_on_is_refused() {
        // Bound but not listening, so the port stays taken while it refuses:
        // a port freed instead can be another test's by the time it's tried.
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.bind(([127, 0, 0, 1], 0).into()).unwrap();
        let port = socket.local_addr().unwrap().port();
        let ports = Arc::new(Ports::default());
        assert!(matches!(
            ports.connect(u32::from(port)).await,
            Err(PortError::Connect { .. })
        ));
        assert!(matches!(
            ports.connect(70_000).await,
            Err(PortError::Invalid { .. })
        ));
    }
}
