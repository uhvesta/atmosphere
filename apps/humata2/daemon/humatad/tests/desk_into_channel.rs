//! A Desk session moved into a channel (DK-15..DK-19, D305, D461, MV-16)
//! through a real `humatad2`, on this Mac. A scratch session lands in the
//! channel's `sessions/<name>` and resumes there with its conversation,
//! and nothing is left at its old Desk place or its old conversation key,
//! and nothing in the store names them. A folder the user picked is copied
//! into the channel like a scratch one and the original stays where it was,
//! whether or not an older app still sends the old "move the folder too"
//! tick, which is ignored (D461). A move that would cut off a
//! running turn asks first and changes nothing. A Copilot session moves
//! too: its session state, kept by id, is rewritten where it is to name
//! the new folder, and it resumes there.
//! A power cut before the switch is undone at the next start, one after it
//! is finished.
//!
//! The agent keeps each conversation at `<conv>/<folder, dashed>/<id>.jsonl`
//! as Claude does, so a session whose folder moved resumes only if its
//! conversation moved with it. Everything runs under a HOME of the test's
//! own.

use humatad_test_common as common;

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Build, Daemon, World, agent_tab, binary, create_channel, error_of, finished, folder, get_session, is_running,
    last_start, open, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::{DeskFolderKind, Store};
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

/// An agent Settings › Agents knows as Copilot: its conversation is
/// Copilot's session state, kept by id under `COPILOT_HOME`.
const COPILOT: &str = "copilot";

/// The test agent as Copilot: on a start it writes the session start
/// Copilot CLI 1.0.112-0 writes, naming the folder it was given, into
/// `<state>/<id>/events.jsonl`, and logs every start as the agent above
/// does. A resume whose session state doesn't name the folder it is
/// given fails. What is said to it is not written down, as that would be
/// no Copilot record.
const COPILOT_AGENT: &str = r#"STATE='@STATE@'
mode=$1; id=$2; key=$3; place=$4
f="$STATE/$id/events.jsonl"
printf '%s\t%s\t%s\t%s\t%s\n' "$mode" "$id" "$key" "$place" "$(pwd -P)" >> '@LOG@'
case "$mode" in
  --session-id) mkdir -p "$STATE/$id"
    printf '{"type":"session.start","data":{"sessionId":"%s","version":1,"producer":"copilot-agent","copilotVersion":"1.0.112-0","startTime":"2026-11-17T02:42:42.725Z","context":{"cwd":"%s"}},"id":"ccf77867-e1d1-42aa-a40e-3862c7cf317e","timestamp":"2026-11-17T02:42:43.111Z","parentId":null}\n' "$id" "$place" > "$f.partial"
    mv "$f.partial" "$f" ;;
  --resume) grep -qF "\"cwd\":\"$place\"" "$f" || { echo "no session state naming $place at $f"; exit 3; } ;;
esac
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; done
"#;

/// What is said to a session before it moves, one turn each.
const SAID: [&str; 3] = ["sketch the Q4 plan", "add the payments milestones", "it belongs with payments now"];

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

/// Makes the world's agent the one above, and Copilot the same script
/// under Copilot's conversation format; `turn_end` as Settings › Agents
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
    let copilot_script = world.root.join("copilot.sh");
    let state = copilot_state(world);
    let log = world.root.join("conv.log");
    let text = COPILOT_AGENT.replace("@STATE@", &state.display().to_string());
    std::fs::write(&copilot_script, text.replace("@LOG@", &log.display().to_string())).unwrap();
    let mut copilot = agent.clone();
    copilot["name"] = COPILOT.into();
    copilot["start_args"] =
        serde_json::json!([copilot_script, "--session-id", "{conversation}", "{folder_dashed}", "{folder}"]);
    copilot["resume_args"] =
        serde_json::json!([copilot_script, "--resume", "{conversation}", "{folder_dashed}", "{folder}"]);
    copilot["conversation_file"] = "{home}/.copilot/session-state/{conversation}/events.jsonl".into();
    copilot["conversation_format"] = "copilot-session-state".into();
    copilot["version_args"] = serde_json::json!([]);
    std::fs::write(&world.agents, serde_json::json!([agent, copilot]).to_string()).unwrap();
}

/// Where the Copilot agent keeps its session state: `COPILOT_HOME`'s
/// `session-state`, the home being the fixture's `<root>/home`.
fn copilot_state(world: &World) -> PathBuf {
    world.root.join("home/.copilot/session-state")
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
    /// The HOME variables, which every start of the daemon needs.
    env: Vec<(String, String)>,
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
        let refs: Vec<(&str, &str)> = env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        let mut args = world.args();
        args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &arg_refs, &refs).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        Self { daemon, world, channel, env }
    }

    /// Restarts the daemon with its HOME and `extra`, and waits until it
    /// brought its sessions back.
    async fn restart_with(&mut self, extra: &[(&str, &str)]) {
        let mut env: Vec<(&str, &str)> = self.env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        env.extend_from_slice(extra);
        self.daemon.restart_env(&env).await;
        last_start(&self.daemon).await;
    }

    fn channel_id(&self) -> pb::ChannelId {
        self.channel.id.clone().unwrap()
    }

    /// The channel's session folders' folder.
    fn sessions_dir(&self) -> PathBuf {
        self.world.channels.join(&self.channel.folder).join("sessions")
    }

    fn store(&self) -> Store {
        Store::open(self.daemon.state.join("store")).unwrap()
    }

    /// The folder the store holds for `id`.
    fn folder_of(&self, id: &pb::SessionId) -> PathBuf {
        self.store().session(&id.value).unwrap().expect("the store has the session").folder
    }

    fn on_desk(&self, id: &pb::SessionId) -> bool {
        self.store().desk(&id.value).unwrap().is_some()
    }

    /// Makes the running scratch session `id` look as one an older release
    /// started in `picked` (D458): the store says its folder is `picked` and
    /// its Desk row is a picked one, its agent record names `picked` in its
    /// folder, arguments and conversation file, the conversation itself names
    /// `picked` where it named the scratch folder, and the conversation's folder
    /// key for `picked` is a link to the key the agent is writing to, so the
    /// turns it is still given land in the file the move will read.
    fn age_into_picked(&self, id: &pb::SessionId, picked: &Path) {
        let store = self.store();
        let mut session = store.session(&id.value).unwrap().expect("the store has the session");
        let scratch = std::mem::replace(&mut session.folder, picked.to_owned());
        store.put_session(&session).unwrap();
        let mut row = store.desk(&id.value).unwrap().expect("a Desk session has a row");
        row.kind = DeskFolderKind::Picked;
        store.put_desk(&row).unwrap();
        let (old_key, new_key) = (dashed(&scratch), dashed(picked));
        let (old_path, new_path) = (scratch.display().to_string(), picked.display().to_string());
        let swap = |text: &OsStr| -> OsString {
            text.to_string_lossy().replace(&old_key, &new_key).replace(&old_path, &new_path).into()
        };
        let mut agent = store.agent(&id.value).unwrap().expect("a running session has an agent");
        // The conversation an older release kept names the folder it ran in.
        let said = std::fs::read_to_string(&agent.launch.conversation_file).expect("a running session has a conversation");
        std::fs::write(&agent.launch.conversation_file, said.replace(&old_path, &new_path)).unwrap();
        agent.launch.folder = picked.to_owned();
        agent.launch.argv = agent.launch.argv.iter().map(|arg| swap(arg)).collect();
        agent.launch.resume_argv = agent.launch.resume_argv.iter().map(|arg| swap(arg)).collect();
        agent.launch.conversation_file = PathBuf::from(swap(agent.launch.conversation_file.as_os_str()));
        store.put_agent(&agent).unwrap();
        std::os::unix::fs::symlink(self.world.conv.join(&old_key), self.world.conv.join(&new_key)).unwrap();
    }

    async fn desk(&self) -> pb::desk_service_client::DeskServiceClient<Channel> {
        pb::desk_service_client::DeskServiceClient::new(self.daemon.channel().await)
    }

    /// Starts a plain Desk session in a scratch folder with `agent` (unset:
    /// the default), and waits until its agent runs. With `picked`, the
    /// session is then aged into one an older release started in that folder
    /// (no new start can, D451); `picked` must exist.
    async fn desk_session(&self, picked: Option<&Path>, agent: Option<&str>) -> Started {
        let started = self
            .desk()
            .await
            .start_desk_session(pb::StartDeskSessionRequest {
                title: "Plan Q4".to_owned(),
                agent: agent.map(|name| pb::AgentId { value: name.to_owned() }),
                access: Some(pb::DeskAccess { boards: true, channels_and_machines: false }),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        let id = started.session.unwrap();
        let session = wait_running(&self.daemon, &id).await;
        if let Some(picked) = picked {
            self.age_into_picked(&id, picked);
        }
        let folder = self.folder_of(&id);
        Started { id, conversation: session.conversation_id, folder }
    }

    async fn plan(&self, id: &pb::SessionId) -> Result<pb::DeskMovePlan, tonic::Status> {
        self.desk()
            .await
            .plan_desk_move(pb::PlanDeskMoveRequest {
                session: Some(id.clone()),
                channel: Some(self.channel_id()),
                branch: String::new(),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// The user's move of `id` into Payments by `plan`, as it started.
    async fn start_move(
        &self,
        id: &pb::SessionId,
        plan: &pb::DeskMovePlan,
        old_tick: bool,
        confirm_running: bool,
    ) -> Result<pb::Operation, tonic::Status> {
        self.desk()
            .await
            .move_desk_session_to_channel(pb::MoveDeskSessionToChannelRequest {
                session: Some(id.clone()),
                channel: Some(self.channel_id()),
                plan: plan.token.clone(),
                // Deprecated and ignored (D461); an older app may still send it.
                move_picked_folder: old_tick,
                confirm_running,
                ..Default::default()
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// As [`Self::start_move`], waiting for the move to end.
    async fn move_in(
        &self,
        id: &pb::SessionId,
        plan: &pb::DeskMovePlan,
        old_tick: bool,
        confirm_running: bool,
    ) -> Result<pb::Operation, tonic::Status> {
        let started = self.start_move(id, plan, old_tick, confirm_running).await?;
        Ok(finished(&self.daemon, &started).await)
    }

    async fn say(&self, id: &pb::SessionId, line: &str) {
        let tab = agent_tab(&self.daemon, id).await;
        let mut attach = common::Attach::start(&self.daemon, open(&tab, 0)).await;
        attach.type_text(&format!("{line}\r")).await;
        attach.wait_for(&format!("got: {line}")).await;
    }

    async fn say_three(&self, id: &pb::SessionId) {
        for line in SAID {
            self.say(id, line).await;
        }
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

    /// Waits until `session`'s agent last started in `folder` and runs, and
    /// checks it took up its conversation there: started by a resume, its
    /// conversation kept at `folder`'s key with every turn said to it and
    /// naming `folder`, and nothing left at its old folder's key.
    async fn resumed_in(&self, session: &Started, folder: &Path) {
        until("the agent to start in its new folder", || {
            self.starts(&session.conversation).last().is_some_and(|last| last.place == folder)
        })
        .await;
        let last = self.starts(&session.conversation).pop().unwrap();
        assert_eq!(last.mode, "--resume", "it took up its conversation: {last:?}");
        assert_eq!(last.key, dashed(folder), "its conversation is keyed by its new folder: {last:?}");
        assert_eq!(last.cwd, std::fs::canonicalize(folder).unwrap(), "it runs in its new folder: {last:?}");
        let back = wait_running(&self.daemon, &session.id).await;
        assert_eq!(back.conversation_id, session.conversation, "the same conversation");
        let kept = std::fs::read_to_string(self.conversation_at(folder, &session.conversation))
            .expect("the conversation is at its new folder's key");
        for said in SAID {
            assert!(kept.lines().any(|line| line == said), "it holds {said:?}: {kept:?}");
        }
        assert!(kept.contains(&folder.display().to_string()), "it names its new folder: {kept:?}");
        let before = self.conversation_at(&session.folder, &session.conversation);
        until("the conversation to leave its old key", || !before.exists()).await;
    }

    /// What the store holds of every session, agent, start, channel Desk
    /// row, tab and earlier conversation.
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

    /// Checks that nothing is left at `old` and that nothing in the store,
    /// under `now` or among the conversations names it.
    async fn nothing_names(&self, old: &Path, now: &Path) {
        until("the old folder to go", || !old.exists()).await;
        let key = self.world.conv.join(dashed(old));
        assert!(names_in(&key).is_empty(), "the old key still holds {:?}", names_in(&key));
        let found = needles(old);
        let dump = self.store_dump();
        for needle in &found {
            assert!(!dump.contains(needle.as_str()), "the store still names {needle}: {dump}");
        }
        let files = [files_naming(now, &found), files_naming(&self.world.conv, &found)].concat();
        assert!(files.is_empty(), "these still name {}: {files:?}", old.display());
    }

    /// Waits until `operation`'s copy is done; the watch pushes each change,
    /// so nothing sleeps.
    async fn copied(&self, operation: &pb::Operation) {
        let done = |operation: &pb::Operation| {
            operation.steps.iter().any(|step| step.key == "copy" && step.state == pb::StepState::Done as i32)
        };
        let mut watch = pb::operation_service_client::OperationServiceClient::new(self.daemon.channel().await)
            .watch_operation(pb::WatchOperationRequest { id: operation.id.clone(), from_version: 0 })
            .await
            .expect("the move can be watched")
            .into_inner();
        let mut last = operation.clone();
        while !done(&last) {
            match tokio::time::timeout(Duration::from_secs(60), watch.message()).await {
                Ok(Ok(Some(next))) => last = next,
                other => panic!("the move never finished its copy ({other:?}); it last was {last:?}"),
            }
        }
    }

    /// Waits until `id` is no longer moving.
    async fn settled(&self, id: &pb::SessionId) -> pb::Session {
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let session = get_session(&self.daemon, id).await;
            if session.status != pb::SessionStatus::Moving as i32 {
                return session;
            }
            assert!(Instant::now() < deadline, "the session is still moving");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

/// What names `old` or a folder under it: the path itself, written as a
/// path or as a quoted value.
fn needles(old: &Path) -> Vec<String> {
    let path = old.display().to_string();
    vec![format!("{path}/"), format!("{path}\"")]
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

/// The names in `folder`, sorted; none when it isn't there.
fn names_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .map(|found| found.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// Waits up to 30 seconds for `done` to hold.
async fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn succeeded(operation: &pb::Operation) -> bool {
    operation.state == pb::OperationState::Succeeded as i32
}

/// A picked folder outside Humata's folders, holding notes.md.
fn picked_folder(world: &World) -> PathBuf {
    let picked = world.root.join("picked").join("q4-notes");
    std::fs::create_dir_all(&picked).unwrap();
    std::fs::write(picked.join("notes.md"), "# Q4\n").unwrap();
    std::fs::canonicalize(picked).unwrap()
}

/// DK-15, D801 E: a scratch Desk session moved into Payments is renamed to
/// `<root>/payments/sessions/<name>`, not copied: its agent is not started
/// again, and its old folder is a link to the new one while it runs. Its
/// next resume (a stop and resume here) takes up its conversation under the
/// new folder's key with every turn of it.
#[tokio::test(flavor = "multi_thread")]
async fn a_scratch_desk_session_moved_into_a_channel_keeps_running_and_resumes_in_its_session_folder_later() {
    let fixture = Fixture::new().await;
    let session = fixture.desk_session(None, None).await;
    fixture.say_three(&session.id).await;

    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);
    assert_eq!(new.parent(), Some(fixture.sessions_dir().as_path()), "it lands among the channel's sessions");
    let moved = fixture.move_in(&session.id, &plan, false, false).await.unwrap();

    assert!(succeeded(&moved), "{:?}", moved.error);
    assert_eq!(fixture.folder_of(&session.id), new);
    assert_eq!(get_session(&fixture.daemon, &session.id).await.channel, Some(fixture.channel_id()));
    assert!(!fixture.on_desk(&session.id), "it left the Desk");
    assert_eq!(fixture.starts(&session.conversation).len(), 1, "the agent was not started again");
    assert_eq!(std::fs::read_link(&session.folder).unwrap(), new, "the old name is a link to the new folder");

    let mut sessions = fixture.daemon.session_service().await;
    sessions
        .stop_session(pb::StopSessionRequest { idempotency_key: None, id: Some(session.id.clone()), for_move: false })
        .await
        .expect("the agent stops");
    sessions
        .resume_session(pb::ResumeSessionRequest { id: Some(session.id.clone()), ..Default::default() })
        .await
        .expect("the agent resumes");
    fixture.resumed_in(&session, &new).await;
    fixture.nothing_names(&session.folder, &new).await;
}

/// D801 E: a rename needs no confirming however busy the agent is, since it
/// does not stop it.
#[tokio::test(flavor = "multi_thread")]
async fn a_scratch_session_in_a_turn_is_renamed_into_the_channel_with_no_confirming() {
    let fixture = Fixture::with_turn_end(Some("hooks")).await;
    let session = fixture.desk_session(None, None).await;
    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);

    let moved = fixture.move_in(&session.id, &plan, false, false).await.expect("no confirming is asked for");

    assert!(succeeded(&moved), "{:?}", moved.error);
    assert_eq!(fixture.folder_of(&session.id), new);
    assert_eq!(fixture.starts(&session.conversation).len(), 1, "the agent was not stopped");
}

/// D461, DK-17: a picked folder (one picked before new sessions stopped
/// being started in one) moves into the channel like a scratch folder: it is
/// copied to the channel place, the session resumes there, and the original
/// stays where it was with its files. No tick is needed and nothing the user
/// owns goes to the Bin.
#[tokio::test(flavor = "multi_thread")]
async fn a_picked_folder_is_copied_into_the_channel_and_the_original_stays() {
    let fixture = Fixture::new().await;
    let picked = picked_folder(&fixture.world);
    let session = fixture.desk_session(Some(&picked), None).await;
    assert_eq!(session.folder, picked);
    fixture.say_three(&session.id).await;

    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);
    assert_eq!(new.parent(), Some(fixture.sessions_dir().as_path()));
    let moved = fixture.move_in(&session.id, &plan, false, true).await.unwrap();

    assert!(succeeded(&moved), "{:?}", moved.error);
    fixture.resumed_in(&session, &new).await;
    assert_eq!(fixture.folder_of(&session.id), new);
    assert!(!fixture.on_desk(&session.id), "it left the Desk");
    assert_eq!(std::fs::read_to_string(new.join("notes.md")).unwrap(), "# Q4\n", "the copy is at the channel place");
    assert_eq!(std::fs::read_to_string(picked.join("notes.md")).unwrap(), "# Q4\n", "the original keeps its files");
}

/// D461: an older app that still sends the "move the folder too" tick gets
/// the same move: the picked folder is copied, the session resumes in the
/// channel place, and the original stays with its files.
#[tokio::test(flavor = "multi_thread")]
async fn the_old_tick_is_ignored_and_a_picked_folder_still_stays_where_it_was() {
    let fixture = Fixture::new().await;
    let picked = picked_folder(&fixture.world);
    let session = fixture.desk_session(Some(&picked), None).await;
    fixture.say_three(&session.id).await;

    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);
    let moved = fixture.move_in(&session.id, &plan, true, true).await.unwrap();

    assert!(succeeded(&moved), "{:?}", moved.error);
    fixture.resumed_in(&session, &new).await;
    assert_eq!(fixture.folder_of(&session.id), new);
    assert_eq!(std::fs::read_to_string(new.join("notes.md")).unwrap(), "# Q4\n");
    assert_eq!(std::fs::read_to_string(picked.join("notes.md")).unwrap(), "# Q4\n", "the original keeps its files");
}

/// DK-15, D314 and D318's rule for running turns: a move while the session
/// is Working, Probably idle or Needs you is refused unless confirmed, naming the busy session, and
/// nothing changes; confirmed, it goes through.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_that_would_cut_off_a_running_turn_asks_first_and_changes_nothing() {
    let fixture = Fixture::with_turn_end(Some("hooks")).await;
    // A picked folder is copied, which stops the agent; a scratch one is
    // renamed, which doesn't.
    let picked = picked_folder(&fixture.world);
    let session = fixture.desk_session(Some(&picked), None).await;
    assert!(is_running(&get_session(&fixture.daemon, &session.id).await));
    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);

    let asked = fixture.move_in(&session.id, &plan, false, false).await.unwrap_err();

    assert_eq!(asked.code(), Code::FailedPrecondition, "{asked:?}");
    let busy = match error_of(&asked).kind {
        Some(pb::error::Kind::SessionBusy(busy)) => busy.session,
        other => panic!("not a busy session: {other:?}"),
    };
    assert_eq!(busy.as_ref(), Some(&session.id), "it names the session in a turn");
    assert_eq!(fixture.folder_of(&session.id), session.folder);
    assert!(fixture.on_desk(&session.id));
    assert!(session.folder.is_dir());
    assert!(!new.exists());
    assert_eq!(fixture.starts(&session.conversation).len(), 1, "the agent was not stopped");
    assert!(is_running(&get_session(&fixture.daemon, &session.id).await));

    let moved = fixture.move_in(&session.id, &plan, false, true).await.expect("confirmed, it goes through");

    assert!(succeeded(&moved), "{:?}", moved.error);
    assert_eq!(fixture.folder_of(&session.id), new);
    until("the agent to start in its new folder", || {
        fixture.starts(&session.conversation).last().is_some_and(|last| last.place == new)
    })
    .await;
}

/// CV-3, CV-7, MV-16: a Copilot Desk session moves as any other. Its
/// session state, kept by id under `COPILOT_HOME`, is rewritten where it
/// is to name the new folder, and the agent resumes it there.
#[tokio::test(flavor = "multi_thread")]
async fn a_copilot_desk_session_moved_into_a_channel_resumes_its_session_state_in_the_new_folder() {
    let fixture = Fixture::new().await;
    // A picked folder is copied; a scratch one is renamed and its agent not restarted.
    let picked = picked_folder(&fixture.world);
    let session = fixture.desk_session(Some(&picked), Some(COPILOT)).await;
    let events = copilot_state(&fixture.world).join(&session.conversation).join("events.jsonl");
    until("the agent to write its session state", || events.is_file()).await;

    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);
    let moved = fixture.move_in(&session.id, &plan, false, true).await.unwrap();

    assert!(succeeded(&moved), "{:?}", moved.error);
    until("the agent to start in its new folder", || {
        fixture.starts(&session.conversation).last().is_some_and(|last| last.place == new)
    })
    .await;
    let last = fixture.starts(&session.conversation).pop().unwrap();
    assert_eq!(last.mode, "--resume", "it took up its session state: {last:?}");
    assert_eq!(fixture.folder_of(&session.id), new);
    assert!(!fixture.on_desk(&session.id), "it left the Desk");
    let kept = std::fs::read_to_string(&events).unwrap();
    assert!(kept.contains(&new.display().to_string()), "it names its new folder: {kept:?}");
    assert!(!kept.contains(&session.folder.display().to_string()), "it names the Desk folder: {kept:?}");
}

/// MV-4's power-cut rule for this move: cut after the copy, before the
/// switch, the move is undone at the next start. The session is still a
/// Desk session in its Desk folder with its conversation at its key, and
/// the copy is gone.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_cut_after_the_copy_is_undone_at_the_next_start() {
    let mut fixture = Fixture::new().await;
    fixture.restart_with(&[("HUMATA2_STOP_AT", "move-copied")]).await;
    let picked = picked_folder(&fixture.world);
    let session = fixture.desk_session(Some(&picked), None).await;
    fixture.say_three(&session.id).await;
    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);

    // The daemon stops at the point, so the call may or may not answer.
    let _ = fixture.start_move(&session.id, &plan, false, true).await;
    fixture.daemon.stopped(Duration::from_secs(120)).await;
    fixture.restart_with(&[]).await;

    let now = fixture.settled(&session.id).await;
    assert_ne!(now.channel, Some(fixture.channel_id()));
    assert!(fixture.on_desk(&session.id));
    assert_eq!(fixture.folder_of(&session.id), session.folder);
    assert!(session.folder.is_dir(), "its Desk folder is still there");
    let kept = std::fs::read_to_string(fixture.conversation_at(&session.folder, &session.conversation))
        .expect("the conversation is at its key");
    assert!(SAID.iter().all(|said| kept.lines().any(|line| line == *said)), "{kept:?}");
    until("the copy to go", || !new.exists()).await;
    let back = wait_running(&fixture.daemon, &session.id).await;
    assert_eq!(back.conversation_id, session.conversation);
}

/// A file written in the session's folder after the pack and the copy, a
/// file removed meanwhile and a line added to its conversation all move
/// with it: once the agent stops, the landing is synced again before it is
/// taken up. The move holds after the copy until the test has written.
#[tokio::test(flavor = "multi_thread")]
async fn what_the_session_wrote_while_it_was_copied_moves_with_it() {
    let mut fixture = Fixture::new().await;
    let hold = fixture.world.root.join("hold.fifo");
    assert!(std::process::Command::new("mkfifo").arg(&hold).status().unwrap().success());
    let fifo = hold.display().to_string();
    fixture.restart_with(&[("HUMATA2_HOLD_AT", "move-copied"), ("HUMATA2_HOLD_FIFO", &fifo)]).await;
    let picked = picked_folder(&fixture.world);
    let session = fixture.desk_session(Some(&picked), None).await;
    fixture.say_three(&session.id).await;
    std::fs::write(session.folder.join("gone.md"), "removed after the copy\n").unwrap();
    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);

    let started = fixture.start_move(&session.id, &plan, false, true).await.unwrap();
    fixture.copied(&started).await;
    std::fs::write(session.folder.join("late.md"), "written after the copy\n").unwrap();
    std::fs::remove_file(session.folder.join("gone.md")).unwrap();
    let conversation = fixture.conversation_at(&session.folder, &session.conversation);
    let mut talk = std::fs::read_to_string(&conversation).unwrap();
    talk.push_str("said while it was copied\n");
    std::fs::write(&conversation, talk).unwrap();
    let opened = tokio::task::spawn_blocking(move || std::fs::OpenOptions::new().write(true).open(hold).map(drop));
    opened.await.unwrap().expect("the held move opens the FIFO");
    let moved = finished(&fixture.daemon, &started).await;

    assert!(succeeded(&moved), "{:?}", moved.error);
    fixture.resumed_in(&session, &new).await;
    assert_eq!(std::fs::read_to_string(new.join("late.md")).unwrap(), "written after the copy\n");
    assert!(!new.join("gone.md").exists(), "a file removed from the session's folder is not in the channel's");
    let kept = std::fs::read_to_string(fixture.conversation_at(&new, &session.conversation)).unwrap();
    assert!(kept.lines().any(|line| line == "said while it was copied"), "{kept:?}");
}

/// MV-4's power-cut rule for this move: cut after the switch, the move is
/// finished at the next start. The session resumes in the channel's
/// session folder and nothing is left at its Desk folder.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_cut_after_the_switch_is_finished_at_the_next_start() {
    let mut fixture = Fixture::new().await;
    fixture.restart_with(&[("HUMATA2_STOP_AT", "move-switched")]).await;
    let picked = picked_folder(&fixture.world);
    let session = fixture.desk_session(Some(&picked), None).await;
    fixture.say_three(&session.id).await;
    let plan = fixture.plan(&session.id).await.unwrap();
    let new = PathBuf::from(&plan.folder);

    let _ = fixture.start_move(&session.id, &plan, false, true).await;
    fixture.daemon.stopped(Duration::from_secs(120)).await;
    fixture.restart_with(&[]).await;

    let now = fixture.settled(&session.id).await;
    assert_eq!(now.channel, Some(fixture.channel_id()));
    assert!(!fixture.on_desk(&session.id));
    assert_eq!(fixture.folder_of(&session.id), new);
    fixture.resumed_in(&session, &new).await;
}
