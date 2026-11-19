//! A session follows its agent into the conversation its CLI is in now,
//! through a real `humatad2`: Copilot's `/clear` and `/restart` start
//! another conversation in the same running agent, and the hook the daemon
//! wrote into Copilot's plugin names it from the payload on its stdin. The
//! session's conversation becomes that one, a Stop then Resume takes it up,
//! a conversation not written yet is passed over until a later hook names
//! it again, and another session's agent is left where it is. Claude
//! Code's `/clear` and Codex's `/new` are followed the same way, from the
//! `session_id` their hooks (Claude) and wrapper (Codex) name (D799).

use humatad_test_common as common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{
    Attach, Build, Daemon, WAIT, World, agent_tab, binary, create_channel, finished, folder,
    get_session, is_running, open, shell_token, start_session, wait_for_path, wait_running,
};
use humata2_grpc::humata::v1 as pb;
use serde_json::Value;

/// The conversation Copilot moved to after `/clear`, as its id looks.
const CLEARED: &str = "f78588d5-b8ac-40f0-8448-6af7d59c8a9c";

struct Fixture {
    daemon: Daemon,
    world: World,
    channel: pb::Channel,
}

impl Fixture {
    async fn new() -> Self {
        let dir = folder();
        let world = World::new(dir.path());
        world.remote("acme", "api");
        let mut args = world.args();
        args.extend(["--humata".to_owned(), binary("HUMATA").display().to_string()]);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let daemon = Daemon::start_with(Build::Admin, dir, &refs).await;
        let (channel, setup) = create_channel(&daemon, "Payments", &[("acme", "api")]).await;
        assert_eq!(setup.state, pb::OperationState::Succeeded as i32, "{:?}", setup.error);
        Self {
            daemon,
            world,
            channel,
        }
    }

    /// A session whose agent runs, and its conversation.
    async fn session(&self, title: &str) -> (pb::SessionId, String) {
        let (id, operation) = start_session(&self.daemon, &self.channel, title).await;
        assert_eq!(operation.state, pb::OperationState::Succeeded as i32, "{:?}", operation.error);
        let running = wait_running(&self.daemon, &id).await;
        assert!(!running.conversation_id.is_empty());
        (id, running.conversation_id)
    }

    /// The command Copilot runs for `event` to name its conversation, as
    /// the daemon wrote it into the plugin.
    fn conversation_hook(&self, event: &str) -> String {
        let file = self.daemon.state.join("copilot-plugin").join("hooks.json");
        let text = std::fs::read_to_string(&file).expect("read the Copilot plugin's hooks.json");
        let hooks: Value = serde_json::from_str(&text).expect("hooks.json is JSON");
        hooks["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("no {event} hooks"))
            .iter()
            .filter_map(|hook| hook["bash"].as_str())
            .find(|bash| bash.contains("conversation"))
            .unwrap_or_else(|| panic!("no {event} hook names the conversation"))
            .to_owned()
    }

    /// The command Claude Code runs for `event` to name its conversation,
    /// as the daemon wrote it into the hooks file Claude is given.
    fn claude_conversation_hook(&self, event: &str) -> String {
        let file = self.daemon.state.join("claude-hooks.json");
        let text = std::fs::read_to_string(&file).expect("read Claude's hooks file");
        let hooks: Value = serde_json::from_str(&text).expect("claude-hooks.json is JSON");
        hooks["hooks"][event]
            .as_array()
            .unwrap_or_else(|| panic!("no {event} hooks"))
            .iter()
            .filter_map(|group| group["hooks"].as_array())
            .flatten()
            .filter_map(|hook| hook["command"].as_str())
            .find(|command| command.contains("conversation"))
            .unwrap_or_else(|| panic!("no {event} hook names the conversation"))
            .to_owned()
    }

    /// Copilot, in the session's terminal, runs its `event` hook with a
    /// payload naming conversation `id`; returns once the hook has exited.
    async fn hook_names(&self, attach: &mut Attach, event: &str, id: &str, name: &str) {
        let payload = serde_json::json!({
            "sessionId": id,
            "timestamp": 1_795_016_795_934_i64,
            "cwd": "/work/api",
            "source": "new",
        });
        let command = self.conversation_hook(event);
        self.run_hook(attach, &command, &payload, name).await;
    }

    /// Claude Code, in the session's terminal, runs its `event` hook with
    /// the payload it sends after `/clear`, naming conversation `id`.
    async fn claude_names(&self, attach: &mut Attach, event: &str, id: &str, name: &str) {
        let payload = serde_json::json!({
            "session_id": id,
            "transcript_path": format!("/home/u/.claude/projects/-work-api/{id}.jsonl"),
            "cwd": "/work/api",
            "hook_event_name": event,
            "source": "clear",
        });
        let command = self.claude_conversation_hook(event);
        self.run_hook(attach, &command, &payload, name).await;
    }

    /// The `humata codex` wrapper, in the session's terminal, names thread
    /// `id` as Codex's app-server announced it after `/new`; it reaches the
    /// daemon as `humata hook codex conversation` does.
    async fn codex_names(&self, attach: &mut Attach, id: &str, name: &str) {
        let payload = serde_json::json!({ "session_id": id, "source": "thread/started" });
        let command = self
            .claude_conversation_hook("SessionStart")
            .replace("hook claude conversation", "hook codex conversation");
        self.run_hook(attach, &command, &payload, name).await;
    }

    async fn run_hook(&self, attach: &mut Attach, command: &str, payload: &Value, name: &str) {
        let input = self.daemon.dir.path().join(format!("{name}.in"));
        std::fs::write(&input, payload.to_string()).expect("write a test file");
        let done = self.daemon.dir.path().join(format!("{name}.done"));
        attach
            .type_text(&format!(
                "{{ {command}; }} < '{}'; echo $? > '{}'\r",
                input.display(),
                done.display(),
            ))
            .await;
        wait_for_path(&done).await;
    }

    /// Copilot writes conversation `id`'s file, as it does by the end of
    /// the conversation's first turn.
    fn writes(&self, id: &str) -> PathBuf {
        let file = self.world.conversation(id);
        std::fs::write(&file, "{\"type\":\"session.start\"}\n").expect("write a test file");
        file
    }

    async fn conversation(&self, id: &pb::SessionId) -> String {
        get_session(&self.daemon, id).await.conversation_id
    }
}

#[tokio::test]
async fn a_cleared_conversation_is_followed_and_resume_takes_it_up() {
    let fix = Fixture::new().await;
    let (id, before) = fix.session("Fix login").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &id).await;

    fix.writes(CLEARED);
    fix.hook_names(&mut shell, "sessionStart", CLEARED, "start").await;
    assert_eq!(fix.conversation(&id).await, CLEARED, "the session stayed in {before}");
    drop(shell);

    let stopped = fix
        .daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            for_move: false,
        })
        .await
        .expect("the session stops")
        .into_inner();
    assert_eq!(stopped.conversation_id, CLEARED);
    let deadline = Instant::now() + WAIT;
    while is_running(&get_session(&fix.daemon, &id).await) {
        assert!(Instant::now() < deadline, "the stopped session still runs");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let resumed = fix
        .daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .expect("the resume is taken")
        .into_inner();
    let done = finished(&fix.daemon, &resumed).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
    assert_eq!(wait_running(&fix.daemon, &id).await.conversation_id, CLEARED);

    // The resumed agent is in the cleared conversation: what it is told
    // goes there, and the one from before the clear is left as it was.
    let mut agent = Attach::start(&fix.daemon, open(&agent_tab(&fix.daemon, &id).await, 0)).await;
    agent.type_text("after-resume\r").await;
    agent.wait_for("got: after-resume").await;
    let cleared = std::fs::read_to_string(fix.world.conversation(CLEARED)).expect("read");
    assert!(cleared.contains("after-resume"), "the resume took up another conversation");
    let old = std::fs::read_to_string(fix.world.conversation(&before)).unwrap_or_default();
    assert!(!old.contains("after-resume"), "the resume took up the conversation from before the clear");
}

#[tokio::test]
async fn a_conversation_not_written_yet_is_followed_once_a_later_hook_names_it() {
    let fix = Fixture::new().await;
    let (id, before) = fix.session("Fix login").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &id).await;

    // Right after `/clear` the new conversation has no file to resume yet.
    fix.hook_names(&mut shell, "sessionStart", CLEARED, "start").await;
    assert_eq!(fix.conversation(&id).await, before);

    // Its first turn writes it, and the turn's end names it again.
    fix.writes(CLEARED);
    fix.hook_names(&mut shell, "agentStop", CLEARED, "stop").await;
    assert_eq!(fix.conversation(&id).await, CLEARED);
}

#[tokio::test]
async fn only_the_session_whose_agent_moved_follows() {
    let fix = Fixture::new().await;
    let (moved, _) = fix.session("Fix login").await;
    let (stayed, stayed_in) = fix.session("Fix signup").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &moved).await;

    fix.writes(CLEARED);
    fix.hook_names(&mut shell, "userPromptSubmitted", CLEARED, "prompt").await;
    assert_eq!(fix.conversation(&moved).await, CLEARED);
    assert_eq!(fix.conversation(&stayed).await, stayed_in);

    // Another session's agent never takes a conversation one already holds.
    let (_other, mut other_shell) = shell_token(&fix.daemon, &stayed).await;
    fix.hook_names(&mut other_shell, "userPromptSubmitted", CLEARED, "other").await;
    assert_eq!(fix.conversation(&stayed).await, stayed_in);
    assert_eq!(fix.conversation(&moved).await, CLEARED);
}

/// Stops session `id` and resumes it; returns the conversation it runs in.
async fn stop_and_resume(fix: &Fixture, id: &pb::SessionId) -> String {
    fix.daemon
        .session_service()
        .await
        .stop_session(pb::StopSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            for_move: false,
        })
        .await
        .expect("the session stops");
    let deadline = Instant::now() + WAIT;
    while is_running(&get_session(&fix.daemon, id).await) {
        assert!(Instant::now() < deadline, "the stopped session still runs");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let resumed = fix
        .daemon
        .session_service()
        .await
        .resume_session(pb::ResumeSessionRequest {
            idempotency_key: None,
            id: Some(id.clone()),
            ..Default::default()
        })
        .await
        .expect("the resume is taken")
        .into_inner();
    let done = finished(&fix.daemon, &resumed).await;
    assert_eq!(done.state, pb::OperationState::Succeeded as i32, "{done:?}");
    wait_running(&fix.daemon, id).await.conversation_id
}

#[tokio::test]
async fn a_claude_conversation_cleared_is_followed_and_resume_takes_it_up() {
    let fix = Fixture::new().await;
    let (id, before) = fix.session("Fix login").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &id).await;

    fix.writes(CLEARED);
    fix.claude_names(&mut shell, "SessionStart", CLEARED, "start").await;
    assert_eq!(fix.conversation(&id).await, CLEARED, "the session stayed in {before}");
    drop(shell);
    assert_eq!(stop_and_resume(&fix, &id).await, CLEARED);
}

#[tokio::test]
async fn a_claude_conversation_not_written_at_clear_is_followed_at_the_turns_end() {
    let fix = Fixture::new().await;
    let (id, before) = fix.session("Fix login").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &id).await;

    fix.claude_names(&mut shell, "SessionStart", CLEARED, "start").await;
    assert_eq!(fix.conversation(&id).await, before);
    fix.writes(CLEARED);
    fix.claude_names(&mut shell, "Stop", CLEARED, "stop").await;
    assert_eq!(fix.conversation(&id).await, CLEARED);
}

#[tokio::test]
async fn claude_names_its_conversation_at_each_prompt_too() {
    let fix = Fixture::new().await;
    let (id, _) = fix.session("Fix login").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &id).await;
    fix.writes(CLEARED);
    fix.claude_names(&mut shell, "UserPromptSubmit", CLEARED, "prompt").await;
    assert_eq!(fix.conversation(&id).await, CLEARED);
}

#[tokio::test]
async fn a_codex_thread_started_by_new_is_followed_and_resume_takes_it_up() {
    let fix = Fixture::new().await;
    let (id, before) = fix.session("Fix login").await;
    let (_token, mut shell) = shell_token(&fix.daemon, &id).await;

    fix.writes(CLEARED);
    fix.codex_names(&mut shell, CLEARED, "thread").await;
    assert_eq!(fix.conversation(&id).await, CLEARED, "the session stayed in {before}");
    drop(shell);
    assert_eq!(stop_and_resume(&fix, &id).await, CLEARED);
}
