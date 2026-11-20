//! Continue in another channel (CV-8, D307) through a real `humatad2`: a
//! session in Payments goes on in Billing as a new session there, on
//! Billing's repository and a branch of its own, whose agent resumes a copy
//! of the conversation, from now or from before a turn, under a new id. The
//! session it came from, and its conversation, are as they were. Claude,
//! Codex and Copilot alike: each one's turns are listed, and a fork before
//! a turn and a continue read and cut its conversation through its CLI's
//! adapter. A conversation holding a record Humata doesn't know is refused,
//! as are a channel that isn't there and an archived session, before
//! anything changes. A power cut part way leaves no new session once the
//! daemon is back.
//!
//! Everything runs under a HOME of the test's own.

use humatad_test_common as common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, World, agent_tab, binary, create_channel, error_of, finished, folder, get_session,
    last_start, machine_id, open, start_request, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use humata2_store::Store;
use tonic::Code;

const CLAUDE: &str = common::AGENT_NAME;
const CODEX: &str = "codex";
const COPILOT: &str = "copilot";

/// The prompts of every test conversation, in order.
const PROMPTS: [&str; 3] = ["Find the flaky test", "Make it deterministic", "Now delete the old sleep"];

/// Where the Codex test agent keeps its rollouts, under `CODEX_HOME`.
const CODEX_DAY: &str = "sessions/2026/11/16";

/// The test agent, given its mode and its conversation file: a start makes
/// the file, a resume needs it there. It never writes the file, so what the
/// test put there is what a copy carries. Asked its version, it prints what
/// [`VERSION_FILE`] beside it holds, else a Copilot version Humata knows.
const AGENT: &str = r#"mode=$1; f=$2
# A file under the CLI's own home is given as the variable naming that home
# (the session's composed one) and the path under it.
if [ -n "${3:-}" ]; then eval "f=\"\${$2}/$3\""; fi
case "$mode" in
  --version) if [ -f "$0.version" ]; then cat "$0.version"; else echo "GitHub Copilot CLI 1.0.112-0."; fi; exit 0 ;;
  --session-id) mkdir -p "$(dirname "$f")"; : > "$f" ;;
  --resume) [ -f "$f" ] || { echo "no conversation at $f"; exit 3; } ;;
esac
# Reads keys itself, as agent CLIs do, so typing never waits out the
# holder's keys limit (see common/mod.rs).
stty -icanon 2>/dev/null
echo "agent up $mode"
while IFS= read -r line; do echo "got: $line"; done
"#;

/// Beside the test agent's script, the version it prints when asked.
const VERSION_FILE: &str = "continue-agent.sh.version";

/// A Claude conversation of the three prompts.
fn claude_transcript() -> String {
    let lines = [
        r#"{"type":"user","message":{"role":"user","content":"Find the flaky test"},"timestamp":"2026-11-16T09:00:00Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"It is retry_test."}]},"timestamp":"2026-11-16T09:00:05Z"}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"Make it deterministic"}]},"timestamp":"2026-11-16T09:01:00Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done with a fake clock."}]}}"#,
        r#"{"type":"user","message":{"role":"user","content":"Now delete the old sleep"},"timestamp":"2026-11-16T09:02:00Z"}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Deleted the sleep."}]}}"#,
    ];
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// A Codex rollout of the three prompts, each turn finished, for the
/// conversation `id`.
fn codex_transcript(id: &str) -> String {
    let mut lines = vec![serde_json::json!({
        "timestamp": "2026-11-16T00:00:00Z",
        "type": "session_meta",
        "payload": {"id": id, "session_id": id, "timestamp": "2026-11-16T00:00:00Z", "cli_version": "0.130.0"},
    })];
    for (index, prompt) in PROMPTS.iter().enumerate() {
        let turn = index + 1;
        let at = format!("2026-11-16T00:0{turn}:00Z");
        lines.push(serde_json::json!({
            "timestamp": at,
            "type": "event_msg",
            "payload": {
                "type": "item_completed",
                "turn_id": format!("t{turn}"),
                "item": {"type": "UserMessage", "id": format!("u{turn}"), "content": [{"type": "text", "text": prompt}]},
            },
        }));
        lines.push(serde_json::json!({
            "timestamp": at,
            "type": "event_msg",
            "payload": {"type": "task_complete", "turn_id": format!("t{turn}"), "last_agent_message": "ok"},
        }));
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// A Copilot session state of the three prompts, each turn finished, for
/// the conversation `id` working in `folder`.
fn copilot_transcript(id: &str, folder: &Path) -> String {
    let mut lines = vec![serde_json::json!({
        "type": "session.start",
        "data": {
            "sessionId": id,
            "version": 1,
            "producer": "copilot-agent",
            "copilotVersion": "1.0.112-0",
            "startTime": "2026-11-16T00:00:00.000Z",
            "selectedModel": "gpt-4.1",
            "contextTier": null,
            "context": {"cwd": folder.display().to_string()},
            "alreadyInUse": false,
            "remoteSteerable": false,
        },
        "id": "00000000-0000-4000-8000-000000000000",
        "timestamp": "2026-11-16T00:00:00.000Z",
        "parentId": null,
    })];
    let mut parent = "00000000-0000-4000-8000-000000000000".to_owned();
    for (index, prompt) in PROMPTS.iter().enumerate() {
        let turn = index + 1;
        let at = format!("2026-11-16T00:0{turn}:00.000Z");
        let records = [
            (
                "user.message",
                serde_json::json!({
                    "content": prompt,
                    "hasSubmittedTask": false,
                    "transformedContent": prompt,
                    "messageId": format!("m{turn}"),
                    "supportedNativeDocumentMimeTypes": [],
                    "delivery": "immediate",
                    "interactionId": format!("i{turn}"),
                    "turnId": "0",
                    "parentAgentTaskId": null,
                }),
            ),
            ("assistant.turn_start", serde_json::json!({"turnId": "0", "interactionId": format!("i{turn}"), "agentMode": "interactive"})),
            (
                "assistant.message",
                serde_json::json!({
                    "messageId": format!("a{turn}"),
                    "originatingMessageId": format!("m{turn}"),
                    "model": "gpt-4.1",
                    "content": "ok",
                    "toolRequests": [],
                    "interactionId": format!("i{turn}"),
                    "turnId": "0",
                    "rte": null,
                    "apiCallId": format!("c{turn}"),
                }),
            ),
            ("assistant.turn_end", serde_json::json!({"turnId": "0"})),
        ];
        for (step, (kind, data)) in records.into_iter().enumerate() {
            let record = format!("00000000-0000-4000-8000-00000000{turn:02}{step:02}");
            lines.push(serde_json::json!({"type": kind, "data": data, "id": record, "timestamp": at, "parentId": parent}));
            parent = record;
        }
    }
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// Settings › Agents: the test agent as Claude, Codex and Copilot, each
/// keeping its conversation where that CLI does (Claude's by folder, in the
/// world's conversations folder).
fn write_agents(world: &World) {
    let script = world.root.join("continue-agent.sh");
    std::fs::write(&script, AGENT).unwrap();
    // `args` locate the file as the agent finds it: an absolute path, or
    // its home's variable and the path under it, since each session runs
    // with a composed home of its own.
    let agent = |name: &str, format: &str, file: String, args: Vec<String>| {
        let start_args = [vec![script.display().to_string(), "--session-id".to_owned()], args.clone()].concat();
        let resume_args = [vec![script.display().to_string(), "--resume".to_owned()], args].concat();
        serde_json::json!({
            "name": name,
            "program": "/bin/sh",
            "start_args": start_args,
            "resume_args": resume_args,
            "conversation_file": file,
            "conversation_format": format,
            "version_args": [],
        })
    };
    let claude = format!("{}/{{folder_dashed}}/{{conversation}}.jsonl", world.conv.display());
    let codex = format!("{CODEX_DAY}/rollout-2026-11-16T00-00-00-{{conversation}}.jsonl");
    let copilot = "session-state/{conversation}/events.jsonl";
    // Copilot's CLI is asked its version at each launch (CV-3).
    let mut copilot_agent = agent(
        COPILOT,
        "copilot-session-state",
        format!("{{home}}/.copilot/{copilot}"),
        vec!["COPILOT_HOME".to_owned(), copilot.to_owned()],
    );
    copilot_agent["version_args"] = serde_json::json!([script, "--version"]);
    let agents = serde_json::json!([
        agent(CLAUDE, "claude-jsonl", claude.clone(), vec![claude]),
        agent(
            CODEX,
            "codex-rollout",
            format!("{{home}}/.codex/{codex}"),
            vec!["CODEX_HOME".to_owned(), codex.clone()],
        ),
        copilot_agent,
    ]);
    std::fs::write(&world.agents, agents.to_string()).unwrap();
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

/// The names in `dir`, sorted; none when it isn't there.
fn names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries.map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

/// Every file under `dir`, at any depth.
fn files_under(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, found);
        } else {
            found.push(path);
        }
    }
}

/// The session a test continues: its id, conversation id and file, and
/// what the file holds.
struct Source {
    id: pb::SessionId,
    conversation: String,
    file: PathBuf,
    text: String,
}

/// What is there before a refused or cut call.
struct Before {
    sessions: Vec<String>,
    billing: Vec<String>,
    files: Vec<PathBuf>,
}

struct Fixture {
    daemon: Daemon,
    world: World,
    home: PathBuf,
    payments: pb::Channel,
    billing: pb::Channel,
    /// The HOME variables, which every start of the daemon needs.
    env: Vec<(String, String)>,
}

impl Fixture {
    /// A daemon with channels Payments on acme/api and Billing on acme/web.
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        let home = dir.path().join("home");
        write_agents(&world);
        world.remote("acme", "api");
        world.remote("acme", "web");
        let env = home_env(&home);
        let refs: Vec<(&str, &str)> = env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        let mut args = world.args();
        args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with_env(Build::Admin, dir, &arg_refs, &refs).await;
        let (payments, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        let (billing, setup) = create_channel(&daemon, "Billing", &[("acme", "web")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        Self { daemon, world, home, payments, billing, env }
    }

    /// Restarts the daemon with its HOME and `extra`, and waits until it
    /// brought its sessions back.
    async fn restart_with(&mut self, extra: &[(&str, &str)]) {
        let mut env: Vec<(&str, &str)> = self.env.iter().map(|(name, value)| (name.as_str(), value.as_str())).collect();
        env.extend_from_slice(extra);
        self.daemon.restart_env(&env).await;
        last_start(&self.daemon).await;
    }

    fn billing_id(&self) -> pb::ChannelId {
        self.billing.id.clone().unwrap()
    }

    /// A channel's session folders' folder.
    fn sessions_dir(&self, channel: &pb::Channel) -> PathBuf {
        self.world.channels.join(&channel.folder).join("sessions")
    }

    fn store(&self) -> Store {
        Store::open(self.daemon.state.join("store")).unwrap()
    }

    /// Every conversation file of every agent.
    fn files(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        files_under(&self.world.conv, &mut found);
        files_under(&self.home.join(".codex/sessions"), &mut found);
        files_under(&self.home.join(".copilot/session-state"), &mut found);
        found.sort();
        found
    }

    async fn session_ids(&self) -> Vec<String> {
        let listed = self
            .daemon
            .session_service()
            .await
            .list_sessions(pb::ListSessionsRequest { channel: None, machine: None, include_archived: true, page: None })
            .await
            .unwrap()
            .into_inner()
            .sessions;
        let mut ids: Vec<String> = listed.into_iter().filter_map(|session| session.id.map(|id| id.value)).collect();
        ids.sort();
        ids
    }

    async fn before(&self) -> Before {
        Before { sessions: self.session_ids().await, billing: names(&self.sessions_dir(&self.billing)), files: self.files() }
    }

    /// A session of `agent` in Payments, running, whose conversation holds
    /// the three prompts.
    async fn source(&self, agent: &str) -> Source {
        let machine = machine_id(&self.daemon).await;
        let mut request = start_request(self.payments.id.as_ref().unwrap(), &machine, "Fix the flaky test");
        request.agent = Some(pb::AgentId { value: agent.to_owned() });
        let started = self.daemon.session_service().await.start_session(request).await.unwrap().into_inner();
        let id = started.session.unwrap();
        wait_running(&self.daemon, &id).await;
        let record = self.store().agent(&id.value).unwrap().expect("the session's agent is recorded");
        let launch = record.launch;
        let folder = self.store().session(&id.value).unwrap().expect("the store has the session").folder;
        let text = match agent {
            CODEX => codex_transcript(&launch.conversation_id),
            COPILOT => copilot_transcript(&launch.conversation_id, &folder),
            _ => claude_transcript(),
        };
        // Running means the terminal is up, not that the agent has made its
        // file yet: wait for that first, or its `: >` empties what is written.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while !launch.conversation_file.is_file() {
            assert!(
                std::time::Instant::now() < deadline,
                "the agent never made its conversation file {}",
                launch.conversation_file.display()
            );
            tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        }
        std::fs::write(&launch.conversation_file, &text).unwrap();
        Source { id, conversation: launch.conversation_id, file: launch.conversation_file, text }
    }

    async fn turns(&self, session: &pb::SessionId) -> Vec<(u32, String)> {
        let listed = self
            .daemon
            .session_service()
            .await
            .list_turns(pb::ListTurnsRequest { session: Some(session.clone()), page: None })
            .await
            .unwrap()
            .into_inner()
            .turns;
        listed.into_iter().map(|turn| (turn.index, turn.user_text)).collect()
    }

    /// Forks `parent` with its conversation, before turn `before_turn`.
    async fn fork(&self, parent: &pb::SessionId, before_turn: u32) -> Result<pb::Operation, tonic::Status> {
        self.fork_to(parent, before_turn, None).await
    }

    /// As [`Fixture::fork`], to `agent` when one is given.
    async fn fork_to(
        &self,
        parent: &pb::SessionId,
        before_turn: u32,
        agent: Option<&str>,
    ) -> Result<pb::Operation, tonic::Status> {
        self.daemon
            .session_service()
            .await
            .fork_session(pb::ForkSessionRequest {
                idempotency_key: None,
                parent: Some(parent.clone()),
                kind: pb::ForkKind::CodeAndContext as i32,
                before_turn,
                title: "Try a fake clock".to_owned(),
                agent: agent.map(|value| pb::AgentId { value: value.to_owned() }),
                model: String::new(),
                machine: None,
                branch: String::new(),
                reason: String::new(),
            })
            .await
            .map(tonic::Response::into_inner)
    }

    async fn continue_in(
        &self,
        source: &pb::SessionId,
        channel: pb::ChannelId,
        before_turn: u32,
    ) -> Result<pb::StartSessionResponse, tonic::Status> {
        self.daemon
            .session_service()
            .await
            .continue_in_channel(pb::ContinueInChannelRequest {
                idempotency_key: None,
                session: Some(source.clone()),
                channel: Some(channel),
                before_turn,
            })
            .await
            .map(tonic::Response::into_inner)
    }

    /// Waits until the daemon is gone: it kills itself at the fault point.
    async fn wait_dead(&self) {
        let deadline = Instant::now() + Duration::from_secs(60);
        while std::os::unix::net::UnixStream::connect(&self.daemon.socket).is_ok() {
            assert!(Instant::now() < deadline, "the daemon never reached its fault point");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The source session is where it was, in Payments, on its own
    /// conversation, and that conversation is as the test wrote it.
    async fn assert_source_untouched(&self, source: &Source) {
        let now = get_session(&self.daemon, &source.id).await;
        assert_eq!(now.channel, self.payments.id, "{now:?}");
        assert_eq!(now.conversation_id, source.conversation, "{now:?}");
        assert!(now.forks.is_empty(), "{now:?}");
        assert_eq!(std::fs::read_to_string(&source.file).unwrap(), source.text);
    }

    async fn assert_nothing_new(&self, before: &Before) {
        assert_eq!(self.session_ids().await, before.sessions, "no new session is left");
        assert_eq!(names(&self.sessions_dir(&self.billing)), before.billing, "no folder is left in Billing");
        assert_eq!(self.files(), before.files, "no conversation copy is left");
    }
}

/// Continues a session of `agent` in Billing before turn `before_turn`
/// (0: from now), and checks the new session has the prompts in `kept` and
/// none in `cut`, under a new id, where Billing's sessions are.
async fn continues(agent: &str, before_turn: u32, kept: &[&str], cut: &[&str]) {
    let fixture = Fixture::new().await;
    let source = fixture.source(agent).await;

    let response = fixture.continue_in(&source.id, fixture.billing_id(), before_turn).await.unwrap();
    let done = finished(&fixture.daemon, response.operation.as_ref().unwrap()).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
    let id = response.session.unwrap();
    assert_ne!(id, source.id);
    let session = wait_running(&fixture.daemon, &id).await;

    assert_eq!(session.channel, fixture.billing.id, "{session:?}");
    assert_eq!(session.agent.as_ref().map(|agent| agent.value.as_str()), Some(agent));
    assert!(session.fork_origin.is_none(), "a continued session is a top-level one: {session:?}");
    assert_eq!(
        session.repos.iter().filter_map(|repo| repo.repo.as_ref().map(|repo| repo.name.as_str())).collect::<Vec<_>>(),
        ["web"],
        "it works on Billing's repository"
    );
    let folder = fixture.store().session(&id.value).unwrap().expect("the store has the session").folder;
    assert!(folder.starts_with(fixture.sessions_dir(&fixture.billing)), "{}", folder.display());
    assert!(folder.join("web/.git").exists(), "Billing's repository is cloned there");

    let launch = fixture.store().agent(&id.value).unwrap().expect("its agent is recorded").launch;
    assert_ne!(launch.conversation_id, source.conversation, "the copy has an id of its own");
    assert_eq!(session.conversation_id, launch.conversation_id);
    assert_ne!(launch.conversation_file, source.file);
    let copy = std::fs::read_to_string(&launch.conversation_file).unwrap();
    for prompt in kept {
        assert!(copy.contains(prompt), "the copy keeps {prompt:?}:\n{copy}");
    }
    for prompt in cut {
        assert!(!copy.contains(prompt), "the copy stops before {prompt:?}:\n{copy}");
    }
    if agent == CODEX {
        assert!(in_own_home(&launch.conversation_file, &id.value, "codex/sessions"), "{}", launch.conversation_file.display());
        assert!(!copy.contains(&source.conversation), "the rollout names its new id only:\n{copy}");
    }
    if agent == COPILOT {
        assert!(in_own_home(&launch.conversation_file, &id.value, "copilot/session-state"), "{}", launch.conversation_file.display());
        assert!(!copy.contains(&source.conversation), "the session state names its new id only:\n{copy}");
    }
    fixture.assert_source_untouched(&source).await;
}

/// Whether `file` is under session `session`'s own composed config home
/// (`<machine root>/.sessions/<session>/config/<agent>`) at `under`, where
/// the agent keeps its conversations (A3f).
fn in_own_home(file: &std::path::Path, session: &str, under: &str) -> bool {
    let place = std::path::Path::new(".sessions").join(session).join("config").join(under);
    file.ancestors().any(|dir| dir.ends_with(&place))
}

/// A session of `agent` lists the three prompts as its turns, in order.
async fn lists_its_turns(agent: &str) {
    let fixture = Fixture::new().await;
    let source = fixture.source(agent).await;

    let turns = fixture.turns(&source.id).await;

    let want: Vec<(u32, String)> = (1..).zip(PROMPTS.map(str::to_owned)).collect();
    assert_eq!(turns, want);
}

/// A fork of a session of `agent` before turn 2 resumes a copy of its
/// conversation with turn 1 only, and the session is as it was.
async fn forks_before_turn_2(agent: &str) {
    let fixture = Fixture::new().await;
    let source = fixture.source(agent).await;

    let started = fixture.fork(&source.id, 2).await.unwrap();
    let done = finished(&fixture.daemon, &started).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
    let Some(pb::operation::Result::Session(id)) = done.result.clone() else {
        panic!("the fork's operation names its session: {done:?}");
    };
    let fork = wait_running(&fixture.daemon, &id).await;

    assert_eq!(fork.fork_origin.as_ref().map(|origin| origin.before_turn), Some(2), "{fork:?}");
    let launch = fixture.store().agent(&id.value).unwrap().expect("its agent is recorded").launch;
    assert_ne!(launch.conversation_file, source.file);
    let copy = std::fs::read_to_string(&launch.conversation_file).unwrap();
    assert!(copy.contains(PROMPTS[0]), "the fork keeps turn 1:\n{copy}");
    for prompt in &PROMPTS[1..] {
        assert!(!copy.contains(prompt), "the fork stops before turn 2:\n{copy}");
    }
    assert_eq!(fixture.turns(&id).await, [(1, PROMPTS[0].to_owned())]);
    assert_eq!(std::fs::read_to_string(&source.file).unwrap(), source.text);
}

/// A fork of `source` to another agent before turn 2 is handed a copy of
/// the conversation with turn 1 only, and the session is as it was.
/// Returns the fork, running.
async fn hands_over_before_turn_2(fixture: &Fixture, source: &Source) -> pb::Session {
    let started = fixture.fork_to(&source.id, 2, Some(CLAUDE)).await.unwrap();
    let done = finished(&fixture.daemon, &started).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{:?}", done.error);
    let Some(pb::operation::Result::Session(id)) = done.result.clone() else {
        panic!("the fork's operation names its session: {done:?}");
    };
    let fork = wait_running(&fixture.daemon, &id).await;

    assert_eq!(fork.agent.as_ref().map(|agent| agent.value.as_str()), Some(CLAUDE));
    let folder = fixture.store().session(&id.value).unwrap().expect("the store has the fork").folder;
    let handed = folder.join(".humata").join(format!("handover-{}.jsonl", id.value));
    let handed = std::fs::read_to_string(&handed).unwrap();
    assert!(handed.contains(PROMPTS[0]), "the handover keeps turn 1:\n{handed}");
    for prompt in &PROMPTS[1..] {
        assert!(!handed.contains(prompt), "the handover stops before turn 2:\n{handed}");
    }
    assert_eq!(std::fs::read_to_string(&source.file).unwrap(), source.text);
    fork
}

#[tokio::test]
async fn a_codex_session_handed_to_another_agent_before_turn_2_hands_over_turn_1_only() {
    let fixture = Fixture::new().await;
    let source = fixture.source(CODEX).await;
    hands_over_before_turn_2(&fixture, &source).await;
}

#[tokio::test]
async fn a_copilot_session_handed_to_another_agent_before_turn_2_hands_over_turn_1_only() {
    let fixture = Fixture::new().await;
    let source = fixture.source(COPILOT).await;
    hands_over_before_turn_2(&fixture, &source).await;
}

#[tokio::test]
async fn a_claude_session_continues_in_another_channel_from_now() {
    continues(CLAUDE, 0, &PROMPTS, &[]).await;
}

#[tokio::test]
async fn a_claude_session_continues_in_another_channel_from_before_turn_2() {
    continues(CLAUDE, 2, &PROMPTS[..1], &PROMPTS[1..]).await;
}

#[tokio::test]
async fn a_codex_session_continues_in_another_channel_from_now() {
    continues(CODEX, 0, &PROMPTS, &[]).await;
}

#[tokio::test]
async fn a_codex_session_continues_in_another_channel_from_before_turn_2() {
    continues(CODEX, 2, &PROMPTS[..1], &PROMPTS[1..]).await;
}

#[tokio::test]
async fn a_copilot_session_continues_in_another_channel_from_now() {
    continues(COPILOT, 0, &PROMPTS, &[]).await;
}

#[tokio::test]
async fn a_copilot_session_continues_in_another_channel_from_before_turn_2() {
    continues(COPILOT, 2, &PROMPTS[..1], &PROMPTS[1..]).await;
}

#[tokio::test]
async fn a_codex_session_lists_its_turns() {
    lists_its_turns(CODEX).await;
}

#[tokio::test]
async fn a_copilot_session_lists_its_turns() {
    lists_its_turns(COPILOT).await;
}

#[tokio::test]
async fn a_codex_fork_before_turn_2_resumes_with_turn_1_only() {
    forks_before_turn_2(CODEX).await;
}

#[tokio::test]
async fn a_copilot_fork_before_turn_2_resumes_with_turn_1_only() {
    forks_before_turn_2(COPILOT).await;
}

#[tokio::test]
async fn a_copilot_conversation_with_a_record_humata_doesnt_know_is_refused_and_nothing_changes() {
    let fixture = Fixture::new().await;
    let mut source = fixture.source(COPILOT).await;
    let unknown = serde_json::json!({
        "type": "session.mystery",
        "data": {},
        "id": "00000000-0000-4000-8000-0000000099ff",
        "timestamp": "2026-11-16T00:09:00.000Z",
        "parentId": null,
    });
    source.text.push_str(&format!("{unknown}\n"));
    std::fs::write(&source.file, &source.text).unwrap();
    let before = fixture.before().await;

    for before_turn in [0, 2] {
        let refused = fixture.continue_in(&source.id, fixture.billing_id(), before_turn).await.unwrap_err();

        assert_eq!(refused.code(), Code::FailedPrecondition, "{before_turn}: {refused:?}");
        assert!(
            matches!(error_of(&refused).kind, Some(pb::error::Kind::AgentLacksFeature(_))),
            "{before_turn}: {:?}",
            error_of(&refused)
        );
    }
    fixture.assert_nothing_new(&before).await;
    fixture.assert_source_untouched(&source).await;
}

/// Forks of `source` with its conversation, from now and before turn 2,
/// are each refused before anything is made, as the agent lacking a fork
/// with context, and the session is as it was.
async fn forks_are_refused_up_front(fixture: &Fixture, source: &Source) {
    let before = fixture.before().await;
    let payments = names(&fixture.sessions_dir(&fixture.payments));

    for before_turn in [0, 2] {
        let refused = fixture.fork(&source.id, before_turn).await.unwrap_err();

        assert_eq!(refused.code(), Code::FailedPrecondition, "{before_turn}: {refused:?}");
        assert!(
            matches!(error_of(&refused).kind, Some(pb::error::Kind::AgentLacksFeature(_))),
            "{before_turn}: {:?}",
            error_of(&refused)
        );
    }
    fixture.assert_nothing_new(&before).await;
    assert_eq!(names(&fixture.sessions_dir(&fixture.payments)), payments, "no fork folder is left");
    fixture.assert_source_untouched(source).await;
}

#[tokio::test]
async fn a_fork_of_a_copilot_conversation_with_a_record_humata_doesnt_know_is_refused_up_front() {
    let fixture = Fixture::new().await;
    let mut source = fixture.source(COPILOT).await;
    let unknown = serde_json::json!({
        "type": "session.mystery",
        "data": {},
        "id": "00000000-0000-4000-8000-0000000099fe",
        "timestamp": "2026-11-16T00:09:00.000Z",
        "parentId": null,
    });
    source.text.push_str(&format!("{unknown}\n"));
    std::fs::write(&source.file, &source.text).unwrap();

    forks_are_refused_up_front(&fixture, &source).await;
}

#[tokio::test]
async fn a_copilot_session_keeps_the_version_its_cli_said_at_launch() {
    let fixture = Fixture::new().await;
    std::fs::write(fixture.world.root.join(VERSION_FILE), "GitHub Copilot CLI 1.0.150-0.\n").unwrap();
    let source = fixture.source(COPILOT).await;

    let launch = fixture.store().agent(&source.id.value).unwrap().expect("its agent is recorded").launch;

    assert_eq!(launch.cli_version.as_deref(), Some("GitHub Copilot CLI 1.0.150-0."));
    let want: Vec<(u32, String)> = (1..).zip(PROMPTS.map(str::to_owned)).collect();
    assert_eq!(fixture.turns(&source.id).await, want);
}

#[tokio::test]
async fn a_fork_of_a_copilot_session_launched_by_a_cli_older_than_humata_knows_is_refused_up_front() {
    let fixture = Fixture::new().await;
    std::fs::write(fixture.world.root.join(VERSION_FILE), "GitHub Copilot CLI 1.0.100-0.\n").unwrap();
    let source = fixture.source(COPILOT).await;

    forks_are_refused_up_front(&fixture, &source).await;
}

#[tokio::test]
async fn a_channel_that_isnt_there_is_refused_and_nothing_changes() {
    let fixture = Fixture::new().await;
    let source = fixture.source(CLAUDE).await;
    let before = fixture.before().await;

    let missing = pb::ChannelId { value: "ch_0000000000000000000000000000dead".to_owned() };
    let refused = fixture.continue_in(&source.id, missing, 0).await.unwrap_err();

    assert_eq!(refused.code(), Code::NotFound, "{refused:?}");
    fixture.assert_nothing_new(&before).await;
    fixture.assert_source_untouched(&source).await;
}

#[tokio::test]
async fn a_deleted_channel_is_refused_and_nothing_changes() {
    let fixture = Fixture::new().await;
    let source = fixture.source(CLAUDE).await;
    let mut channels = fixture.daemon.channel_service().await;
    let plan = channels
        .plan_delete_channel(pb::PlanDeleteChannelRequest { channel: Some(fixture.billing_id()) })
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
    let done = finished(&fixture.daemon, deleted.operation.as_ref().unwrap()).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
    let before = fixture.before().await;

    let refused = fixture.continue_in(&source.id, fixture.billing_id(), 0).await.unwrap_err();

    assert_eq!(refused.code(), Code::NotFound, "{refused:?}");
    fixture.assert_nothing_new(&before).await;
    fixture.assert_source_untouched(&source).await;
}

#[tokio::test]
async fn an_archived_session_is_refused_and_nothing_changes() {
    let fixture = Fixture::new().await;
    let source = fixture.source(CLAUDE).await;
    let service = fixture.daemon.session_service().await;
    let plan = service
        .clone()
        .plan_archive(pb::PlanArchiveRequest { id: Some(source.id.clone()) })
        .await
        .unwrap()
        .into_inner();
    let started = service
        .clone()
        .archive_session(pb::ArchiveSessionRequest {
            idempotency_key: None,
            id: Some(source.id.clone()),
            choice: pb::ArchiveChoice::Archive as i32,
            plan: plan.token,
            end: Some(true),
        })
        .await
        .unwrap()
        .into_inner();
    let archived = finished(&fixture.daemon, &started).await;
    assert_eq!(archived.state, pb::OperationState::Succeeded as i32, "{archived:?}");
    let before = fixture.before().await;

    let refused = fixture.continue_in(&source.id, fixture.billing_id(), 0).await.unwrap_err();

    assert!(
        [Code::FailedPrecondition, Code::NotFound].contains(&refused.code()),
        "an archived session isn't continued: {refused:?}"
    );
    fixture.assert_nothing_new(&before).await;
    assert_eq!(std::fs::read_to_string(&source.file).unwrap(), source.text);
}

/// Cuts a continue at `point` and restarts: the half-made session is taken
/// back, its folder and conversation copy with it, and the source session
/// is as it was.
async fn cut_at(point: &str) {
    let mut fixture = Fixture::new().await;
    let source = fixture.source(CLAUDE).await;
    fixture.restart_with(&[("HUMATA2_STOP_AT", point)]).await;
    wait_running(&fixture.daemon, &source.id).await;
    let before = fixture.before().await;

    // The daemon dies part way, so the answer, if any, doesn't matter.
    let _ = fixture.continue_in(&source.id, fixture.billing_id(), 0).await;
    fixture.wait_dead().await;
    fixture.restart_with(&[]).await;

    fixture.assert_nothing_new(&before).await;
    wait_running(&fixture.daemon, &source.id).await;
    fixture.assert_source_untouched(&source).await;
}

#[tokio::test]
async fn a_continue_cut_once_its_records_are_saved_is_undone_at_restart() {
    cut_at("continue.write_record").await;
}

#[tokio::test]
async fn a_continue_cut_once_its_conversation_is_copied_is_undone_at_restart() {
    cut_at("continue.copy_conversation").await;
}

#[tokio::test]
async fn a_codex_session_handed_to_another_agent_before_turn_2_is_told_of_turn_1_only() {
    let fixture = Fixture::new().await;
    let source = fixture.source(CODEX).await;
    let fork = hands_over_before_turn_2(&fixture, &source).await;
    let id = fork.id.as_ref().unwrap();

    // The message names the handed file after its summary, so once that
    // name is echoed the summary is all there.
    let tab = agent_tab(&fixture.daemon, id).await;
    let mut attach = Attach::start(&fixture.daemon, open(&tab, 0)).await;
    attach.wait_for(&format!("handover-{}.jsonl", id.value)).await;
    assert!(attach.shows(PROMPTS[0]), "its first message summarizes turn 1");
    for prompt in &PROMPTS[1..] {
        assert!(!attach.shows(prompt), "its first message stops before turn 2");
    }
}

/// As [`cut_at`], with the stop at `point` a power cut: what the daemon
/// wrote in its state and channels folders and didn't sync is undone.
async fn power_cut_at(point: &str) {
    let mut fixture = Fixture::new().await;
    let source = fixture.source(CLAUDE).await;
    let roots = std::env::join_paths([fixture.daemon.state.clone(), fixture.world.channels.clone()])
        .expect("the folders join");
    let spare = fixture.home.parent().expect("the home is in the test folder").join("spare");
    let (roots, spare) = (roots.to_str().expect("UTF-8").to_owned(), spare.display().to_string());
    fixture
        .restart_with(&[
            ("HUMATA2_STOP_AT", point),
            ("HUMATA2_POWER_CUT_ROOTS", roots.as_str()),
            ("HUMATA2_POWER_CUT_SPARE", spare.as_str()),
        ])
        .await;
    wait_running(&fixture.daemon, &source.id).await;
    let before = fixture.before().await;

    // The daemon dies part way, so the answer, if any, doesn't matter.
    let _ = fixture.continue_in(&source.id, fixture.billing_id(), 0).await;
    fixture.wait_dead().await;
    fixture.restart_with(&[]).await;

    fixture.assert_nothing_new(&before).await;
    wait_running(&fixture.daemon, &source.id).await;
    fixture.assert_source_untouched(&source).await;
}

#[tokio::test]
async fn a_continue_cut_once_its_copy_is_noted_is_undone_at_restart() {
    cut_at("sessions.continue.noted").await;
}

#[tokio::test]
async fn a_continue_cut_by_a_power_cut_once_its_copy_is_noted_is_undone_at_restart() {
    power_cut_at("sessions.continue.noted").await;
}

#[tokio::test]
async fn a_continue_cut_by_a_power_cut_while_its_records_are_written_is_undone_at_restart() {
    power_cut_at("store.session.put_with_detail.committing").await;
}

#[tokio::test]
async fn a_continue_cut_by_a_power_cut_once_its_records_are_written_is_undone_at_restart() {
    power_cut_at("store.session.put_with_detail.committed").await;
}
