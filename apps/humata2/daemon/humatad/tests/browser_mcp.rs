//! Loopback integration tests for BROWSER MCP + Inbox wiring (B2 & B3):
//! - 4-agent scope (`claude`, `codex`, `copilot`, `agy` offered; `opencode` and `zsh` refused)
//! - 30-minute per-session Read grant reused without raising a second card
//! - Write card rejects `AllowFor30Min` and `AlwaysLet`, is skipped by `Approve all`,
//!   and executes once approved with `Approve`.

use humatad_test_common as common;

use std::io::{BufReader, BufWriter};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use common::mcp_client::Mcp;
use common::remote_rig::{CODEX_AGENT, LONG, Rig, is_live};
use common::{Attach, Daemon, WAIT, folder, open, state_folder};
use humata2_browser_copilot::{
    BrowserAction, BrowserReply, BrowserReplyPayload, BrowserRequest, PageSnapshot, TabInfo,
    read_frame, write_frame,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};
use serde_json::{Value, json};
use tonic::Code;

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

async fn token_for(daemon: &Daemon, session_id: &str) -> (String, Attach) {
    let tab = daemon
        .terminals()
        .await
        .open_shell_tab(pb::OpenShellTabRequest {
            idempotency_key: None,
            session: Some(pb::SessionId {
                value: session_id.to_owned(),
            }),
            repo: None,
            folder: String::new(),
        })
        .await
        .unwrap()
        .into_inner()
        .id
        .unwrap();
    let mut attach = Attach::start(daemon, open(&tab, 0)).await;
    let file = daemon.dir.path().join(format!("tok_{session_id}"));
    attach
        .type_text(&format!(
            "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}\r",
            file.display()
        ))
        .await;
    let token = read_when_written(&file).await;
    assert_eq!(token.len(), 32);
    (token, attach)
}

async fn wait_for_pending_request(
    inbox: &mut pb::inbox_service_client::InboxServiceClient<tonic::transport::Channel>,
    kind: pb::RequestKind,
) -> pb::InboxRequest {
    let deadline = Instant::now() + WAIT;
    loop {
        let listed = inbox
            .list_requests(pb::ListRequestsRequest::default())
            .await
            .unwrap()
            .into_inner();
        if let Some(found) = listed
            .requests
            .into_iter()
            .find(|r| r.kind == kind as i32 && r.state == pb::RequestState::Pending as i32)
        {
            return found;
        }
        assert!(Instant::now() < deadline, "never saw pending {kind:?} card");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[tokio::test]
async fn four_agent_scope_offers_browser_tools_only_to_supported_agents() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_channel(&ChannelRecord {
            id: "ch_scope".to_owned(),
            name: "Scope".to_owned(),
            folder: "scope".to_owned(),
            branch_prefix: None,
            default_agent: "claude".to_owned(),
            repos: Vec::new(),
            machines: Vec::new(),
            revision: 1,
            created_unix_ms: 0,
        })
        .unwrap();
    for (sid, agent) in [
        ("se_claude", "claude"),
        ("se_agy", "agy"),
        ("se_opencode", "opencode"),
        ("se_zsh", "zsh"),
    ] {
        store
            .put_session(&SessionRecord {
                id: sid.to_owned(),
                channel: "ch_scope".to_owned(),
                machine: String::new(),
                title: sid.to_owned(),
                agent: agent.to_owned(),
                folder: dir.path().to_owned(),
                created_unix_ms: 0,
            })
            .unwrap();
    }
    drop(store);

    let daemon = Daemon::start_in(dir).await;

    for (sid, expect_offered) in [
        ("se_claude", true),
        ("se_agy", true),
        ("se_opencode", false),
        ("se_zsh", false),
    ] {
        let (token, _attach) = token_for(&daemon, sid).await;
        let mut mcp = Mcp::start(&daemon.socket, &token).await;
        let listed = mcp.call(1, "humata_list", json!({})).await;
        let names: Vec<&str> = listed["structuredContent"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t.as_str()?.split_whitespace().next())
            .collect();
        assert!(
            !names.contains(&"browser_submit"),
            "browser_submit must never be exposed"
        );
        for tool in [
            "browser_list_tabs",
            "browser_read_tab",
            "browser_guide_highlight",
            "browser_click",
            "browser_type",
            "browser_navigate",
        ] {
            assert_eq!(
                names.contains(&tool),
                expect_offered,
                "{sid}: {tool} offered={expect_offered}"
            );
        }
        if !expect_offered {
            let refused = mcp.call(2, "browser_list_tabs", json!({})).await;
            assert_eq!(refused["isError"], Value::Bool(true), "{sid}: {refused}");
        }
    }
}

#[tokio::test]
async fn read_grant_reused_and_write_rejects_30m_and_approve_all() {
    let dir = folder();
    let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
    store
        .put_channel(&ChannelRecord {
            id: "ch_flow".to_owned(),
            name: "Flow".to_owned(),
            folder: "flow".to_owned(),
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
            id: "se_flow".to_owned(),
            channel: "ch_flow".to_owned(),
            machine: String::new(),
            title: "flow".to_owned(),
            agent: "claude".to_owned(),
            folder: dir.path().to_owned(),
            created_unix_ms: 0,
        })
        .unwrap();
    drop(store);

    let daemon = Daemon::start_in(dir).await;
    let channel = daemon.channel().await;
    let mut bridge =
        pb::browser_bridge_service_client::BrowserBridgeServiceClient::new(channel.clone());
    let mut inbox = pb::inbox_service_client::InboxServiceClient::new(channel);

    // Connect a real `humatad2 browser-host` process playing the browser extension.
    let token_file = daemon.state.join("browser-host.token");
    let mut host = Command::new(common::binary("HUMATAD2"))
        .arg("browser-host")
        .arg("--socket")
        .arg(&daemon.socket)
        .arg("--token-file")
        .arg(&token_file)
        .arg("chrome-extension://abcdefghijklmnopabcdefghijklmnop/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn browser-host");
    let mut host_stdin = BufWriter::new(host.stdin.take().unwrap());
    let mut host_stdout = BufReader::new(host.stdout.take().unwrap());

    let responder = tokio::task::spawn_blocking(move || {
        while let Ok(req) = read_frame::<_, BrowserRequest>(&mut host_stdout) {
            let payload = match req.action {
                BrowserAction::ListTabs => BrowserReplyPayload::Tabs {
                    tabs: vec![TabInfo {
                        tab_id: 1,
                        window_id: 1,
                        title: "Example".to_owned(),
                        url: "https://example.com".to_owned(),
                        active: true,
                        pinned: false,
                    }],
                },
                BrowserAction::ReadTab { .. } => BrowserReplyPayload::Snapshot {
                    snapshot: PageSnapshot {
                        tab_id: 1,
                        url: "https://example.com".to_owned(),
                        title: "Example".to_owned(),
                        visible_text: "Hello browser".to_owned(),
                        interactive_elements: Vec::new(),
                    },
                },
                BrowserAction::Click { url, selector, .. } => BrowserReplyPayload::ActionAck {
                    tab_id: 1,
                    url,
                    detail: format!("clicked {selector}"),
                },
                _ => BrowserReplyPayload::ActionAck {
                    tab_id: 1,
                    url: String::new(),
                    detail: "ok".to_owned(),
                },
            };
            let reply = BrowserReply::ok(req.request_id, payload);
            if write_frame(&mut host_stdin, &reply).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + WAIT;
    loop {
        if bridge
            .get_browser_bridge_status(pb::GetBrowserBridgeStatusRequest {})
            .await
            .unwrap()
            .into_inner()
            .connected
        {
            break;
        }
        assert!(Instant::now() < deadline, "browser host never connected");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }

    let (token, _attach) = token_for(&daemon, "se_flow").await;
    let mut mcp = Mcp::start(&daemon.socket, &token).await;

    // 1. First Read raises a BrowserRead card; user answers AllowFor30Min.
    let read_call = tokio::spawn(async move {
        let res = mcp
            .call(10, "browser_list_tabs", json!({ "reason": "check tabs" }))
            .await;
        (mcp, res)
    });
    let read_card = wait_for_pending_request(&mut inbox, pb::RequestKind::BrowserRead).await;
    let read_id = read_card.id.clone().unwrap();
    inbox
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: None,
            id: Some(read_id),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::AllowFor30Min(
                    pb::AllowFor30Min {},
                )),
            }),
            expected_revision: None,
            presence: None,
        })
        .await
        .unwrap();

    let (mut mcp, first_read) = read_call.await.unwrap();
    assert_ne!(first_read["isError"], Value::Bool(true), "{first_read}");

    // 2. Second Read executes immediately under the 30-min grant (no new card).
    let second_read = mcp
        .call(11, "browser_read_tab", json!({ "reason": "read tab" }))
        .await;
    assert_ne!(second_read["isError"], Value::Bool(true), "{second_read}");

    // 3. Write raises a BrowserWrite card; rejects AllowFor30Min & Always; skipped by Approve all.
    let write_call = tokio::spawn(async move {
        let res = mcp
            .call(
                12,
                "browser_click",
                json!({
                    "url": "https://example.com/settings",
                    "selector": "button#confirm",
                    "element_preview": "Confirm",
                    "reason": "confirm setting"
                }),
            )
            .await;
        (mcp, res)
    });
    let write_card = wait_for_pending_request(&mut inbox, pb::RequestKind::BrowserWrite).await;
    let write_id = write_card.id.clone().unwrap();

    let err_30m = inbox
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: None,
            id: Some(write_id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::AllowFor30Min(
                    pb::AllowFor30Min {},
                )),
            }),
            expected_revision: None,
            presence: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err_30m.code(), Code::InvalidArgument);

    let err_always = inbox
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: None,
            id: Some(write_id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Always(pb::AlwaysLet::default())),
            }),
            expected_revision: None,
            presence: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err_always.code(), Code::InvalidArgument);

    let bulk = inbox
        .answer_all(pb::AnswerAllRequest {
            idempotency_key: None,
            scope: None,
            ids: vec![write_id.clone()],
            approve: true,
            presence: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(bulk.answered.is_empty());
    assert_eq!(bulk.skipped.len(), 1);

    // Approve once lets the write execute.
    inbox
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: None,
            id: Some(write_id),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            expected_revision: None,
            presence: None,
        })
        .await
        .unwrap();

    let (_mcp, write_res) = write_call.await.unwrap();
    assert_ne!(write_res["isError"], Value::Bool(true), "{write_res}");

    let _ = host.kill();
    let _ = host.wait();
    responder.abort();
}

/// D891: an agent on a remote machine gets the same browser access to the host
/// Mac as a local session — same 4-agent scope, same 30-min read grants and
/// per-action write approvals, with the Inbox card on the host naming the
/// remote machine, while host setup RPCs stay host-only.
#[tokio::test(flavor = "multi_thread")]
async fn remote_machine_session_reads_and_clicks_through_hosts_browser_with_inbox_attribution() {
    const BOX: &str = "box-1";
    const REPO: (&str, &str) = ("acme", "api");

    let rig = Rig::start_seen(&[BOX], &[REPO]).await;
    let (host_id, box_id) = (rig.host_id().await, Rig::box_id(BOX));
    let channel = rig
        .create_channel("RemoteBrowser", &[REPO], &[host_id, box_id.clone()])
        .await;

    // Start a supported agent (`codex`) session on the remote machine `box-1`.
    let mut start = common::start_request(channel.id.as_ref().unwrap(), &box_id, "RemoteBrowse");
    start.agent = Some(pb::AgentId {
        value: CODEX_AGENT.to_owned(),
    });
    let started = pb::session_service_client::SessionServiceClient::new(rig.channel().await)
        .start_session(start)
        .await
        .expect("start remote codex session")
        .into_inner();
    let ended = rig
        .ended(&started.operation.expect("start has operation"))
        .await;
    assert_eq!(
        ended.state,
        pb::OperationState::Succeeded as i32,
        "remote codex start failed: {:?}",
        ended.error
    );
    let session = started.session.expect("start names session");
    rig.wait_session(&session, "live", is_live).await;

    // Host-only browser setup RPCs are refused on the remote machine daemon.
    let box_daemon = rig.the(BOX).daemon();
    let mut box_bridge = pb::browser_bridge_service_client::BrowserBridgeServiceClient::new(
        box_daemon.channel().await,
    );
    let list_err = box_bridge
        .list_detected_browsers(pb::ListDetectedBrowsersRequest {})
        .await
        .expect_err("ListDetectedBrowsers is host-only");
    assert_eq!(list_err.code(), Code::FailedPrecondition);
    let install_err = box_bridge
        .install_browser_host(pb::InstallBrowserHostRequest::default())
        .await
        .expect_err("InstallBrowserHost is host-only");
    assert_eq!(install_err.code(), Code::FailedPrecondition);
    let uninstall_err = box_bridge
        .uninstall_browser_host(pb::UninstallBrowserHostRequest::default())
        .await
        .expect_err("UninstallBrowserHost is host-only");
    assert_eq!(uninstall_err.code(), Code::FailedPrecondition);

    // Connect a real `humatad2 browser-host` to the HOST Mac daemon.
    let host_channel = rig.channel().await;
    let mut host_bridge =
        pb::browser_bridge_service_client::BrowserBridgeServiceClient::new(host_channel.clone());
    let mut host_inbox = pb::inbox_service_client::InboxServiceClient::new(host_channel);

    let token_file = rig.host.state.join("browser-host.token");
    let mut browser_host = Command::new(common::binary("HUMATAD2"))
        .arg("browser-host")
        .arg("--socket")
        .arg(&rig.host.socket)
        .arg("--token-file")
        .arg(&token_file)
        .arg("chrome-extension://abcdefghijklmnopabcdefghijklmnop/")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn browser-host on host");
    let mut host_stdin = BufWriter::new(browser_host.stdin.take().unwrap());
    let mut host_stdout = BufReader::new(browser_host.stdout.take().unwrap());

    let responder = tokio::task::spawn_blocking(move || {
        while let Ok(req) = read_frame::<_, BrowserRequest>(&mut host_stdout) {
            let payload = match req.action {
                BrowserAction::ListTabs => BrowserReplyPayload::Tabs {
                    tabs: vec![TabInfo {
                        tab_id: 7,
                        window_id: 1,
                        title: "Host Mac Tab".to_owned(),
                        url: "https://host.example.com".to_owned(),
                        active: true,
                        pinned: false,
                    }],
                },
                BrowserAction::ReadTab { .. } => BrowserReplyPayload::Snapshot {
                    snapshot: PageSnapshot {
                        tab_id: 7,
                        url: "https://host.example.com".to_owned(),
                        title: "Host Mac Tab".to_owned(),
                        visible_text: "Hello from host Mac browser".to_owned(),
                        interactive_elements: Vec::new(),
                    },
                },
                BrowserAction::Click { url, selector, .. } => BrowserReplyPayload::ActionAck {
                    tab_id: 7,
                    url,
                    detail: format!("clicked {selector} on host"),
                },
                _ => BrowserReplyPayload::ActionAck {
                    tab_id: 7,
                    url: String::new(),
                    detail: "ok".to_owned(),
                },
            };
            let reply = BrowserReply::ok(req.request_id, payload);
            if write_frame(&mut host_stdin, &reply).is_err() {
                break;
            }
        }
    });

    let deadline = Instant::now() + LONG;
    loop {
        if host_bridge
            .get_browser_bridge_status(pb::GetBrowserBridgeStatusRequest {})
            .await
            .unwrap()
            .into_inner()
            .connected
        {
            break;
        }
        assert!(Instant::now() < deadline, "host browser never connected");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // Start `humatad2 mcp` against the REMOTE machine's daemon as the remote codex session.
    let token = rig.agent_token(BOX, &session).await;
    let mut mcp = Mcp::start(&box_daemon.socket, &token).await;

    // 1. Remote read raises a BrowserRead card mirrored to the host's Inbox naming `box-1`.
    let read_call = tokio::spawn(async move {
        let res = mcp
            .call(101, "browser_list_tabs", json!({ "reason": "remote inspect" }))
            .await;
        (mcp, res)
    });
    let read_card = wait_for_pending_request(&mut host_inbox, pb::RequestKind::BrowserRead).await;
    assert_eq!(
        read_card.journal.as_ref().and_then(|j| j.machine.as_ref()),
        Some(&box_id),
        "remote BrowserRead card must name the remote machine"
    );
    let read_id = read_card.id.clone().unwrap();
    host_inbox
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: None,
            id: Some(read_id),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::AllowFor30Min(
                    pb::AllowFor30Min {},
                )),
            }),
            expected_revision: None,
            presence: None,
        })
        .await
        .unwrap();

    let (mut mcp, first_read) = read_call.await.unwrap();
    assert_ne!(first_read["isError"], Value::Bool(true), "{first_read}");
    assert!(
        first_read.to_string().contains("Host Mac Tab"),
        "remote agent should see host Mac's tab: {first_read}"
    );

    // 2. Second remote read reuses the 30-min read grant without raising a second card.
    let second_read = mcp
        .call(102, "browser_read_tab", json!({ "reason": "remote read tab" }))
        .await;
    assert_ne!(second_read["isError"], Value::Bool(true), "{second_read}");
    assert!(
        second_read
            .to_string()
            .contains("Hello from host Mac browser"),
        "remote agent should read host Mac's tab under 30m grant: {second_read}"
    );

    // 3. Remote write raises a BrowserWrite card naming `box-1`, rejects AllowFor30Min
    //    and Approve all, and executes on the host Mac once approved with Approve.
    let write_call = tokio::spawn(async move {
        let res = mcp
            .call(
                103,
                "browser_click",
                json!({
                    "url": "https://host.example.com/settings",
                    "selector": "button#confirm",
                    "element_preview": "Confirm",
                    "reason": "remote confirm"
                }),
            )
            .await;
        (mcp, res)
    });
    let write_card = wait_for_pending_request(&mut host_inbox, pb::RequestKind::BrowserWrite).await;
    assert_eq!(
        write_card.journal.as_ref().and_then(|j| j.machine.as_ref()),
        Some(&box_id),
        "remote BrowserWrite card must name the remote machine"
    );
    let write_id = write_card.id.clone().unwrap();

    let err_30m = host_inbox
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: None,
            id: Some(write_id.clone()),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::AllowFor30Min(
                    pb::AllowFor30Min {},
                )),
            }),
            expected_revision: None,
            presence: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err_30m.code(), Code::InvalidArgument);

    let bulk = host_inbox
        .answer_all(pb::AnswerAllRequest {
            idempotency_key: None,
            scope: None,
            ids: vec![write_id.clone()],
            approve: true,
            presence: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert!(bulk.answered.is_empty());
    assert_eq!(bulk.skipped.len(), 1);

    host_inbox
        .answer_request(pb::AnswerRequestRequest {
            idempotency_key: None,
            id: Some(write_id),
            answer: Some(pb::RequestAnswer {
                answer: Some(pb::request_answer::Answer::Approve(pb::Approve {})),
            }),
            expected_revision: None,
            presence: None,
        })
        .await
        .unwrap();

    let (_mcp, write_res) = write_call.await.unwrap();
    assert_ne!(write_res["isError"], Value::Bool(true), "{write_res}");
    assert!(
        write_res.to_string().contains("clicked button#confirm on host"),
        "remote click should execute on host browser: {write_res}"
    );

    let _ = browser_host.kill();
    let _ = browser_host.wait();
    responder.abort();
}
