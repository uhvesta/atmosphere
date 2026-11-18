//! An agent's `humata access ask-ahead` against the real daemon (agents-14,
//! CLI.md §7, SE-23's Inbox, TR-7). The real `humata` runs as the agent of
//! a session's shell tab, with that tab's session token; the test is the
//! user in the app and answers in the Inbox. Each test checks the exit
//! code, the JSON printed and what the Inbox holds, never a message's
//! wording.

mod common;

use std::process::Stdio;
use std::time::Duration;

use common::{
    Attach, Daemon, approve, binary, first_pending, folder, humata_as_agent, pending_requests,
    shell_token, state_folder,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{ChannelRecord, SessionRecord, Store};

const SESSION: &str = "se_wait";
const CHANNEL: &str = "ch_wait";

// The CLI's exit codes (cli/src/exit.rs), by meaning.
const SUCCESS: i32 = 0;
const DENIED: i32 = 4;
const PENDING: i32 = 12;
const INTERRUPTED: i32 = 130;

/// What the agent asks for ahead of time, and why.
const ASK: [&str; 6] = [
    "access",
    "ask-ahead",
    "network_reach",
    "network:localhost:5432",
    "--reason",
    "run the database tests",
];

/// A daemon that knows one plain-shell session, with a shell tab open in
/// it, and the session token that tab's shell was given.
struct Fixture {
    daemon: Daemon,
    token: String,
    _shell: Attach,
}

impl Fixture {
    async fn start() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        store
            .put_channel(&ChannelRecord {
                id: CHANNEL.to_owned(),
                name: "Waiting".to_owned(),
                folder: "waiting".to_owned(),
                branch_prefix: None,
                default_agent: "zsh".to_owned(),
                repos: Vec::new(),
                machines: Vec::new(),
                revision: 1,
                created_unix_ms: 0,
            })
            .unwrap();
        store
            .put_session(&SessionRecord {
                id: SESSION.to_owned(),
                channel: CHANNEL.to_owned(),
                machine: String::new(),
                title: "waiting".to_owned(),
                agent: "zsh".to_owned(),
                folder: dir.path().to_owned(),
                created_unix_ms: 0,
            })
            .unwrap();
        drop(store);
        let daemon = Daemon::start_in(dir).await;
        let (token, shell) = shell_token(
            &daemon,
            &pb::SessionId {
                value: SESSION.to_owned(),
            },
        )
        .await;
        Self {
            daemon,
            token,
            _shell: shell,
        }
    }

    /// `humata --json <words> <ASK>` as the session's agent.
    async fn ask(&self, words: &[&str]) -> (i32, serde_json::Value) {
        let all: Vec<&str> = words.iter().copied().chain(ASK).collect();
        humata_as_agent(&self.daemon, &self.token, &all).await
    }

    async fn deny(&self, id: &pb::RequestId) {
        self.daemon
            .inbox_service()
            .await
            .answer_request(pb::AnswerRequestRequest {
                id: Some(id.clone()),
                answer: Some(pb::RequestAnswer {
                    answer: Some(pb::request_answer::Answer::Deny(pb::Deny {
                        note: "not the database".to_owned(),
                    })),
                }),
                ..Default::default()
            })
            .await
            .unwrap();
    }
}

/// Agents-14 step 12: `--json` doesn't wait. It exits pending and names
/// the request, which is in the Inbox.
#[tokio::test(flavor = "multi_thread")]
async fn json_doesnt_wait_and_exits_pending_naming_the_request_in_the_inbox() {
    let fixture = Fixture::start().await;
    let (code, printed) = fixture.ask(&[]).await;
    assert_eq!(code, PENDING, "{printed}");
    assert_eq!(printed["error"]["kind"], "pending", "{printed}");
    let waiting = pending_requests(&fixture.daemon).await;
    assert_eq!(waiting.len(), 1, "one request is in the Inbox");
    assert_eq!(printed["error"]["details"]["request"], waiting[0].value, "{printed}");
}

/// Agents-14 step 13: `--wait N` holds N seconds, then exits pending with
/// the request still in the Inbox for the user.
#[tokio::test(flavor = "multi_thread")]
async fn a_wait_that_runs_out_leaves_the_request_pending() {
    let fixture = Fixture::start().await;
    let (code, printed) = fixture.ask(&["--wait", "1"]).await;
    assert_eq!(code, PENDING, "{printed}");
    let waiting = pending_requests(&fixture.daemon).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(printed["error"]["details"]["request"], waiting[0].value, "{printed}");
}

/// Agents-14 step 14: a wait ends when the user approves, and the run
/// succeeds.
#[tokio::test(flavor = "multi_thread")]
async fn a_wait_ends_with_the_users_yes() {
    let fixture = Fixture::start().await;
    let ((code, printed), ()) = tokio::join!(fixture.ask(&["--wait", "120"]), async {
        let id = first_pending(&fixture.daemon).await;
        approve(&fixture.daemon, &id).await;
    });
    assert_eq!(code, SUCCESS, "{printed}");
    assert!(pending_requests(&fixture.daemon).await.is_empty());
}

/// Agents-14 step 16: the user's no ends the wait as denied.
#[tokio::test(flavor = "multi_thread")]
async fn a_wait_ends_denied_with_the_users_no() {
    let fixture = Fixture::start().await;
    let ((code, printed), ()) = tokio::join!(fixture.ask(&["--wait", "120"]), async {
        let id = first_pending(&fixture.daemon).await;
        fixture.deny(&id).await;
    });
    assert_eq!(code, DENIED, "{printed}");
    assert_eq!(printed["error"]["kind"], "denied", "{printed}");
}

/// Agents-14 step 15: ⌃C stops waiting with 130, and the request stays in
/// the Inbox for the user.
#[tokio::test(flavor = "multi_thread")]
async fn ctrl_c_stops_waiting_and_the_request_stays() {
    let fixture = Fixture::start().await;
    let child = tokio::process::Command::new(binary("HUMATA"))
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", fixture.daemon.dir.path())
        .env("HUMATA_SOCKET", &fixture.daemon.socket)
        .env("HUMATA_SESSION_TOKEN", &fixture.token)
        .args(["--json", "--wait", "120"])
        .args(ASK)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    // Once the request is in the Inbox the run is waiting on its answer.
    let id = first_pending(&fixture.daemon).await;
    let pid = child.id().expect("the run is still going").to_string();
    let sent = std::process::Command::new("/bin/kill")
        .args(["-INT", &pid])
        .status()
        .unwrap();
    assert!(sent.success());
    let ended = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .expect("⌃C ends the wait")
        .unwrap();
    assert_eq!(ended.status.code(), Some(INTERRUPTED));
    assert_eq!(pending_requests(&fixture.daemon).await, vec![id]);
}

/// Agents-14, a session's terminal: the agent can't answer its own request
/// from the CLI, and the request stays for the user.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_cannot_approve_its_own_request_from_the_cli() {
    let fixture = Fixture::start().await;
    let (code, _) = fixture.ask(&[]).await;
    assert_eq!(code, PENDING);
    let id = first_pending(&fixture.daemon).await;
    let (code, printed) =
        humata_as_agent(&fixture.daemon, &fixture.token, &["inbox", "approve", &id.value]).await;
    assert_ne!(code, SUCCESS, "{printed}");
    assert_eq!(printed["ok"], false, "{printed}");
    assert_eq!(pending_requests(&fixture.daemon).await, vec![id]);
}
