//! A channel's own Desk (CD-1..CD-3, CD-6, D306) through a real
//! `humatad2`: a channel Desk session starts in `<channel>/desk/<name>`
//! with no clones and stays out of the plain Desk; it is listed by its
//! channel and by its target; its agent works under the channel's rules,
//! with its switches allowing more on top: on the target and its forks
//! only, after Never, from the agent's next call and across a restart.
//! Channel actions on it are refused.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{
    Build, Daemon, World, create_channel, finished, folder, get_session, is_running, shell_token,
    start_session, wait_running, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use pb::grant_target::Target;
use tonic::Code;
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
    /// The session the Desk sessions look after, in Payments.
    target: pb::SessionId,
    /// Another session in Payments.
    other: pb::SessionId,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let args = world.args();
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &[]).await;
        world.remote("acme", "api");
        let (channel, _) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        let (target, _) = start_session(&daemon, &channel, "Ledger fix").await;
        let (other, _) = start_session(&daemon, &channel, "Docs pass").await;
        wait_running(&daemon, &target).await;
        wait_running(&daemon, &other).await;
        Self {
            daemon,
            world,
            channel,
            target,
            other,
        }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    async fn access(&self) -> pb::access_service_client::AccessServiceClient<Channel> {
        pb::access_service_client::AccessServiceClient::new(self.daemon.channel().await)
    }

    fn channel_id(&self) -> pb::ChannelId {
        self.channel.id.clone().unwrap()
    }

    /// Where Payments' Desk sessions work.
    fn desk_root(&self) -> PathBuf {
        self.world.channels.join(&self.channel.folder).join("desk")
    }

    fn request(
        &self,
        title: &str,
        target: Option<&pb::SessionId>,
        switches: Option<pb::ChannelDeskSwitches>,
    ) -> pb::StartDeskSessionRequest {
        pb::StartDeskSessionRequest {
            title: title.to_owned(),
            channel: Some(self.channel_id()),
            target: target.cloned(),
            switches,
            ..Default::default()
        }
    }

    /// Starts `request` and waits until its agent is up.
    async fn start(&self, request: pb::StartDeskSessionRequest) -> (pb::SessionId, pb::DeskSession) {
        let started = self
            .desk()
            .await
            .start_desk_session(request)
            .await
            .unwrap()
            .into_inner();
        let id = started.session.unwrap();
        wait_running(&self.daemon, &id).await;
        (id, started.desk.unwrap())
    }

    /// A Desk session of Payments looking after the target.
    async fn on_target(&self, switches: pb::ChannelDeskSwitches) -> pb::SessionId {
        self.start(self.request("Watch the ledger", Some(&self.target), Some(switches)))
            .await
            .0
    }

    async fn listed(&self, request: pb::ListDeskSessionsRequest) -> Vec<pb::DeskSession> {
        self.desk()
            .await
            .list_desk_sessions(request)
            .await
            .unwrap()
            .into_inner()
            .sessions
    }

    async fn by_channel(&self) -> Vec<pb::DeskSession> {
        self.listed(pb::ListDeskSessionsRequest {
            channel: Some(self.channel_id()),
            target: None,
        })
        .await
    }

    async fn find(&self, id: &pb::SessionId) -> pb::DeskSession {
        self.by_channel()
            .await
            .into_iter()
            .find(|desk| desk.session.as_ref() == Some(id))
            .expect("the channel lists its Desk session")
    }

    async fn set_switches(&self, id: &pb::SessionId, switches: pb::ChannelDeskSwitches) {
        self.desk()
            .await
            .set_desk_access(pb::SetDeskAccessRequest {
                session: Some(id.clone()),
                access: None,
                switches: Some(switches),
            })
            .await
            .unwrap();
    }

    async fn set_rule(&self, capability: pb::Capability, decision: pb::Decision) {
        self.access()
            .await
            .set_channel_rule(pb::SetChannelRuleRequest {
                channel: Some(self.channel_id()),
                capability: capability as i32,
                decision: decision as i32,
                ..Default::default()
            })
            .await
            .expect("the user sets the rule");
    }

    /// What the agent holding `token` would meet doing `capability` on
    /// `target` now.
    async fn explain(&self, token: &str, capability: pb::Capability, target: Target) -> pb::Decision {
        let request = as_agent(
            pb::ExplainRequest {
                session: None,
                capability: capability as i32,
                target: Some(pb::GrantTarget {
                    target: Some(target),
                }),
            },
            token,
        );
        self.access()
            .await
            .explain(request)
            .await
            .unwrap()
            .into_inner()
            .decision()
    }

    async fn note(&self, token: &str, to: &pb::SessionId) -> pb::Decision {
        self.explain(token, pb::Capability::SessionNote, Target::Session(to.clone()))
            .await
    }

    async fn read(&self, token: &str, of: &pb::SessionId) -> pb::Decision {
        self.explain(
            token,
            pb::Capability::FilesReadOtherSession,
            Target::Path(pb::PathScope {
                machine: None,
                session: Some(of.clone()),
                prefix: None,
            }),
        )
        .await
    }

    /// The user forks `parent` and waits for the fork to be made.
    async fn fork(&self, parent: &pb::SessionId) -> pb::SessionId {
        let operation = self
            .daemon
            .session_service()
            .await
            .fork_session(fork_request(parent))
            .await
            .unwrap()
            .into_inner();
        let done = finished(&self.daemon, &operation).await;
        match done.result {
            Some(pb::operation::Result::Session(fork)) => fork,
            other => panic!("the fork made no session: {other:?}"),
        }
    }
}

fn as_agent<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "humata-session-token",
        MetadataValue::try_from(token).unwrap(),
    );
    // Short enough that a call held for the Inbox comes back in the test's
    // time instead of after the agent's 40 seconds.
    request.set_timeout(Duration::from_secs(20));
    request
}

fn fork_request(parent: &pb::SessionId) -> pb::ForkSessionRequest {
    pb::ForkSessionRequest {
        parent: Some(parent.clone()),
        kind: pb::ForkKind::CodeOnly as i32,
        title: "Ledger side".to_owned(),
        ..Default::default()
    }
}

fn defaults() -> pb::ChannelDeskSwitches {
    pb::ChannelDeskSwitches {
        read_files: true,
        send_notes: true,
        ..Default::default()
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
}

/// CD-1, CD-6: the folder is `<channel>/desk/<name>`, with no clones; the
/// session is listed by its channel and is not on the plain Desk.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_desk_session_works_in_the_channels_desk_folder_and_stays_off_the_desk() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture
        .start(fixture.request("Triage the queue", None, None))
        .await;

    let path = PathBuf::from(desk.folder.unwrap().path);
    assert!(path.is_dir());
    assert!(same_dir(path.parent().unwrap(), &fixture.desk_root()));
    assert!(desk.repos.is_empty());
    let clones = std::fs::read_dir(&path)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().path().join(".git").exists())
        .count();
    assert_eq!(clones, 0, "a channel Desk folder has no clones");
    let part = desk.channel_desk.unwrap();
    assert_eq!(part.channel, Some(fixture.channel_id()));
    assert_eq!(part.target, None);
    // Unset switches are the New desk session dialog's defaults.
    assert_eq!(part.switches, Some(defaults()));

    let plain = fixture.listed(pb::ListDeskSessionsRequest::default()).await;
    assert!(plain.iter().all(|desk| desk.session.as_ref() != Some(&id)));
    let mine = fixture.by_channel().await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0].session.as_ref(), Some(&id));
}

/// CD-1: a target in another channel is refused before anything is made.
#[tokio::test(flavor = "multi_thread")]
async fn a_target_in_another_channel_is_refused_and_leaves_no_folder() {
    let fixture = Fixture::new().await;
    fixture.world.remote("acme", "web");
    let (search, _) = create_channel(&fixture.daemon, "Search", &[("acme", "web")]).await;
    let (elsewhere, _) = start_session(&fixture.daemon, &search, "Index tune").await;

    let refused = fixture
        .desk()
        .await
        .start_desk_session(fixture.request("Watch it", Some(&elsewhere), None))
        .await
        .unwrap_err();
    assert!(
        matches!(refused.code(), Code::FailedPrecondition | Code::InvalidArgument),
        "refused as the caller's mistake, not {refused:?}"
    );
    let left = std::fs::read_dir(fixture.desk_root())
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(left, 0, "nothing is left in the channel's desk folder");
    assert!(fixture.by_channel().await.is_empty());
}

/// CD-2: by channel lists all of the channel's Desk sessions; by target
/// only those looking after that session.
#[tokio::test(flavor = "multi_thread")]
async fn desk_sessions_are_listed_by_channel_and_by_target() {
    let fixture = Fixture::new().await;
    let (whole, _) = fixture
        .start(fixture.request("Plan the week", None, None))
        .await;
    let watching = fixture.on_target(defaults()).await;

    let by_channel: Vec<_> = fixture
        .by_channel()
        .await
        .into_iter()
        .map(|desk| desk.session.unwrap())
        .collect();
    assert_eq!(by_channel.len(), 2);
    assert!(by_channel.contains(&whole) && by_channel.contains(&watching));

    let by_target: Vec<_> = fixture
        .listed(pb::ListDeskSessionsRequest {
            channel: None,
            target: Some(fixture.target.clone()),
        })
        .await
        .into_iter()
        .map(|desk| desk.session.unwrap())
        .collect();
    assert_eq!(by_target, vec![watching]);
}

/// CD-3: with the default switches the agent may note the target without
/// asking; a note to another session of the channel is the channel rule's
/// (Ask); turning the switch off leaves the target to the rule too, from
/// the agent's next call.
#[tokio::test(flavor = "multi_thread")]
async fn the_note_switch_covers_the_target_only_and_applies_from_the_next_call() {
    let fixture = Fixture::new().await;
    let desk = fixture.on_target(defaults()).await;
    let (token, _attach) = shell_token(&fixture.daemon, &desk).await;

    assert_eq!(fixture.note(&token, &fixture.target).await, pb::Decision::Allow);
    assert_eq!(fixture.note(&token, &fixture.other).await, pb::Decision::Ask);

    fixture
        .set_switches(
            &desk,
            pb::ChannelDeskSwitches {
                send_notes: false,
                ..defaults()
            },
        )
        .await;
    assert_eq!(fixture.note(&token, &fixture.target).await, pb::Decision::Ask);
}

/// CD-3: a fork of the target made after the Desk session started is
/// covered from its first call.
#[tokio::test(flavor = "multi_thread")]
async fn a_fork_of_the_target_made_later_is_covered() {
    let fixture = Fixture::new().await;
    let desk = fixture.on_target(defaults()).await;
    let (token, _attach) = shell_token(&fixture.daemon, &desk).await;

    let fork = fixture.fork(&fixture.target).await;
    assert_eq!(fixture.note(&token, &fork).await, pb::Decision::Allow);
    assert_eq!(fixture.read(&token, &fork).await, pb::Decision::Allow);
}

/// CD-3: the read switch lets the agent read the target's files; another
/// session's are the rule's.
#[tokio::test(flavor = "multi_thread")]
async fn the_read_switch_reads_the_targets_files_only() {
    let fixture = Fixture::new().await;
    let desk = fixture.on_target(defaults()).await;
    let (token, _attach) = shell_token(&fixture.daemon, &desk).await;

    assert_eq!(fixture.read(&token, &fixture.target).await, pb::Decision::Allow);
    assert_eq!(fixture.read(&token, &fixture.other).await, pb::Decision::Ask);
}

/// CD-3: a rule set to Never refuses even with the switch on, and the
/// switch is shown as blocked by it.
#[tokio::test(flavor = "multi_thread")]
async fn a_never_rule_blocks_the_switch_and_shows_it_blocked() {
    let fixture = Fixture::new().await;
    let desk = fixture.on_target(defaults()).await;
    let (token, _attach) = shell_token(&fixture.daemon, &desk).await;

    fixture
        .set_rule(pb::Capability::SessionNote, pb::Decision::Never)
        .await;
    assert_eq!(fixture.note(&token, &fixture.target).await, pb::Decision::Never);
    let blocked = fixture.find(&desk).await.channel_desk.unwrap().blocked;
    assert!(blocked.contains(&(pb::ChannelDeskSwitch::SendNotes as i32)));
    assert!(!blocked.contains(&(pb::ChannelDeskSwitch::ReadFiles as i32)));
}

/// CD-3: with "Stop and archive sessions" on, the agent stops its target
/// without asking. The rule is set to Ask here so the test holds whatever
/// the channel's default for stopping another session is.
#[tokio::test(flavor = "multi_thread")]
async fn the_stop_switch_stops_the_target() {
    let fixture = Fixture::new().await;
    fixture
        .set_rule(pb::Capability::SessionStopOther, pb::Decision::Ask)
        .await;
    let desk = fixture
        .on_target(pb::ChannelDeskSwitches {
            stop_and_archive: true,
            ..defaults()
        })
        .await;
    let (token, _attach) = shell_token(&fixture.daemon, &desk).await;

    fixture
        .daemon
        .session_service()
        .await
        .stop_session(as_agent(
            pb::StopSessionRequest {
                idempotency_key: None,
                id: Some(fixture.target.clone()),
            },
            &token,
        ))
        .await
        .expect("the switch lets the agent stop its target");
    // A stopped agent reads as stopped (D784).
    wait_status(&fixture.daemon, &fixture.target, pb::SessionStatus::Stopped).await;
    // The other session is still the rule's: its agent still runs.
    let other = get_session(&fixture.daemon, &fixture.other).await;
    assert!(is_running(&other), "{other:?}");
}

/// CD-3: with a session target, "Start and fork sessions" forks the
/// target without asking but leaves a fresh start to the rule (Ask).
#[tokio::test(flavor = "multi_thread")]
async fn the_start_switch_forks_the_target_but_a_fresh_start_asks() {
    let fixture = Fixture::new().await;
    let desk = fixture
        .on_target(pb::ChannelDeskSwitches {
            start_and_fork: true,
            ..defaults()
        })
        .await;
    let (token, _attach) = shell_token(&fixture.daemon, &desk).await;

    let fresh = fixture
        .explain(
            &token,
            pb::Capability::SessionStartSameMachine,
            Target::AgentOnMachine(pb::AgentOnMachine {
                machine: None,
                agent: Some(pb::AgentId {
                    value: "zsh".to_owned(),
                }),
            }),
        )
        .await;
    assert_eq!(fresh, pb::Decision::Ask);

    let operation = fixture
        .daemon
        .session_service()
        .await
        .fork_session(as_agent(fork_request(&fixture.target), &token))
        .await
        .expect("the switch lets the agent fork its target")
        .into_inner();
    let done = finished(&fixture.daemon, &operation).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32);
}

/// CD-3: with the whole channel as the target, the board switch lets the
/// agent change anyone's items on the channel board; off, it is the rule's.
#[tokio::test(flavor = "multi_thread")]
async fn the_board_switch_covers_the_channel_board_for_a_channel_target() {
    let fixture = Fixture::new().await;
    let (desk, _) = fixture
        .start(fixture.request(
            "Groom the board",
            None,
            Some(pb::ChannelDeskSwitches {
                change_board_items: true,
                ..defaults()
            }),
        ))
        .await;
    let (token, _attach) = shell_token(&fixture.daemon, &desk).await;
    let board = || {
        Target::Board(pb::BoardRef {
            board: Some(pb::board_ref::Board::Channel(fixture.channel_id())),
        })
    };

    let on = fixture
        .explain(&token, pb::Capability::ChannelBoardEditOthers, board())
        .await;
    assert_eq!(on, pb::Decision::Allow);
    fixture.set_switches(&desk, defaults()).await;
    let off = fixture
        .explain(&token, pb::Capability::ChannelBoardEditOthers, board())
        .await;
    assert_eq!(off, pb::Decision::Ask);
}

/// CD-3, persistence: the switches the user set are there after a restart,
/// and still decide the agent's calls.
#[tokio::test(flavor = "multi_thread")]
async fn the_switches_survive_a_restart() {
    let mut fixture = Fixture::new().await;
    let desk = fixture.on_target(defaults()).await;
    let changed = pb::ChannelDeskSwitches {
        send_notes: false,
        type_into_terminal: true,
        ..defaults()
    };
    fixture.set_switches(&desk, changed.clone()).await;

    fixture.daemon.restart().await;
    let part = fixture.find(&desk).await.channel_desk.unwrap();
    assert_eq!(part.switches, Some(changed));
    assert_eq!(part.target, Some(fixture.target.clone()));
    let (token, _attach) = shell_token(&fixture.daemon, &desk).await;
    assert_eq!(fixture.note(&token, &fixture.target).await, pb::Decision::Ask);
}

/// CD-1: a channel Desk session has no fork and no archive of its own: the
/// channel actions are refused on it, as on a plain Desk session.
#[tokio::test(flavor = "multi_thread")]
async fn channel_actions_are_refused_on_a_channel_desk_session() {
    let fixture = Fixture::new().await;
    let (desk, _) = fixture
        .start(fixture.request("Scratch notes", None, None))
        .await;
    let mut sessions = fixture.daemon.session_service().await;

    let fork = sessions
        .fork_session(fork_request(&desk))
        .await
        .unwrap_err();
    assert_eq!(fork.code(), Code::FailedPrecondition);
    let archive = sessions
        .archive_session(pb::ArchiveSessionRequest {
            id: Some(desk.clone()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(archive.code(), Code::FailedPrecondition);
}

/// CD-4: the listing says how many lines each Markdown file in a channel
/// Desk folder has, so the Close confirmation can show them.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_desk_session_lists_the_line_count_of_each_markdown_file() {
    let fixture = Fixture::new().await;
    let (id, desk) = fixture
        .start(fixture.request("Draft the runbook", None, None))
        .await;
    let folder = PathBuf::from(desk.folder.unwrap().path);
    std::fs::write(folder.join("a.md"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(folder.join("notes.txt"), "not markdown\n").unwrap();

    let listed = fixture.find(&id).await;
    assert_eq!(listed.documents, vec!["a.md".to_owned()]);
    let expected: std::collections::HashMap<String, u32> =
        [("a.md".to_owned(), 3)].into_iter().collect();
    assert_eq!(listed.document_lines, expected);
}
