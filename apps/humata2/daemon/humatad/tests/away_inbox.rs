//! A machine's journal of its Inbox (AWAY T5, SL-5, AW-3, AW-5, AW-6,
//! GRPC.md §9.7, D225). An agent's request made on a machine while its
//! host is away waits in the machine's Inbox. The user answers it there
//! with a passphrase login. When the host links again, the machine sends
//! both, in order, as journal entries; the host's ack ends a resend. A
//! real role-machine daemon and a real `serve --stdio` link; the test plays
//! the host.

mod common;
#[path = "common/away.rs"]
mod away;

use away::{Host, OK, PASSPHRASE, Terminal, host_reachable_becomes, verifier};
use common::{Attach, Build, Daemon, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use prost::Message;
use tonic::Request;
use tonic::metadata::MetadataValue;

const CHANNEL: &str = "ch_away_inbox";
/// The session whose tab's agent asks.
const ASKER: &str = "se_asker";
/// Another session in the same channel, whose files the asker wants.
const OTHER: &str = "se_other";
const TOKEN_HEADER: &str = "humata-session-token";

const REQUEST: &str = "inbox.request";
const ANSWER: &str = "inbox.answer";

/// The policy copy: the channel's While I'm away rule is Wait, and the
/// passphrase's hash is held.
fn policy() -> pb::PolicySnapshot {
    let wait = pb::AwayRule {
        mode: pb::AwayMode::Wait as i32,
        low_risk: Vec::new(),
    };
    pb::PolicySnapshot {
        channels: vec![pb::ChannelRules {
            channel: Some(pb::ChannelId {
                value: CHANNEL.to_owned(),
            }),
            rules: Vec::new(),
            away: Some(wait.clone()),
            revision: None,
            copy: None,
        }],
        defaults: Some(pb::DefaultRules {
            rules: Vec::new(),
            away: Some(wait),
            revision: None,
            copy: None,
        }),
        approval_verifier: verifier(),
        ..Default::default()
    }
}

/// A machine that knows `ASKER` and `OTHER`, with a shell tab open in
/// `ASKER` and the token that tab's shell was given, which got the policy
/// copy from its host and whose host then went away.
struct Machine {
    daemon: Daemon,
    token: String,
    _attach: Attach,
}

impl Machine {
    async fn away() -> Self {
        let dir = folder();
        let store = Store::open(state_folder(dir.path()).join("store")).unwrap();
        for (id, title) in [(ASKER, "asker"), (OTHER, "other")] {
            store
                .put_session(&SessionRecord {
                    id: id.to_owned(),
                    channel: CHANNEL.to_owned(),
                    machine: String::new(),
                    title: title.to_owned(),
                    agent: "zsh".to_owned(),
                    folder: dir.path().to_owned(),
                    created_unix_ms: 0,
                })
                .unwrap();
        }
        drop(store);
        // The admin build lets the test stand in for the tab's agent; the
        // user's answer still needs the passphrase login (AW-5).
        let daemon = away::machine_in(Build::Admin, dir).await;
        let mut host = Host::link(&daemon).await;
        host.resume(0).await;
        host.send_policy(1, policy()).await;
        assert!(host.journal_until_policy_ack(1).await.is_empty());
        drop(host);
        host_reachable_becomes(&daemon, false).await;

        let tab = daemon
            .terminals()
            .await
            .open_shell_tab(pb::OpenShellTabRequest {
                idempotency_key: None,
                session: Some(session(ASKER)),
                repo: None,
                folder: String::new(),
            })
            .await
            .unwrap()
            .into_inner()
            .id
            .unwrap();
        let mut attach = Attach::start(&daemon, open(&tab, 0)).await;
        let file = daemon.dir.path().join("token");
        // The shell prints the marker once the token is in the file, so
        // the file is read only after the screen shows it.
        attach
            .type_text(&format!(
                "printf '%s' \"$HUMATA_SESSION_TOKEN\" > {}; echo tok''en-written\r",
                file.display()
            ))
            .await;
        attach.wait_for("token-written").await;
        let token = std::fs::read_to_string(&file).expect("the session token was kept");
        Self {
            daemon,
            token,
            _attach: attach,
        }
    }

    /// The agent asks ahead to read `OTHER`'s files; the request waits.
    async fn ask(&self) -> pb::RequestId {
        let mut request = Request::new(pb::RequestAccessRequest {
            capability: pb::Capability::FilesReadOtherSession as i32,
            target: Some(other_files("src")),
            reason: "to compare the schemas".to_owned(),
            ..Default::default()
        });
        request.metadata_mut().insert(
            TOKEN_HEADER,
            MetadataValue::try_from(self.token.as_str()).unwrap(),
        );
        let asked = pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .request_access(request)
            .await
            .unwrap()
            .into_inner();
        asked.id.expect("the request waits in the machine's Inbox")
    }
}

fn session(id: &str) -> pb::SessionId {
    pb::SessionId {
        value: id.to_owned(),
    }
}

/// `path` in the acme/ledger repo of `OTHER`'s clone.
fn other_files(path: &str) -> pb::GrantTarget {
    pb::GrantTarget {
        target: Some(pb::grant_target::Target::Path(pb::PathScope {
            machine: None,
            session: Some(session(OTHER)),
            prefix: Some(pb::RepoPath {
                repo: Some(pb::RepoRef {
                    owner: "acme".to_owned(),
                    name: "ledger".to_owned(),
                }),
                path: path.to_owned(),
            }),
        })),
    }
}

/// The request an entry carries.
fn carried(entry: &pb::JournalEntry) -> pb::InboxRequest {
    pb::InboxRequest::decode(entry.request.as_slice()).expect("the entry carries an InboxRequest")
}

fn asker_of(entry: &pb::JournalEntry) -> Option<&str> {
    entry
        .principal
        .as_ref()
        .and_then(|principal| principal.session.as_ref())
        .map(|session| session.value.as_str())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_answered_on_the_machine_while_away_reaches_the_host_in_order() {
    let machine = Machine::away().await;
    let id = machine.ask().await;

    let mut terminal = Terminal::open(&machine.daemon);
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    let approve = format!(r#""$HUMATA" --json inbox approve {} > /dev/null 2>&1"#, id.value);
    assert_eq!(terminal.exit_code(&approve, None).await, OK);

    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    let entries = host.journal_until_policy_ack(2).await;
    let kinds: Vec<&str> = entries.iter().map(|entry| entry.operation.as_str()).collect();
    assert_eq!(kinds, [REQUEST, ANSWER]);
    assert!(entries[0].seq < entries[1].seq);
    for entry in &entries {
        assert_eq!(carried(entry).id.as_ref(), Some(&id));
        assert_eq!(asker_of(entry), Some(ASKER));
        assert_eq!(entry.policy_revision, 1, "judged under the copy it held");
    }
    assert_eq!(carried(&entries[0]).state(), pb::RequestState::Pending);
    let answered = carried(&entries[1]);
    assert_eq!(answered.state(), pb::RequestState::Approved);

    // Acked: the next link sends nothing again.
    host.ack_journal(entries[1].seq).await;
    host.send_policy(3, policy()).await;
    assert!(host.journal_until_policy_ack(3).await.is_empty());
    drop(host);
    host_reachable_becomes(&machine.daemon, false).await;
    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(4, policy()).await;
    assert!(host.journal_until_policy_ack(4).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_still_pending_is_sent_and_new_ones_follow_while_linked() {
    let machine = Machine::away().await;
    let first = machine.ask().await;

    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    let backlog = host.journal_until_policy_ack(2).await;
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0].operation, REQUEST);
    assert_eq!(carried(&backlog[0]).id.as_ref(), Some(&first));
    assert_eq!(carried(&backlog[0]).state(), pb::RequestState::Pending);

    // While linked, a new request is journaled and sent as it is made.
    let second = machine.ask().await;
    let live = host.journal(1).await;
    assert_eq!(live[0].operation, REQUEST);
    assert_eq!(carried(&live[0]).id.as_ref(), Some(&second));
    assert!(live[0].seq > backlog[0].seq);
}

#[tokio::test(flavor = "multi_thread")]
async fn entries_the_host_never_acked_are_sent_again_after_a_restart() {
    let mut machine = Machine::away().await;
    let id = machine.ask().await;

    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    assert_eq!(host.journal_until_policy_ack(2).await.len(), 1);
    drop(host);

    machine.daemon.restart().await;
    host_reachable_becomes(&machine.daemon, false).await;
    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(3, policy()).await;
    let again = host.journal_until_policy_ack(3).await;
    assert_eq!(again.len(), 1);
    assert_eq!(carried(&again[0]).id.as_ref(), Some(&id));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hosts_resume_skips_what_it_already_holds() {
    let machine = Machine::away().await;
    machine.ask().await;
    let second = machine.ask().await;

    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    let both = host.journal_until_policy_ack(2).await;
    assert_eq!(both.len(), 2);
    drop(host);
    host_reachable_becomes(&machine.daemon, false).await;

    // The host applied the first before the link dropped, but its ack was
    // lost: its Resume says so.
    let mut host = Host::link(&machine.daemon).await;
    host.resume(both[0].seq).await;
    host.send_policy(3, policy()).await;
    let rest = host.journal_until_policy_ack(3).await;
    assert_eq!(rest.len(), 1);
    assert_eq!(carried(&rest[0]).id.as_ref(), Some(&second));
}
