//! Loopback integration tests for `humata2-browser-host` (A3 & A4):
//! - atomic `argv[0]` symlink installation & manifest hygiene in a fake browser home
//! - 4-byte native-endian JSON framing over stdio <-> gRPC `ConnectBrowserHost`
//! - `0o600` token file, wrong-token / wrong-method / inside-session auth rejection

use humatad_test_common as common;

use std::io::{BufReader, BufWriter};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::{Attach, Build, Daemon, WAIT, folder, open, state_folder};
use humata2_browser_copilot::{
    BrowserAction, BrowserReply, BrowserReplyPayload, BrowserRequest, TabInfo, read_frame,
    write_frame,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::metadata::MetadataValue;
use tonic::{Code, Request};

const BROWSER_HOST_HEADER: &str = "humata-browser-host-token";
const CHROMIUM_ORIGIN: &str = "chrome-extension://abcdefghijklmnopabcdefghijklmnop/";

async fn wait_for_connected(
    client: &mut pb::browser_bridge_service_client::BrowserBridgeServiceClient<
        tonic::transport::Channel,
    >,
    want: bool,
) {
    let deadline = Instant::now() + WAIT;
    loop {
        let status = client
            .get_browser_bridge_status(pb::GetBrowserBridgeStatusRequest {})
            .await
            .unwrap()
            .into_inner();
        if status.connected == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "browser bridge connected state never became {want}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

async fn read_when_written(path: &Path) -> String {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && !text.is_empty()
        {
            return text;
        }
        assert!(Instant::now() < deadline, "{} was never written", path.display());
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test]
async fn install_creates_argv0_symlink_and_round_trips_4byte_native_frames() {
    let dir = folder();
    let fake_home = dir.path().join("fake_home");
    std::fs::create_dir_all(fake_home.join("Applications/Google Chrome.app")).unwrap();
    let home_str = fake_home.to_string_lossy().into_owned();

    let daemon = Daemon::start_with_env(
        Build::Admin,
        dir,
        &[],
        &[("HUMATA2_BROWSER_HOME", &home_str)],
    )
    .await;

    // Token file is created with strict 0o600 permissions.
    let token_file = daemon.state.join("browser-host.token");
    let mode = std::fs::metadata(&token_file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "browser-host.token must be 0o600");

    let channel = daemon.channel().await;
    let mut bridge = pb::browser_bridge_service_client::BrowserBridgeServiceClient::new(channel);

    // Install into fake home: creates symlink + manifest without touching real home.
    let installed = bridge
        .install_browser_host(pb::InstallBrowserHostRequest {
            idempotency_key: None,
            kinds: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(installed.browsers.len(), 1);
    let chrome = &installed.browsers[0];
    assert!(chrome.host_installed);
    assert!(chrome.host_matches_current);

    let symlink = fake_home.join(".humata2/bin/humata2-browser-host");
    assert!(symlink.is_symlink());

    // Launch the real `humata2-browser-host` symlink as a browser would (argv[1] = origin).
    let mut host = Command::new(&symlink)
        .arg(CHROMIUM_ORIGIN)
        .env("HUMATA_SOCKET", &daemon.socket)
        .env("HUMATA_BROWSER_HOST_TOKEN_FILE", &token_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn humata2-browser-host symlink");

    wait_for_connected(&mut bridge, true).await;

    let mut host_stdin = BufWriter::new(host.stdin.take().unwrap());
    let mut host_stdout = BufReader::new(host.stdout.take().unwrap());

    // Drive one real ExecuteBrowserAction through the daemon -> symlink host -> stdio -> daemon.
    let mut bridge_exec = bridge.clone();
    let exec_task = tokio::spawn(async move {
        bridge_exec
            .execute_browser_action(pb::ExecuteBrowserActionRequest {
                approval: None,
                action_json: serde_json::to_string(&BrowserAction::ListTabs).unwrap(),
            })
            .await
            .unwrap()
            .into_inner()
    });

    let round_trip = tokio::task::spawn_blocking(move || {
        let req: BrowserRequest = read_frame(&mut host_stdout).expect("read request frame");
        assert_eq!(req.action, BrowserAction::ListTabs);
        let reply = BrowserReply::ok(
            req.request_id,
            BrowserReplyPayload::Tabs {
                tabs: vec![TabInfo {
                    tab_id: 9,
                    window_id: 1,
                    title: "Humata".to_owned(),
                    url: "https://humata.example/".to_owned(),
                    active: true,
                    pinned: false,
                }],
            },
        );
        write_frame(&mut host_stdin, &reply).expect("write reply frame");
        drop(host_stdin);
    });

    let response = tokio::time::timeout(WAIT, exec_task)
        .await
        .expect("execute timed out")
        .unwrap();
    round_trip.await.unwrap();

    let decoded: BrowserReply = serde_json::from_str(&response.reply_json).unwrap();
    assert!(decoded.ok);
    assert!(matches!(
        decoded.payload,
        Some(BrowserReplyPayload::Tabs { ref tabs }) if tabs.len() == 1 && tabs[0].tab_id == 9
    ));

    // Closing host stdin disconnects cleanly.
    let status = tokio::task::spawn_blocking(move || host.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "host exited with {status}");
    wait_for_connected(&mut bridge, false).await;

    // Uninstall removes the manifest cleanly.
    let after = bridge
        .uninstall_browser_host(pb::UninstallBrowserHostRequest {
            idempotency_key: None,
            kinds: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!after.browsers[0].host_installed);
}

#[tokio::test]
async fn browser_host_token_rejects_bad_token_wrong_method_and_in_session_caller() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_channel(&ChannelRecord {
            id: "ch_bh".to_owned(),
            name: "BH".to_owned(),
            folder: "bh".to_owned(),
            branch_prefix: None,
            default_agent: "claude".to_owned(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    store
        .put_session(&SessionRecord {
            id: "se_bh".to_owned(),
            channel: "ch_bh".to_owned(),
            machine: String::new(),
            title: "bh".to_owned(),
            agent: "claude".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);

    let daemon = Daemon::start_in(dir).await;
    let token_file = daemon.state.join("browser-host.token");
    let token = std::fs::read_to_string(&token_file).unwrap().trim().to_owned();
    let channel = daemon.channel().await;
    let mut bridge = pb::browser_bridge_service_client::BrowserBridgeServiceClient::new(channel);

    // 1. Forged token on ConnectBrowserHost is rejected with Unauthenticated.
    let (tx, rx) = mpsc::channel(2);
    let _ = tx
        .send(pb::BrowserHostFrame {
            frame: Some(pb::browser_host_frame::Frame::Hello(pb::BrowserHostHello {
                extension_origin: CHROMIUM_ORIGIN.to_owned(),
                host_pid: std::process::id(),
            })),
        })
        .await;
    let mut bad_req = Request::new(ReceiverStream::new(rx));
    bad_req.metadata_mut().insert(
        BROWSER_HOST_HEADER,
        MetadataValue::try_from("00000000000000000000000000000000").unwrap(),
    );
    let err = bridge.connect_browser_host(bad_req).await.unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);

    // 2. Valid browser-host token on any other method is rejected with PermissionDenied.
    let mut wrong_method = Request::new(pb::GetBrowserBridgeStatusRequest {});
    wrong_method.metadata_mut().insert(
        BROWSER_HOST_HEADER,
        MetadataValue::try_from(token.as_str()).unwrap(),
    );
    let err = bridge
        .get_browser_bridge_status(wrong_method)
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);

    // 3. Running `humatad2 browser-host` from inside a session tab is rejected (!in_a_session).
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId {
                value: "se_bh".to_owned(),
            }),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
    let code_file = daemon.dir.path().join("exit_code");
    let humatad2 = common::binary("HUMATAD2");
    attach
        .type_text(&format!(
            "{} browser-host --socket {} --token-file {} < /dev/null >/dev/null 2>&1; printf '%s' $? > {}\r",
            humatad2.display(),
            daemon.socket.display(),
            token_file.display(),
            code_file.display()
        ))
        .await;
    let exit_code = read_when_written(&code_file).await;
    assert_ne!(
        exit_code.trim(),
        "0",
        "browser-host spawned inside a session tab must be rejected"
    );
}
