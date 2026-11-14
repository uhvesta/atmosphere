//! Port forwarding: the daemon listens on 127.0.0.1 on the Mac and carries
//! each connection to a port on a host, over the host's connection, so a
//! dev server there opens in the browser here.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use humata_wire::{Reply, pb};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::Daemon;
use crate::error::DaemonError;

/// A forward the daemon listens for, until it's stopped.
pub(super) struct Forwarding {
    forward: pb::Forward,
    listening: tokio::task::AbortHandle,
}

/// How much of the browser's side is read at a time.
const CHUNK: usize = 64 * 1024;

impl Daemon {
    /// Starts listening for `request`'s port, after checking something
    /// answers on it, so a mistyped port fails here and not in the browser.
    pub(super) async fn forward_port(
        &self,
        request: pb::ForwardPort,
    ) -> Result<Reply, DaemonError> {
        let port = u16::try_from(request.port)
            .ok()
            .filter(|port| *port != 0)
            .ok_or(DaemonError::BadPort { port: request.port })?;
        let host = request.host;
        let probe = self.forward(&host, connect(&host, port)).await?;
        match probe {
            Reply::Stream {
                response: pb::response::Kind::PortConnected(connected),
                ..
            } => {
                self.close_port(&host, connected.connection_id).await;
            }
            Reply::Response(pb::response::Kind::Error(error)) => {
                return Ok(Reply::Response(pb::response::Kind::Error(error)));
            }
            _ => return Err(DaemonError::NotAPortStream { host }),
        }
        // The same port here when it's free, so links the server prints
        // work as they are; else any.
        let listener = match TcpListener::bind(("127.0.0.1", port)).await {
            Ok(listener) if !host.is_empty() => listener,
            _ => TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(|source| DaemonError::Listen { source })?,
        };
        let local_port = listener
            .local_addr()
            .map_err(|source| DaemonError::Listen { source })?
            .port();
        let id = format!("f{}", self.next_forward.fetch_add(1, Ordering::Relaxed) + 1);
        let forward = pb::Forward {
            id: id.clone(),
            host: host.clone(),
            port: u32::from(port),
            local_port: u32::from(local_port),
            session_id: request.session_id,
        };
        let me = self.me.clone();
        let listening = tokio::spawn(async move {
            loop {
                let Ok((socket, _)) = listener.accept().await else {
                    return;
                };
                let Some(daemon) = me.upgrade() else { return };
                let host = host.clone();
                tokio::spawn(async move { daemon.carry(&host, port, socket).await });
            }
        })
        .abort_handle();
        self.forwards
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id,
                Forwarding {
                    forward: forward.clone(),
                    listening,
                },
            );
        Ok(Reply::Response(pb::response::Kind::PortForwarded(
            pb::PortForwarded {
                forward: Some(forward),
            },
        )))
    }

    pub(super) fn forwards_list(&self) -> Vec<pb::Forward> {
        self.forwards
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|forwarding| forwarding.forward.clone())
            .collect()
    }

    /// Stops listening; connections already open run until they close.
    pub(super) fn stop_forward(&self, id: &str) -> Result<(), DaemonError> {
        let forwarding = self
            .forwards
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(id)
            .ok_or_else(|| DaemonError::UnknownForward { id: id.to_owned() })?;
        forwarding.listening.abort();
        Ok(())
    }

    /// Carries one browser connection to the port on `host`, both ways,
    /// until both sides are done. A port that stopped answering drops it.
    async fn carry(&self, host: &str, port: u16, socket: TcpStream) {
        let Ok(Reply::Stream {
            response: pb::response::Kind::PortConnected(connected),
            items: mut from_server,
        }) = self.forward(host, connect(host, port)).await
        else {
            return;
        };
        let id = connected.connection_id;
        let _ = socket.set_nodelay(true);
        let mut socket = socket;
        let (mut reader, mut writer) = socket.split();
        let to_server = async {
            let mut buffer = vec![0; CHUNK];
            loop {
                let count = reader.read(&mut buffer).await.unwrap_or(0);
                // Empty data closes the server's side to match.
                let write = pb::request::Kind::WritePort(pb::WritePort {
                    host: host.to_owned(),
                    connection_id: id.clone(),
                    data: buffer[..count].to_vec(),
                });
                let written = self.forward(host, write).await;
                let sent = matches!(
                    written,
                    Ok(Reply::Response(pb::response::Kind::PortWritten(_)))
                );
                if count == 0 || !sent {
                    return;
                }
            }
        };
        // True if the port's bytes stopped short of what it sent, as when
        // this side fell too far behind reading them.
        let to_browser = async {
            loop {
                match from_server.recv().await {
                    Some(Ok(item)) => {
                        if let Some(pb::stream_item::Kind::PortData(data)) = item.kind
                            && writer.write_all(&data.data).await.is_err()
                        {
                            break;
                        }
                    }
                    Some(Err(_)) => return true,
                    None => break,
                }
            }
            let _ = writer.shutdown().await;
            false
        };
        let (cut_short, ()) = tokio::join!(to_browser, to_server);
        if cut_short {
            // Closed as broken, so the browser reports a failed load
            // instead of taking half a response for a whole one.
            let _ = socket.set_zero_linger();
        }
        drop(socket);
        self.close_port(host, id).await;
    }

    async fn close_port(&self, host: &str, connection_id: String) {
        let close = pb::request::Kind::ClosePort(pb::ClosePort {
            host: host.to_owned(),
            connection_id,
        });
        let _ = self.forward(host, close).await;
    }
}

fn connect(host: &str, port: u16) -> pb::request::Kind {
    pb::request::Kind::ConnectPort(pb::ConnectPort {
        host: host.to_owned(),
        port: u32::from(port),
    })
}
