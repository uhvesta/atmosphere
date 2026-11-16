//! A role-machine daemon with two sessions of one channel, a shell tab's
//! session token for the first, and a clone folder, for the AW-7 suites.
//! The machine's host is either away or linked (the test's `Host`), and its
//! channel's rules come from a policy copy the test sends.
//!
//! Use it as
//! `mod common; #[path = "common/away.rs"] mod away;
//!  #[path = "common/away_machine.rs"] mod away_machine;`.

#![allow(dead_code)]

use std::path::PathBuf;

use humata2_grpc::humata::v1 as pb;
use humata2_store::{SessionRecord, Store};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Request, Response, Status};

use crate::away::{self, Host, host_reachable_becomes, verifier};
use crate::common::{self, Attach, Build, Daemon, folder, open, state_folder};

pub const CHANNEL: &str = "ch_aw7";
/// The session whose tab's agent calls.
pub const ASKER: &str = "se_asker";
/// Another session of the same channel, on the same machine.
pub const OTHER: &str = "se_other";
/// The repo both sessions have a clone of, under the same folder.
pub const REPO: &str = "ledger";
const TOKEN_HEADER: &str = "humata-session-token";

/// The journal's operation names for the Inbox (GRPC.md §9.7).
pub const REQUEST: &str = "inbox.request";
pub const ANSWER: &str = "inbox.answer";

/// The policy copy: `channel`'s rules are `rules`, its While I'm away rule
/// is `mode`, and the approval passphrase's hash is held.
pub fn policy(mode: pb::AwayMode, rules: &[(pb::Capability, pb::Decision)]) -> pb::PolicySnapshot {
    let away = pb::AwayRule {
        mode: mode as i32,
        low_risk: Vec::new(),
    };
    pb::PolicySnapshot {
        channels: vec![pb::ChannelRules {
            channel: Some(pb::ChannelId {
                value: CHANNEL.to_owned(),
            }),
            rules: rules
                .iter()
                .map(|(capability, decision)| pb::Rule {
                    capability: *capability as i32,
                    decision: *decision as i32,
                    grants: Vec::new(),
                })
                .collect(),
            away: Some(away.clone()),
            revision: None,
            copy: None,
        }],
        defaults: Some(pb::DefaultRules {
            rules: Vec::new(),
            away: Some(away),
            revision: None,
            copy: None,
        }),
        approval_verifier: verifier(),
        ..Default::default()
    }
}

/// Where the machine's host is once the machine is up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Link {
    /// Linked once, to give the machine its policy copy, then gone.
    Away,
    /// Linked and kept linked.
    Reachable,
}

pub struct AwayMachine {
    pub daemon: Daemon,
    /// The token the first session's shell tab was given: an agent's identity.
    pub token: String,
    /// The clone of [`REPO`] both sessions share, on disk.
    pub clone: PathBuf,
    /// The host's end of the link, while the machine's host is [`Link::Reachable`].
    pub host: Option<Host>,
    snapshot: pb::PolicySnapshot,
    _attach: Attach,
}

impl AwayMachine {
    /// A machine that holds `snapshot` as its policy copy, in `link`.
    pub async fn start(snapshot: pb::PolicySnapshot, link: Link) -> Self {
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
        let clone = dir.path().join(REPO);
        std::fs::create_dir(&clone).unwrap();
        // The admin build lets the test stand in for the tab's agent.
        let daemon = away::machine_in(Build::Admin, dir).await;
        let mut host = Host::link(&daemon).await;
        host.resume(0).await;
        host.send_policy(1, snapshot.clone()).await;
        assert!(host.journal_until_policy_ack(1).await.is_empty());
        let host = match link {
            Link::Reachable => {
                host_reachable_becomes(&daemon, true).await;
                Some(host)
            }
            Link::Away => {
                drop(host);
                host_reachable_becomes(&daemon, false).await;
                None
            }
        };

        let tab = daemon
            .terminals()
            .await
            .open_shell_tab(pb::OpenShellTabRequest {
                idempotency_key: None,
                session: Some(session(ASKER)),
                repo: None,
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
            clone,
            host,
            snapshot,
            _attach: attach,
        }
    }

    /// `message` as the first session's agent sends it.
    pub fn as_agent<T>(&self, message: T) -> Request<T> {
        let mut request = Request::new(message);
        request.metadata_mut().insert(
            TOKEN_HEADER,
            MetadataValue::try_from(self.token.as_str()).unwrap(),
        );
        request
    }

    pub async fn inbox(&self) -> pb::inbox_service_client::InboxServiceClient<Channel> {
        pb::inbox_service_client::InboxServiceClient::new(self.daemon.channel().await)
    }

    pub async fn files(&self) -> pb::file_service_client::FileServiceClient<Channel> {
        pb::file_service_client::FileServiceClient::new(self.daemon.channel().await)
    }

    pub async fn access(&self) -> pb::access_service_client::AccessServiceClient<Channel> {
        pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await)
    }

    /// The agent asks ahead for `capability` on `target`.
    pub async fn ask_ahead(
        &self,
        capability: pb::Capability,
        target: pb::GrantTarget,
    ) -> Result<pb::WaitResponse, Status> {
        self.inbox()
            .await
            .request_access(self.as_agent(pb::RequestAccessRequest {
                capability: capability as i32,
                target: Some(target),
                reason: "AW-7".to_owned(),
                ..Default::default()
            }))
            .await
            .map(Response::into_inner)
    }

    /// What the machine journaled, as a host that links now is sent it: the
    /// entries before it acks policy revision `revision`. Only for a machine
    /// whose host is away; the host that took them leaves again.
    pub async fn journal(&self, revision: u64) -> Vec<pb::JournalEntry> {
        assert!(self.host.is_none(), "a linked host reads the journal as it is sent");
        let mut host = Host::link(&self.daemon).await;
        host.resume(0).await;
        host.send_policy(revision, self.snapshot.clone()).await;
        let entries = host.journal_until_policy_ack(revision).await;
        drop(host);
        host_reachable_becomes(&self.daemon, false).await;
        entries
    }
}

pub fn session(id: &str) -> pb::SessionId {
    pb::SessionId {
        value: id.to_owned(),
    }
}

fn repo() -> pb::RepoRef {
    pb::RepoRef {
        owner: "acme".to_owned(),
        name: REPO.to_owned(),
    }
}

/// `path` in `owner`'s clone of [`REPO`].
pub fn files_of(owner: &str, path: &str) -> pb::FileLocation {
    pb::FileLocation {
        machine: None,
        root: Some(pb::file_location::Root::SessionClone(pb::SessionClone {
            session: Some(session(owner)),
            repo: Some(repo()),
        })),
        path: path.to_owned(),
    }
}

/// Reading `path` of [`OTHER`]'s clone, as an ask-ahead names it.
pub fn other_files(path: &str) -> pb::GrantTarget {
    pb::GrantTarget {
        target: Some(pb::grant_target::Target::Path(pb::PathScope {
            machine: None,
            session: Some(session(OTHER)),
            prefix: Some(pb::RepoPath {
                repo: Some(repo()),
                path: path.to_owned(),
            }),
        })),
    }
}

/// Typing into [`OTHER`].
pub fn other_session() -> pb::GrantTarget {
    pb::GrantTarget {
        target: Some(pb::grant_target::Target::Session(session(OTHER))),
    }
}

/// Pushing [`REPO`]'s main: something that needs GitHub.
pub fn push_main() -> pb::GrantTarget {
    pb::GrantTarget {
        target: Some(pb::grant_target::Target::RepoBranch(pb::RepoBranch {
            repo: Some(repo()),
            branch: "main".to_owned(),
        })),
    }
}

/// The typed kind a failed call carries.
pub fn kind(status: &Status) -> Option<pb::error::Kind> {
    common::error_of(status).kind
}

/// The Inbox request an `inbox.*` entry carries. A board write's entry
/// carries the call that made it: see [`board_request`].
pub fn carried(entry: &pb::JournalEntry) -> pb::InboxRequest {
    use prost::Message;
    assert!(
        entry.operation.starts_with("inbox."),
        "`{}` is not an Inbox entry, so it carries no InboxRequest",
        entry.operation
    );
    pb::InboxRequest::decode(entry.request.as_slice()).expect("the entry carries an InboxRequest")
}

/// The call a `board.*` entry carries, as the agent made it.
pub fn board_request<T: prost::Message + Default>(entry: &pb::JournalEntry) -> T {
    assert!(
        entry.operation.starts_with("board."),
        "`{}` is not a board entry",
        entry.operation
    );
    T::decode(entry.request.as_slice()).expect("the entry carries the request of the board call")
}

/// The entries whose operation starts with `prefix`: `inbox.` or `board.`.
pub fn only(entries: Vec<pb::JournalEntry>, prefix: &str) -> Vec<pb::JournalEntry> {
    entries
        .into_iter()
        .filter(|entry| entry.operation.starts_with(prefix))
        .collect()
}

/// The policy copy [`policy`] makes, listing `owned` as the
/// board items this machine's sessions may change while the host is away.
pub fn policy_owning(
    mode: pb::AwayMode,
    rules: &[(pb::Capability, pb::Decision)],
    owned: Vec<pb::OwnedItem>,
) -> pb::PolicySnapshot {
    let mut snapshot = policy(mode, rules);
    snapshot.owned_items = owned;
    snapshot
}

/// `item`, on `board`, owned by `owner`, at `revision`.
pub fn owned(item: &str, owner: &str, board: pb::BoardRef, revision: u64) -> pb::OwnedItem {
    pb::OwnedItem {
        item: Some(pb::BoardItemId {
            value: item.to_owned(),
        }),
        owner: Some(session(owner)),
        revision: Some(pb::Revision { value: revision }),
        board: Some(board),
    }
}
