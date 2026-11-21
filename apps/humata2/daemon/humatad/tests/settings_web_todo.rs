//! WEBTODO (D905): Settings switch and loopback To-do web server against the
//! real host daemon (`humatad2`).
//!
//! Verifies:
//! - Off by default (`web_todo_enabled == false`, `web_todo_status == STATE_OFF`).
//! - Turning it on binds a loopback URL (`http://127.0.0.1:<port>/`) and serves
//!   `/` and `/health`.
//! - The switch and running server survive a daemon restart (`daemon.restart()`).
//! - Turning it off stops serving and returns `STATE_OFF`.

use humatad_test_common as common;

use common::Daemon;
use humata2_grpc::humata::v1 as pb;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tonic::transport::Channel;

async fn settings(
    daemon: &Daemon,
) -> pb::settings_service_client::SettingsServiceClient<Channel> {
    pb::settings_service_client::SettingsServiceClient::new(daemon.channel().await)
}

async fn get(daemon: &Daemon) -> pb::Settings {
    settings(daemon)
        .await
        .get_settings(pb::GetSettingsRequest {})
        .await
        .unwrap()
        .into_inner()
}

async fn update(
    daemon: &Daemon,
    request: pb::UpdateSettingsRequest,
) -> Result<pb::Settings, tonic::Status> {
    settings(daemon)
        .await
        .update_settings(request)
        .await
        .map(tonic::Response::into_inner)
}

async fn http_get(url: &str, path: &str) -> Result<String, std::io::Error> {
    let host_port = url
        .strip_prefix("http://")
        .unwrap_or(url)
        .trim_end_matches('/');
    let mut stream = TcpStream::connect(host_port).await?;
    let req = format!("GET {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

#[tokio::test(flavor = "multi_thread")]
async fn web_todo_is_off_by_default_serves_loopback_when_enabled_and_survives_restart() {
    let mut daemon = Daemon::start().await;

    // 1. Off by default.
    let initial = get(&daemon).await;
    assert!(!initial.web_todo_enabled);
    let initial_status = initial.web_todo_status.expect("status present");
    assert_eq!(
        initial_status.state,
        pb::web_todo_status::State::Off as i32
    );
    assert!(initial_status.url.is_empty());

    // 2. Turn on -> listening on loopback and serving HTTP.
    let enabled = update(
        &daemon,
        pb::UpdateSettingsRequest {
            web_todo_enabled: Some(true),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(enabled.web_todo_enabled);
    let status = enabled.web_todo_status.expect("status present");
    assert_eq!(
        status.state,
        pb::web_todo_status::State::Listening as i32,
        "status={status:?}"
    );
    assert!(
        status.url.starts_with("http://127.0.0.1:"),
        "expected loopback url, got {:?}",
        status.url
    );

    let health = http_get(&status.url, "/health").await.expect("GET /health");
    assert!(health.starts_with("HTTP/1.1 200 OK"), "{health}");
    assert!(health.contains(r#"{"ok":true}"#), "{health}");

    let index = http_get(&status.url, "/").await.expect("GET /");
    assert!(index.starts_with("HTTP/1.1 200 OK"), "{index}");
    assert!(index.contains("text/html"), "{index}");

    // 3. Survives daemon restart.
    daemon.restart().await;
    let after_restart = get(&daemon).await;
    assert!(after_restart.web_todo_enabled);
    let restart_status = after_restart.web_todo_status.expect("status after restart");
    assert_eq!(
        restart_status.state,
        pb::web_todo_status::State::Listening as i32,
        "status={restart_status:?}"
    );
    assert!(
        restart_status.url.starts_with("http://127.0.0.1:"),
        "expected loopback url after restart, got {:?}",
        restart_status.url
    );
    let health_after = http_get(&restart_status.url, "/health")
        .await
        .expect("GET /health after restart");
    assert!(health_after.starts_with("HTTP/1.1 200 OK"), "{health_after}");

    // 4. Turn off -> stops serving and reports Off.
    let disabled = update(
        &daemon,
        pb::UpdateSettingsRequest {
            web_todo_enabled: Some(false),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(!disabled.web_todo_enabled);
    let off_status = disabled.web_todo_status.expect("status off");
    assert_eq!(off_status.state, pb::web_todo_status::State::Off as i32);
    assert!(off_status.url.is_empty());
}
