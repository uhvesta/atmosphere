//! The host's answers reach the machine that holds the request (AWAY T7b,
//! AW-6, D226). The host relays its user's answer down the machine's link;
//! the machine applies it in its own Inbox as the host's user, first answer
//! wins, and the result comes back as an `inbox.answer` journal entry. A
//! real role-machine daemon and a real `serve --stdio` link; the test plays
//! the host.

use humatad_test_common as common;
use humatad_test_common::away;

use away::{Host, OK, PASSPHRASE, Terminal, host_reachable_becomes, verifier};
use common::{Attach, Build, Daemon, folder, open, state_folder};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use prost::Message;
use std::time::Duration;
use tonic::{Code, Request};
use tonic::metadata::MetadataValue;

const CHANNEL: &str = "ch_away_replay";
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
/// `ASKER` and the token that tab's shell was given, whose host went away
/// after sending the policy copy.
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
        self.ask_at("src").await
    }

    /// As [`Self::ask`], for `path` of `OTHER`'s clone.
    async fn ask_at(&self, path: &str) -> pb::RequestId {
        let mut request = Request::new(pb::RequestAccessRequest {
            capability: pb::Capability::FilesReadOtherSession as i32,
            target: Some(other_files(path)),
            reason: "to compare the schemas".to_owned(),
            ..Default::default()
        });
        request
            .metadata_mut()
            .insert(TOKEN_HEADER, MetadataValue::try_from(self.token.as_str()).unwrap());
        let asked = pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
            .request_access(request)
            .await
            .unwrap()
            .into_inner();
        asked.id.expect("the request waits in the machine's Inbox")
    }
}

fn session(id: &str) -> pb::SessionId {
    pb::SessionId { value: id.to_owned() }
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

/// The host's relay of its user's `answer` to request `id`.
fn relay(id: &pb::RequestId, answer: pb::request_answer::Answer) -> pb::host_to_machine::Message {
    pb::host_to_machine::Message::Answer(pb::InboxAnswerRelay {
        seq: 0,
        request: Some(id.clone()),
        answer: Some(pb::RequestAnswer { answer: Some(answer) }),
        by: Some(pb::Actor {
            actor: Some(pb::actor::Actor::User(pb::UserActor {
                via: pb::user_actor::Via::App as i32,
                machine: Some(pb::MachineId {
                    value: "mac".to_owned(),
                }),
            })),
        }),
        at: None,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn an_answer_relayed_by_the_host_settles_the_request_on_the_machine() {
    let machine = Machine::away().await;
    let id = machine.ask().await;

    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    let backlog = host.journal_until_policy_ack(2).await;
    assert_eq!(backlog.len(), 1);
    assert_eq!(backlog[0].operation, REQUEST);

    host.send(relay(&id, pb::request_answer::Answer::Approve(pb::Approve {})))
        .await;
    let answered = host.journal(1).await;
    assert_eq!(answered[0].operation, ANSWER);
    let settled = carried(&answered[0]);
    assert_eq!(settled.id.as_ref(), Some(&id));
    assert_eq!(settled.state(), pb::RequestState::Approved);
    assert!(settled.answered.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relayed_deny_settles_the_request_as_denied() {
    let machine = Machine::away().await;
    let id = machine.ask().await;

    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    host.journal_until_policy_ack(2).await;

    host.send(relay(
        &id,
        pb::request_answer::Answer::Deny(pb::Deny { note: String::new() }),
    ))
    .await;
    let answered = host.journal(1).await;
    assert_eq!(answered[0].operation, ANSWER);
    assert_eq!(carried(&answered[0]).state(), pb::RequestState::Denied);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_for_a_request_the_machine_already_answered_changes_nothing() {
    let machine = Machine::away().await;
    let first = machine.ask().await;

    // The user answers on the machine, with the passphrase, while the host
    // is away.
    let mut terminal = Terminal::open(&machine.daemon);
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    let approve = format!(r#""$HUMATA" --json inbox approve {} > /dev/null 2>&1"#, first.value);
    assert_eq!(terminal.exit_code(&approve, None).await, OK);

    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    let backlog = host.journal_until_policy_ack(2).await;
    let kinds: Vec<&str> = backlog.iter().map(|entry| entry.operation.as_str()).collect();
    assert_eq!(kinds, [REQUEST, ANSWER]);
    assert_eq!(carried(&backlog[1]).state(), pb::RequestState::Approved);

    // The host's answer came after: the first answer stands, and the
    // machine writes nothing for the second. The next entry it sends is the
    // next request, not a second answer.
    host.send(relay(
        &first,
        pb::request_answer::Answer::Deny(pb::Deny { note: String::new() }),
    ))
    .await;
    // Not `src`: the first answer's grant already covers it.
    let second = machine.ask_at("docs").await;
    let next = host.journal(1).await;
    assert_eq!(next[0].operation, REQUEST);
    assert_eq!(carried(&next[0]).id.as_ref(), Some(&second));
}

/// How long the machine may take to refuse a relayed answer: the host
/// itself waits 10 s for a verdict, and a refusal must beat that by far.
const REFUSAL_WITHIN: Duration = Duration::from_secs(3);

/// The refusal the machine sends next, skipping its other messages.
async fn refusal(host: &mut Host) -> pb::InboxAnswerRefused {
    tokio::time::timeout(REFUSAL_WITHIN, async {
        loop {
            if let pb::machine_to_host::Message::AnswerRefused(refused) = host.next().await {
                return refused;
            }
        }
    })
    .await
    .expect("the machine refuses the relayed answer at once, not after the host's wait")
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_for_a_request_the_machine_already_answered_comes_back_refused_saying_who_answered() {
    let machine = Machine::away().await;
    let first = machine.ask().await;
    let mut terminal = Terminal::open(&machine.daemon);
    assert_eq!(terminal.log_in(PASSPHRASE).await, OK);
    let approve = format!(r#""$HUMATA" --json inbox approve {} > /dev/null 2>&1"#, first.value);
    assert_eq!(terminal.exit_code(&approve, None).await, OK);

    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    host.journal_until_policy_ack(2).await;

    host.send(relay(
        &first,
        pb::request_answer::Answer::Deny(pb::Deny { note: String::new() }),
    ))
    .await;
    let refused = refusal(&mut host).await;
    assert_eq!(refused.request.as_ref(), Some(&first));
    assert_eq!(Code::from(refused.code), Code::Aborted);
    let Some(pb::error::Kind::AlreadyAnswered(already)) = refused.error.and_then(|error| error.kind) else {
        panic!("the refusal says the request was already answered");
    };
    assert!(already.approved, "the first answer, the approval, stands");
    assert!(already.answered.is_some(), "the refusal names who answered");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_relay_for_a_request_the_machine_does_not_hold_comes_back_refused_as_not_found() {
    let machine = Machine::away().await;
    let mut host = Host::link(&machine.daemon).await;
    host.resume(0).await;
    host.send_policy(2, policy()).await;
    host.journal_until_policy_ack(2).await;

    let unknown = pb::RequestId {
        value: "rq_not_here".to_owned(),
    };
    host.send(relay(&unknown, pb::request_answer::Answer::Approve(pb::Approve {})))
        .await;
    let refused = refusal(&mut host).await;
    assert_eq!(refused.request.as_ref(), Some(&unknown));
    assert_eq!(Code::from(refused.code), Code::NotFound);
    assert!(matches!(
        refused.error.and_then(|error| error.kind),
        Some(pb::error::Kind::NotFound(_))
    ));
}
