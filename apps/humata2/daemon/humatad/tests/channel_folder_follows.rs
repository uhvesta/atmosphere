//! A channel's folder follows the channel (MV-16, D314) through a real
//! `humatad2`: a rename moves `<root>/<old>/` to `<root>/<new>/` once and
//! every session of the channel, its channel Desk sessions included,
//! resumes in its new folder with its conversation; a rename that would cut
//! off a running turn asks first; a rename onto a folder that is there is
//! refused and changes nothing; a channel restored from the trash to
//! `<old>-2` brings its sessions and its Desk sessions back under
//! `<old>-2/`; and a Desk session restored after its channel moved lands in
//! the channel's Desk folder as it is now. After each, nothing in the
//! store, the folders or the conversations names the old place.
//!
//! The agent keeps each conversation at `<conv>/<folder, dashed>/<id>.jsonl`
//! as Claude does, so a session whose folder moved resumes only if its
//! conversation moved with it. Everything runs under a HOME of the test's
//! own.

mod common;

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, World, agent_tab, binary, create_channel, error_of, finished, folder,
    get_session, is_running, open, shell_token, start_session, wait_running, wait_status,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use tonic::Code;
use tonic::transport::Channel;

/// The test agent: it starts or resumes the conversation kept by folder,
/// writes the folder it was given as the conversation's first line, as
/// Claude writes its `cwd`, and logs every start to `<root>/conv.log` as
/// `mode, conversation, key, folder, physical working folder`. A resume
/// whose conversation isn't at its key fails, as Claude's does.
const AGENT: &str = r#"CONV_DIR='@CONV@'
mode=$1; id=$2; key=$3; place=$4
f="$CONV_DIR/$key/$id.jsonl"
printf '%s\t%s\t%s\t%s\t%s\n' "$mode" "$id" "$key" "$place" "$(pwd -P)" >> "$CONV_DIR.log"
case "$mode" in
  --session-id) mkdir -p "$CONV_DIR/$key"; printf '{"cwd":"%s"}\n' "$place" > "$f" ;;
  --resume) [ -f "$f" ] || { echo "no conversation at $f"; exit 3; } ;;
esac
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; printf '%s\n' "$line" >> "$f"; done
"#;

/// One start of the agent, as it logged it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Start {
    mode: String,
    key: String,
    place: PathBuf,
    cwd: PathBuf,
}

/// How an agent that keys its conversations by folder names that folder.
fn dashed(path: &Path) -> String {
    path.display()
        .to_string()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Makes the world's agent the one above; `turn_end` as Settings › Agents
/// names it, or the default (output).
fn keep_conversations_by_folder(world: &World, turn_end: Option<&str>) {
    let script = world.root.join("agent.sh");
    std::fs::write(&script, AGENT.replace("@CONV@", &world.conv.display().to_string())).unwrap();
    let mut agent = serde_json::json!({
        "name": common::AGENT_NAME,
        "program": "/bin/sh",
        "start_args": [script, "--session-id", "{conversation}", "{folder_dashed}", "{folder}"],
        "resume_args": [script, "--resume", "{conversation}", "{folder_dashed}", "{folder}"],
        "conversation_file": format!("{}/{{folder_dashed}}/{{conversation}}.jsonl", world.conv.display()),
    });
    if let Some(turn_end) = turn_end {
        agent["turn_end"] = turn_end.into();
    }
    std::fs::write(&world.agents, serde_json::json!([agent]).to_string()).unwrap();
}

/// The variables that keep every agent's own files inside `home`.
fn home_env(home: &Path) -> Vec<(String, String)> {
    let at = |rest: &str| home.join(rest).display().to_string();
    let env = vec![
        ("HOME".to_owned(), home.display().to_string()),
        ("CLAUDE_CONFIG_DIR".to_owned(), at(".claude")),
        ("CODEX_HOME".to_owned(), at(".codex")),
        ("COPILOT_HOME".to_owned(), at(".copilot")),
        ("COPILOT_CACHE_HOME".to_owned(), at(".cache/copilot")),
        ("XDG_CONFIG_HOME".to_owned(), at(".config")),
        ("XDG_DATA_HOME".to_owned(), at(".local/share")),
        ("XDG_STATE_HOME".to_owned(), at(".local/state")),
        ("XDG_CACHE_HOME".to_owned(), at(".cache")),
    ];
    for (_, path) in &env {
        std::fs::create_dir_all(path).unwrap();
    }
    env
}

/// A session of the test: its id, its conversation and its folder when it
/// started.
struct Started {
    id: pb::SessionId,
    conversation: String,
    folder: PathBuf,
}

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
}

impl Fixture {
    /// A daemon whose agent keeps its conversations by folder, and channel
    /// Payments on acme/api.
    async fn new() -> Self {
        Self::with_turn_end(None).await
    }

    async fn with_turn_end(turn_end: Option<&str>) -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        keep_conversations_by_folder(&world, turn_end);
        world.remote("acme", "api");
        let env = home_env(&dir.path().join("home"));
        let env: Vec<(&str, &str)> = env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        let mut args = world.args();
        args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &refs, &env).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        Self { daemon, world, channel }
    }

    fn channel_id(&self) -> pb::ChannelId {
        self.channel.id.clone().unwrap()
    }

    /// The channel's folder when the test began.
    fn old(&self) -> PathBuf {
        self.world.channels.join(&self.channel.folder)
    }

    fn store(&self) -> Store {
        Store::open(self.daemon.state.join("store")).unwrap()
    }

    /// The folder the store holds for `id`.
    fn folder_of(&self, id: &pb::SessionId) -> PathBuf {
        self.store().session(&id.value).unwrap().expect("the store has the session").folder
    }

    fn launch_of(&self, id: &pb::SessionId) -> humata2_store::AgentLaunch {
        self.store().agent(&id.value).unwrap().expect("the store has the session's agent").launch
    }

    async fn channel_now(&self) -> pb::Channel {
        self.daemon
            .channel_service()
            .await
            .get_channel(pb::GetChannelRequest { id: Some(self.channel_id()) })
            .await
            .unwrap()
            .into_inner()
    }

    async fn rename(&self, name: &str, confirm_running: bool) -> Result<pb::Channel, tonic::Status> {
        self.daemon
            .channel_service()
            .await
            .rename_channel(pb::RenameChannelRequest {
                idempotency_key: None,
                id: Some(self.channel_id()),
                name: name.to_owned(),
                expected_revision: None,
                confirm_running,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Starts a channel session and waits until its agent runs.
    async fn session(&self, title: &str) -> Started {
        let (id, operation) = start_session(&self.daemon, &self.channel, title).await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        self.started(id).await
    }

    async fn started(&self, id: pb::SessionId) -> Started {
        let session = wait_running(&self.daemon, &id).await;
        let folder = self.folder_of(&id);
        Started {
            id,
            conversation: session.conversation_id,
            folder,
        }
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    async fn trash(&self) -> pb::trash_service_client::TrashServiceClient<Channel> {
        pb::trash_service_client::TrashServiceClient::new(self.daemon.channel().await)
    }

    /// Starts a Desk session of the whole channel and waits until its agent
    /// runs.
    async fn desk_session(&self, title: &str) -> Started {
        let started = self
            .desk()
            .await
            .start_desk_session(pb::StartDeskSessionRequest {
                title: title.to_owned(),
                channel: Some(self.channel_id()),
                target: None,
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        self.started(started.session.unwrap()).await
    }

    /// The folder the channel's Desk list shows for `id`, if it lists it.
    async fn desk_folder_listed(&self, id: &pb::SessionId) -> Option<PathBuf> {
        self.desk()
            .await
            .list_desk_sessions(pb::ListDeskSessionsRequest {
                channel: Some(self.channel_id()),
                target: None,
            })
            .await
            .unwrap()
            .into_inner()
            .sessions
            .into_iter()
            .find(|desk| desk.session.as_ref() == Some(id))
            .and_then(|desk| desk.folder)
            .map(|folder| PathBuf::from(folder.path))
    }

    /// Closes Desk session `id` and answers its trash item.
    async fn close(&self, id: &pb::SessionId) -> pb::TrashItemId {
        self.desk()
            .await
            .close_desk_session(pb::CloseDeskSessionRequest {
                idempotency_key: None,
                session: Some(id.clone()),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner()
            .channel_desk
            .and_then(|part| part.trash_item)
            .expect("the closed session names its trash item")
    }

    /// The trash item that is Desk session `id`'s folder.
    async fn item_of(&self, id: &pb::SessionId) -> pb::TrashItemId {
        let was = pb::EntityRef {
            entity: Some(pb::entity_ref::Entity::Session(id.clone())),
        };
        self.trash()
            .await
            .list_trash(pb::ListTrashRequest::default())
            .await
            .unwrap()
            .into_inner()
            .items
            .into_iter()
            .find(|item| item.was.as_ref() == Some(&was))
            .and_then(|item| item.id)
            .expect("the Desk session has its own trash item")
    }

    async fn restore(&self, item: &pb::TrashItemId) -> Result<(), tonic::Status> {
        self.trash()
            .await
            .restore_from_trash(pb::RestoreFromTrashRequest {
                idempotency_key: None,
                id: Some(item.clone()),
            })
            .await
            .map(|_| ())
    }

    /// Deletes the channel, its sessions' work included, and answers its
    /// trash item.
    async fn delete_channel(&self) -> pb::TrashItemId {
        let mut channels = self.daemon.channel_service().await;
        let plan = channels
            .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: Some(self.channel_id()) })
            .await
            .unwrap()
            .into_inner();
        let deleted = channels
            .delete_channel(pb::DeleteChannelRequest {
                idempotency_key: None,
                channel: plan.channel.clone(),
                plan: plan.token.clone(),
                presence: None,
                delete_unpushed_work: true,
                end_live: Some(true),
            })
            .await
            .unwrap()
            .into_inner();
        let done = finished(&self.daemon, deleted.operation.as_ref().unwrap()).await;
        assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
        deleted.trash_item.unwrap()
    }

    async fn channel_listed(&self) -> bool {
        self.daemon
            .channel_service()
            .await
            .list_channels(pb::ListChannelsRequest::default())
            .await
            .unwrap()
            .into_inner()
            .channels
            .iter()
            .any(|channel| channel.id == self.channel.id)
    }

    /// Brings archived session `id` back and waits for it.
    async fn restore_session(&self, id: &pb::SessionId) {
        let started = self
            .daemon
            .session_service()
            .await
            .restore_session(pb::RestoreSessionRequest {
                idempotency_key: None,
                id: Some(id.clone()),
                machine: None,
                stopped: false,
            })
            .await
            .unwrap()
            .into_inner();
        let done = finished(&self.daemon, &started).await;
        assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
    }

    /// Types `line` to session `id`'s agent, so its conversation holds it.
    async fn say(&self, id: &pb::SessionId, line: &str) {
        let tab = agent_tab(&self.daemon, id).await;
        let mut attach = Attach::start(&self.daemon, open(&tab, 0)).await;
        attach.type_text(&format!("{line}\r")).await;
        attach.wait_for(&format!("got: {line}")).await;
    }

    /// Every start of `conversation`'s agent, oldest first.
    fn starts(&self, conversation: &str) -> Vec<Start> {
        let log = std::fs::read_to_string(self.world.root.join("conv.log")).unwrap_or_default();
        log.lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                match fields.as_slice() {
                    [mode, id, key, place, cwd] if *id == conversation => Some(Start {
                        mode: (*mode).to_owned(),
                        key: (*key).to_owned(),
                        place: PathBuf::from(place),
                        cwd: PathBuf::from(cwd),
                    }),
                    _ => None,
                }
            })
            .collect()
    }

    /// The conversation file the agent keeps for `conversation` in `folder`.
    fn conversation_at(&self, folder: &Path, conversation: &str) -> PathBuf {
        self.world.conv.join(dashed(folder)).join(format!("{conversation}.jsonl"))
    }

    /// Waits until `session`'s agent has started `times` times and runs, and
    /// checks it took up its conversation in `folder`: started there by a
    /// resume, its conversation kept at `folder`'s key with what was said to
    /// it and naming `folder`, and nothing left at `before`'s key.
    async fn resumed_in(&self, session: &Started, folder: &Path, times: usize, said: &str) {
        self.wait_starts(&session.conversation, times).await;
        let last = self.starts(&session.conversation).pop().unwrap();
        assert_eq!(last.mode, "--resume", "it took up its conversation: {last:?}");
        assert_eq!(last.key, dashed(folder), "its conversation is keyed by its new folder: {last:?}");
        assert_eq!(last.place, folder, "{last:?}");
        assert_eq!(last.cwd, std::fs::canonicalize(folder).unwrap(), "it runs in its new folder: {last:?}");
        let back = wait_running(&self.daemon, &session.id).await;
        assert_eq!(back.conversation_id, session.conversation, "the same conversation");
        let kept = std::fs::read_to_string(self.conversation_at(folder, &session.conversation))
            .expect("the conversation is at its new folder's key");
        assert!(kept.lines().any(|line| line == said), "it holds what was said before: {kept:?}");
        assert!(kept.contains(&folder.display().to_string()), "it names its new folder: {kept:?}");
        let before = self.conversation_at(&session.folder, &session.conversation);
        let deadline = Instant::now() + Duration::from_secs(30);
        while before.exists() {
            assert!(Instant::now() < deadline, "the conversation stayed at its old key: {}", before.display());
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Stops session `id`'s agent and resumes it, as its menu does: a
    /// channel renamed under a running agent leaves its conversation to be
    /// carried to the new folder's key by this resume (D801 D).
    async fn stop_and_resume(&self, id: &pb::SessionId) {
        let mut sessions = self.daemon.session_service().await;
        sessions
            .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(id.clone()), for_move: false })
            .await
            .expect("the agent stops");
        sessions
            .resume_session(pb::ResumeSessionRequest { id: Some(id.clone()), ..Default::default() })
            .await
            .expect("the agent resumes");
    }

    /// Waits until `conversation`'s agent has started `times` times.
    async fn wait_starts(&self, conversation: &str, times: usize) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.starts(conversation).len() < times {
            let started = self.starts(conversation).len();
            assert!(Instant::now() < deadline, "the agent started {started} times, not {times}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// What the store holds of every session, agent, Desk row, tab and
    /// previous conversation.
    fn store_dump(&self) -> String {
        let store = self.store();
        let sessions = store.sessions().unwrap();
        let mut out = format!(
            "{sessions:?}\n{:?}\n{:?}\n{:?}\n{:?}\n",
            store.agents().unwrap(),
            store.session_details().unwrap(),
            store.channel_desks().unwrap(),
            store.tabs().unwrap(),
        );
        for session in &sessions {
            out.push_str(&format!("{:?}\n", store.previous_conversations(&session.id).unwrap()));
        }
        out
    }

    /// Checks nothing in the store, under `now` or among the conversations
    /// names `old`.
    fn nothing_names(&self, old: &Path, now: &Path) {
        let found = needles(old);
        let dump = self.store_dump();
        for needle in &found {
            assert!(!dump.contains(needle.as_str()), "the store still names {needle}: {dump}");
        }
        let files = [files_naming(now, &found), files_naming(&self.world.conv, &found)].concat();
        assert!(files.is_empty(), "these still name {}: {files:?}", old.display());
    }
}

/// What names `old` or a folder under it: the path itself, written as a
/// path or as a quoted value, and the conversation keys of what was in it.
fn needles(old: &Path) -> Vec<String> {
    let path = old.display().to_string();
    let key = dashed(old);
    vec![
        format!("{path}/"),
        format!("{path}\""),
        format!("{key}-sessions-"),
        format!("{key}-desk-"),
        format!("{key}-main-"),
    ]
}

/// The files under `root`, git's objects aside, whose bytes hold any of
/// `needles`.
fn files_naming(root: &Path, needles: &[String]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let kind = std::fs::symlink_metadata(&path).unwrap();
            if kind.is_dir() {
                if path.file_name().is_some_and(|name| name != "objects") {
                    stack.push(path);
                }
            } else if kind.is_file() && kind.len() < 4 << 20 {
                let bytes = std::fs::read(&path).unwrap();
                if needles
                    .iter()
                    .any(|needle| bytes.windows(needle.len()).any(|window| window == needle.as_bytes()))
                {
                    found.push(path);
                }
            }
        }
    }
    found
}

/// `path` moved from under `old` to under `new`.
fn under(path: &Path, old: &Path, new: &Path) -> PathBuf {
    new.join(path.strip_prefix(old).expect("the folder was under the channel's"))
}

/// The names in `folder`, sorted.
fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Waits up to 30 seconds for `done` to hold.
async fn until<F, Fut>(what: &str, done: F)
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64
}

/// D314, D801 D: renaming Payments to Billing moves `<root>/payments/` to
/// `<root>/billing/` once. The channel session and the channel Desk session
/// keep running, with `<root>/payments` a link to the new folder; stopped
/// and resumed, each takes up its conversation in its new folder, and then
/// nothing names `<root>/payments/` any more.
#[tokio::test(flavor = "multi_thread")]
async fn a_rename_moves_the_channel_folder_and_every_session_resumes_there() {
    let fixture = Fixture::new().await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    let desk = fixture.desk_session("Plan the week").await;
    fixture.say(&session.id, "ledger-line").await;
    fixture.say(&desk.id, "desk-line").await;

    let renamed = fixture.rename("Billing", true).await.expect("the rename goes through");

    assert_eq!(renamed.name, "Billing");
    assert_eq!(renamed.folder, "billing", "the folder follows the name");
    let new = fixture.world.channels.join("billing");
    assert!(new.join("main/api").is_dir(), "the channel's main clone moved with it");
    assert!(
        std::fs::symlink_metadata(&old).unwrap().file_type().is_symlink(),
        "the old name is a link to the new folder while the agents run"
    );
    let session_folder = under(&session.folder, &old, &new);
    let desk_folder = under(&desk.folder, &old, &new);
    assert_eq!(fixture.folder_of(&session.id), session_folder);
    assert_eq!(fixture.folder_of(&desk.id), desk_folder);
    assert!(session_folder.is_dir() && desk_folder.is_dir());
    assert_eq!(fixture.desk_folder_listed(&desk.id).await.as_deref(), Some(desk_folder.as_path()));
    let repo = &get_session(&fixture.daemon, &session.id).await.repos[0];
    assert!(Path::new(&repo.path).starts_with(&session_folder), "{repo:?}");
    assert_eq!(fixture.starts(&session.conversation).len(), 1, "the session's agent was not started again");
    assert_eq!(fixture.starts(&desk.conversation).len(), 1, "the Desk session's agent was not started again");

    fixture.stop_and_resume(&session.id).await;
    fixture.stop_and_resume(&desk.id).await;
    fixture.resumed_in(&session, &session_folder, 2, "ledger-line").await;
    fixture.resumed_in(&desk, &desk_folder, 2, "desk-line").await;
    let launch = fixture.launch_of(&session.id);
    assert_eq!(launch.folder, session_folder);
    assert_eq!(launch.conversation_file, fixture.conversation_at(&session_folder, &session.conversation));
    fixture.nothing_names(&old, &new);
}

/// D801 D: a session in a turn needs no confirming: the rename goes through
/// without `confirm_running`, and the agent is not stopped.
#[tokio::test(flavor = "multi_thread")]
async fn a_rename_with_an_agent_in_a_turn_needs_no_confirming_and_does_not_stop_it() {
    let fixture = Fixture::with_turn_end(Some("hooks")).await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    assert!(is_running(&get_session(&fixture.daemon, &session.id).await));

    let renamed = fixture.rename("Billing", false).await.expect("nothing is cut off, so nothing is asked");

    assert_eq!(renamed.folder, "billing");
    let new = fixture.world.channels.join("billing");
    let session_folder = under(&session.folder, &old, &new);
    assert_eq!(fixture.folder_of(&session.id), session_folder);
    assert_eq!(fixture.starts(&session.conversation).len(), 1, "the agent was not stopped");
    assert!(is_running(&get_session(&fixture.daemon, &session.id).await));
}

/// D314: with every session of the channel between turns, a rename needs
/// no confirming.
#[tokio::test(flavor = "multi_thread")]
async fn a_rename_with_every_session_between_turns_needs_no_confirming() {
    let fixture = Fixture::with_turn_end(Some("hooks")).await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    let (token, _shell) = shell_token(&fixture.daemon, &session.id).await;
    let mut report = tonic::Request::new(pb::ReportTurnRequest {
        state: pb::TurnState::Idle as i32,
        reported_at_unix_ns: now_ns(),
    });
    report
        .metadata_mut()
        .insert("humata-session-token", tonic::metadata::MetadataValue::try_from(token.as_str()).unwrap());
    fixture.daemon.session_service().await.report_turn(report).await.unwrap();
    wait_status(&fixture.daemon, &session.id, pb::SessionStatus::Idle).await;

    let renamed = fixture.rename("Billing", false).await.expect("nothing is cut off, so nothing is asked");

    assert_eq!(renamed.folder, "billing");
    let new = fixture.world.channels.join("billing");
    assert!(new.join("main/api").is_dir());
    assert_eq!(fixture.folder_of(&session.id), under(&session.folder, &old, &new));
}

/// D314: a rename onto a folder that is already there is refused, with the
/// name taken, and changes nothing: not the name, not a folder, not a
/// session.
#[tokio::test(flavor = "multi_thread")]
async fn a_rename_onto_a_folder_that_is_there_is_refused_and_changes_nothing() {
    let fixture = Fixture::new().await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    let taken = fixture.world.channels.join("billing");
    std::fs::create_dir_all(&taken).unwrap();
    std::fs::write(taken.join("keep.txt"), "mine\n").unwrap();

    let refused = fixture.rename("Billing", true).await.unwrap_err();

    assert_eq!(refused.code(), Code::AlreadyExists, "{refused:?}");
    assert!(matches!(error_of(&refused).kind, Some(pb::error::Kind::NameTaken(_))), "{refused:?}");
    let now = fixture.channel_now().await;
    assert_eq!((now.name.as_str(), now.folder.as_str()), ("Payments", fixture.channel.folder.as_str()));
    assert_eq!(names_in(&taken), ["keep.txt"]);
    assert!(old.join("main/api").is_dir());
    assert_eq!(fixture.folder_of(&session.id), session.folder);
    assert!(session.folder.is_dir());
    assert_eq!(fixture.starts(&session.conversation).len(), 1, "the agent was not stopped");
    assert!(is_running(&get_session(&fixture.daemon, &session.id).await));
}

/// MV-16, D194: a deleted channel restored while something took its folder
/// comes back at `<old>-2`. Its channel session and its channel Desk
/// session are both under `<old>-2/` and resume with their conversations;
/// nothing names the old folder, and what took it is left as it was.
#[tokio::test(flavor = "multi_thread")]
async fn a_channel_restored_elsewhere_brings_its_sessions_back_under_its_new_folder() {
    let fixture = Fixture::new().await;
    let old = fixture.old();
    let session = fixture.session("Ledger fix").await;
    let desk = fixture.desk_session("Plan the week").await;
    fixture.say(&session.id, "ledger-line").await;
    fixture.say(&desk.id, "desk-line").await;
    let item = fixture.delete_channel().await;
    let desk_item = fixture.item_of(&desk.id).await;
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join("stray.txt"), "not the channel\n").unwrap();

    fixture.restore(&item).await.expect("the channel comes back");
    until("the channel coming back", || fixture.channel_listed()).await;

    let folder = format!("{}-2", fixture.channel.folder);
    assert_eq!(fixture.channel_now().await.folder, folder);
    let new = fixture.world.channels.join(&folder);
    let session_folder = under(&session.folder, &old, &new);
    let desk_folder = under(&desk.folder, &old, &new);
    assert_eq!(fixture.folder_of(&session.id), session_folder);
    assert_eq!(fixture.folder_of(&desk.id), desk_folder, "the Desk session's record moved with its channel");
    let store = fixture.store();
    for agent in [store.agent(&session.id.value).unwrap(), store.agent(&desk.id.value).unwrap()].into_iter().flatten() {
        assert!(agent.launch.folder.starts_with(&new), "a kept launch names the new folder: {agent:?}");
    }

    fixture.restore_session(&session.id).await;
    fixture.resumed_in(&session, &session_folder, 2, "ledger-line").await;
    let launch = fixture.launch_of(&session.id);
    assert_eq!(launch.folder, session_folder);
    assert_eq!(launch.conversation_file, fixture.conversation_at(&session_folder, &session.conversation));
    fixture.restore(&desk_item).await.expect("then its Desk session comes back");
    assert_eq!(fixture.desk_folder_listed(&desk.id).await.as_deref(), Some(desk_folder.as_path()));
    assert!(desk_folder.is_dir());
    fixture.resumed_in(&desk, &desk_folder, 2, "desk-line").await;

    assert_eq!(names_in(&old), ["stray.txt"], "what took the old folder is left as it was");
    fixture.nothing_names(&old, &new);
}

/// MV-16, CD-4: a Desk session closed into the trash before its channel was
/// renamed comes back in the channel's Desk folder as it is now, and takes
/// up its conversation there.
#[tokio::test(flavor = "multi_thread")]
async fn a_desk_session_restored_after_a_rename_lands_in_the_new_desk_folder() {
    let fixture = Fixture::new().await;
    let old = fixture.old();
    let desk = fixture.desk_session("Plan the week").await;
    fixture.say(&desk.id, "desk-line").await;
    let item = fixture.close(&desk.id).await;

    let renamed = fixture.rename("Billing", false).await.expect("nothing runs, so the rename goes through");
    assert_eq!(renamed.folder, "billing");
    fixture.restore(&item).await.expect("the Desk session comes back");

    let new = fixture.world.channels.join("billing");
    let desk_folder = under(&desk.folder, &old, &new);
    assert_eq!(fixture.desk_folder_listed(&desk.id).await.as_deref(), Some(desk_folder.as_path()));
    assert_eq!(fixture.folder_of(&desk.id), desk_folder);
    assert!(desk_folder.is_dir());
    assert!(!old.exists(), "nothing came back to the old folder");
    fixture.resumed_in(&desk, &desk_folder, 2, "desk-line").await;
    let launch = fixture.launch_of(&desk.id);
    assert_eq!(launch.folder, desk_folder);
    assert_eq!(launch.conversation_file, fixture.conversation_at(&desk_folder, &desk.conversation));
    fixture.nothing_names(&old, &new);
}
