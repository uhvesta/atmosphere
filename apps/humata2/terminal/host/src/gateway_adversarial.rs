//! Real socket probes against an isolated gateway, never the live demo.
use crate::{Config, Host, auth::Token, gateway};
use futures_util::{SinkExt, StreamExt};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use terminal_session::SessionStore;
use terminal_wire::{ClientFrame, HistoryPolicy, PROTOCOL_VERSION, ServerFrame};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
struct Harness {
    _state: tempfile::TempDir,
    host: Host,
    url: String,
    token: String,
    server: tokio::task::JoinHandle<()>,
}
impl Harness {
    async fn start() -> Self {
        Self::start_with_shell(false).await
    }
    async fn start_with_shell(shell: bool) -> Self {
        let state = tempfile::Builder::new()
            .prefix("hws-")
            .tempdir_in("/tmp")
            .unwrap();
        std::fs::set_permissions(state.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let assets = state.path().join("assets");
        std::fs::create_dir(&assets).unwrap();
        std::fs::write(assets.join("index.html"), "isolated gateway").unwrap();
        let token_path = state.path().join("token");
        let token = Token::load_or_create(&token_path).unwrap();
        let secret = std::fs::read_to_string(token_path).unwrap();
        let holder = std::fs::canonicalize(std::env::var_os("HUMATA2_HOLDER").unwrap()).unwrap();
        let store = SessionStore::open_memory_only(state.path(), holder).unwrap();
        let host = Host::start(
            Config {
                state: state.path().into(),
                label: "isolated".into(),
                start_profiles: if shell { vec!["shell".into()] } else { vec![] },
                default_history: HistoryPolicy::Off,
            },
            store,
            if shell {
                vec![crate::Profile {
                    name: "shell".into(),
                    title: "Burst fixture".into(),
                    program: "/bin/sh".into(),
                    args: vec!["-i".into()],
                    cwd: state.path().into(),
                    endpoint: terminal_session::Endpoint::Local,
                    recovery: None,
                    history: HistoryPolicy::Off,
                }]
            } else {
                vec![]
            },
        )
        .await
        .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = gateway::router(host.clone(), assets, token);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            _state: state,
            host,
            url: format!("ws://{address}/ws"),
            token: secret,
            server,
        }
    }
    async fn authenticated(&self) -> Socket {
        let (mut socket, _) = connect_async(&self.url).await.unwrap();
        send(
            &mut socket,
            ClientFrame::Authenticate {
                protocol: PROTOCOL_VERSION,
                token: self.token.clone(),
            },
        )
        .await;
        assert!(matches!(
            receive(&mut socket).await,
            ServerFrame::Sessions { .. }
        ));
        socket
    }
    async fn stop(self) {
        self.server.abort();
        self.host.shutdown().await.unwrap();
    }
}
async fn send(socket: &mut Socket, frame: ClientFrame) {
    socket
        .send(Message::Text(serde_json::to_string(&frame).unwrap().into()))
        .await
        .unwrap();
}
async fn receive(socket: &mut Socket) -> ServerFrame {
    let message = tokio::time::timeout(Duration::from_secs(3), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}
async fn assert_alive(socket: &mut Socket) {
    send(socket, ClientFrame::List).await;
    assert!(matches!(
        receive(socket).await,
        ServerFrame::Sessions { .. }
    ));
}
#[tokio::test]
async fn malformed_oversized_or_unauthenticated_peers_cannot_disrupt_a_valid_viewer() {
    let fixture = Harness::start().await;
    let mut valid = fixture.authenticated().await;
    for frame in [
        ClientFrame::Authenticate {
            protocol: PROTOCOL_VERSION,
            token: "wrong-token".into(),
        },
        ClientFrame::Authenticate {
            protocol: PROTOCOL_VERSION + 1,
            token: fixture.token.clone(),
        },
        ClientFrame::Authenticate {
            protocol: PROTOCOL_VERSION,
            token: "x".repeat(2048),
        },
    ] {
        let (mut attacker, _) = connect_async(&fixture.url).await.unwrap();
        send(&mut attacker, frame).await;
        assert!(
            matches!(receive(&mut attacker).await,ServerFrame::Error{code,..}if code=="authentication")
        );
        assert_alive(&mut valid).await;
    }
    for malformed in [
        r#"{"type":"input","id":1,"seq":1,"bytes":[256]}"#,
        r#"{"type":"list","unexpected":"PRIVATE_MARKER"}"#,
        r#"{"type":"execute_untrusted_program","args":["PRIVATE_MARKER"]}"#,
        "{",
    ] {
        let mut attacker = fixture.authenticated().await;
        attacker
            .send(Message::Text(malformed.into()))
            .await
            .unwrap();
        let result = receive(&mut attacker).await;
        assert!(matches!(&result,ServerFrame::Error{code,..}if code=="invalid_frame"));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("PRIVATE_MARKER")
        );
        assert_alive(&mut valid).await;
    }
    let mut attacker = fixture.authenticated().await;
    let _ = attacker
        .send(Message::Text("x".repeat((2 << 20) + 1).into()))
        .await;
    let disconnected = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match attacker.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break true,
                _ => {}
            }
        }
    })
    .await
    .unwrap();
    assert!(disconnected);
    assert_alive(&mut valid).await;
    drop(valid);
    fixture.stop().await;
}
#[tokio::test]
async fn origin_and_host_spoofing_are_rejected_before_authentication() {
    let fixture = Harness::start().await;
    for origin in [
        "null",
        "file:///tmp/viewer",
        "https://attacker.example",
        "http://localhost:1",
        "http://127.0.0.1:1/path",
        "http://127.0.0.1:1/?query",
    ] {
        let mut request = fixture.url.clone().into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", origin.parse().unwrap());
        assert!(connect_async(request).await.is_err(), "{origin}");
    }
    let mut request = fixture.url.clone().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Host", "attacker.example".parse().unwrap());
    request
        .headers_mut()
        .insert("Origin", "http://attacker.example".parse().unwrap());
    assert!(connect_async(request).await.is_err());
    let mut valid = fixture.authenticated().await;
    assert_alive(&mut valid).await;
    drop(valid);
    fixture.stop().await;
}
#[tokio::test]
async fn pending_unauthenticated_upgrades_have_a_bounded_connection_budget() {
    let fixture = Harness::start().await;
    let mut pending = Vec::new();
    for _ in 0..16 {
        pending.push(connect_async(&fixture.url).await.unwrap().0);
    }
    let rejected = connect_async(&fixture.url).await.is_err();
    drop(pending);
    fixture.stop().await;
    assert!(
        rejected,
        "unauthenticated sockets must share the gateway's finite connection budget"
    );
}

#[tokio::test]
async fn oversized_structured_fields_are_rejected_before_waiting_for_the_actor() {
    let fixture = Harness::start().await;
    let mut socket = fixture.authenticated().await;
    let (ready, started) = tokio::sync::oneshot::channel();
    let (release, resume) = tokio::sync::oneshot::channel();
    fixture
        .host
        .tx
        .send(crate::Command::PauseFixture {
            ready,
            release: resume,
        })
        .await
        .unwrap();
    started.await.unwrap();
    send(
        &mut socket,
        ClientFrame::SetRecovery {
            id: 1,
            hint: Some(terminal_wire::RecoveryHint {
                provider: "fixture".into(),
                session_id: "explicit".into(),
                working_directory: "/tmp".into(),
                state_paths: vec!["/tmp".into(); 17],
            }),
        },
    )
    .await;
    let response = tokio::time::timeout(Duration::from_millis(500), socket.next()).await;
    let _ = release.send(());
    drop(socket);
    fixture.stop().await;
    let rejected = matches!(response,Ok(Some(Ok(Message::Text(ref text))))if matches!(serde_json::from_str::<ServerFrame>(text),Ok(ServerFrame::Error{code,..})if code=="invalid_recovery"));
    assert!(
        rejected,
        "malformed variable-sized payload was queued behind the blocked actor"
    );
}

#[tokio::test]
async fn active_gateway_reader_survives_a_large_plain_output_burst() {
    use ghostty_vt::{Grid, Terminal};
    let fixture = Harness::start_with_shell(true).await;
    let mut socket = fixture.authenticated().await;
    send(&mut socket, ClientFrame::Open { id: 1 }).await;
    loop {
        if matches!(
            receive(&mut socket).await,
            ServerFrame::Control {
                id: 1,
                controller: true
            }
        ) {
            break;
        }
    }
    send(&mut socket,ClientFrame::Input{id:1,seq:1,bytes:b"stty -echo; awk 'BEGIN {for(i=0;i<12000;i++) printf \"BURST_ROW_%05d abcdefghijklmnopqrstuvwxyz0123456789\\n\",i; print \"BURST_FINISHED\"}'\n".to_vec()}).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut output = String::new();
    let mut done = false;
    let mut closed = false;
    while !done && tokio::time::Instant::now() < deadline {
        let message = tokio::time::timeout_at(deadline, socket.next()).await;
        let frame = match message {
            Ok(Some(Ok(Message::Text(text)))) => {
                serde_json::from_str::<ServerFrame>(&text).unwrap()
            }
            _ => {
                closed = true;
                break;
            }
        };
        match frame {
            ServerFrame::Output { id, offset, bytes } => {
                let end = offset + bytes.len() as u64;
                output.push_str(&String::from_utf8_lossy(&bytes));
                send(&mut socket, ClientFrame::Ack { id, offset: end }).await;
                done = output.contains("BURST_FINISHED\r\n");
            }
            ServerFrame::ResyncRequired { id, .. } => {
                send(&mut socket, ClientFrame::SnapshotRequest { id }).await
            }
            ServerFrame::Snapshot {
                bytes, offset, id, ..
            } => {
                let mut vt = Terminal::from_checkpoint(&bytes).unwrap();
                vt.sync();
                let text = (0..vt.size().rows)
                    .filter_map(|y| vt.row(y))
                    .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
                    .collect::<String>();
                done = text.contains("BURST_FINISHED");
                send(&mut socket, ClientFrame::Ack { id, offset }).await;
            }
            _ => {}
        }
    }
    if !closed {
        send(
            &mut socket,
            ClientFrame::Input {
                id: 1,
                seq: 2,
                bytes: b"printf 'AFTER_BURST\\n'\n".to_vec(),
            },
        )
        .await;
        loop {
            match receive(&mut socket).await {
                ServerFrame::Output { id, offset, bytes } => {
                    let found = String::from_utf8_lossy(&bytes).contains("AFTER_BURST");
                    send(
                        &mut socket,
                        ClientFrame::Ack {
                            id,
                            offset: offset + bytes.len() as u64,
                        },
                    )
                    .await;
                    if found {
                        break;
                    }
                }
                ServerFrame::ResyncRequired { id, .. } => {
                    send(&mut socket, ClientFrame::SnapshotRequest { id }).await
                }
                ServerFrame::Snapshot {
                    id, offset, bytes, ..
                } => {
                    let mut vt = Terminal::from_checkpoint(&bytes).unwrap();
                    vt.sync();
                    let found = (0..vt.size().rows)
                        .filter_map(|y| vt.row(y))
                        .flat_map(|row| row.cells.iter().map(|cell| cell.text.as_str()))
                        .collect::<String>()
                        .contains("AFTER_BURST");
                    send(&mut socket, ClientFrame::Ack { id, offset }).await;
                    if found {
                        break;
                    }
                }
                _ => {}
            }
        }
        send(&mut socket, ClientFrame::Stop { id: 1 }).await;
        loop {
            if matches!(
                receive(&mut socket).await,
                ServerFrame::Exited { id: 1, .. }
            ) {
                break;
            }
        }
    } else {
        // Pre-fix failure cleanup is an independent native controller.
        let mut view = fixture.host.subscribe().await.unwrap();
        let _ = view.receiver.recv().await;
        fixture
            .host
            .frame(view.id, ClientFrame::Open { id: 1 })
            .await
            .unwrap();
        fixture
            .host
            .frame(view.id, ClientFrame::TakeControl { id: 1 })
            .await
            .unwrap();
        loop {
            if matches!(
                view.receiver.recv().await.unwrap().frame,
                ServerFrame::Control {
                    controller: true,
                    ..
                }
            ) {
                break;
            }
        }
        fixture
            .host
            .frame(view.id, ClientFrame::Stop { id: 1 })
            .await
            .unwrap();
        loop {
            if matches!(
                view.receiver.recv().await.unwrap().frame,
                ServerFrame::Exited { id: 1, .. }
            ) {
                break;
            }
        }
    }
    drop(socket);
    fixture.stop().await;
    assert!(
        done && !closed,
        "a receiving and ACKing viewer lost its socket during a sub-window output burst"
    );
}
