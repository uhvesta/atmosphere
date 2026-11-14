//! The Claude Code backend on the real Claude Code, pinned in
//! `//apps/humata/claude`, run offline against a fake Anthropic API.
//! `CLAUDE_CLI` is the path to the CLI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use humata_agent::{
    Access, Agent, ClaudeOptions, ClaudeRunner, Env, Event, EventKind, ForkOptions, LocalAgent,
    SendMode, SessionId, SessionState, StartOptions,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const TIMEOUT: Duration = Duration::from_secs(60);

/// What the fake model answers, in order; then "Done.".
#[derive(Clone)]
enum Reply {
    Text(&'static str),
    Shell(&'static str),
    /// Uses the CLI's tool of that name with that input.
    Tool(&'static str, Value),
    /// Never answers.
    Hang,
}

#[derive(Default)]
struct Model {
    replies: Mutex<Vec<Reply>>,
    /// Replies by the last user message's text, ahead of `replies`; for
    /// tests that mustn't depend on how many requests the CLI makes.
    answers: Vec<(&'static str, Reply)>,
    /// The messages of each streamed request, as JSON.
    requests: Mutex<Vec<String>>,
}

impl Model {
    fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn last_request(&self) -> String {
        self.requests().pop().unwrap_or_default()
    }

    fn next(&self, body: &Value) -> Reply {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(body["messages"].to_string());
        let last_user = body["messages"]
            .as_array()
            .and_then(|messages| {
                messages
                    .iter()
                    .rev()
                    .find(|message| message["role"] == "user")
            })
            .map(|message| message["content"].to_string())
            .unwrap_or_default();
        if let Some((_, reply)) = self
            .answers
            .iter()
            .find(|(text, _)| last_user.contains(text))
        {
            return reply.clone();
        }
        let mut replies = self.replies.lock().unwrap_or_else(PoisonError::into_inner);
        if replies.is_empty() {
            Reply::Text("Done.")
        } else {
            replies.remove(0)
        }
    }
}

/// The Anthropic Messages API on loopback; its base URL.
async fn serve(model: Arc<Model>) -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .expect("listen on loopback");
    let port = listener.local_addr().expect("a port").port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(connection(stream, model.clone()));
        }
    });
    format!("http://127.0.0.1:{port}")
}

async fn connection(stream: TcpStream, model: Arc<Model>) {
    let mut stream = BufReader::new(stream);
    loop {
        let mut request_line = String::new();
        if stream.read_line(&mut request_line).await.unwrap_or(0) == 0 {
            return;
        }
        let mut length = 0;
        loop {
            let mut line = String::new();
            if stream.read_line(&mut line).await.unwrap_or(0) == 0 {
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; length];
        if stream.read_exact(&mut body).await.is_err() {
            return;
        }
        let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        let (status, content_type, response) = if !request_line.starts_with("POST") {
            ("200 OK", "application/json", "{}".to_owned())
        } else if request_line.contains("count_tokens") {
            (
                "200 OK",
                "application/json",
                json!({"input_tokens": 10}).to_string(),
            )
        } else if body["stream"] != json!(true) {
            // Anything it asks on the side, such as a title.
            let message = json!({
                "id": "msg_side", "type": "message", "role": "assistant", "model": body["model"],
                "content": [{"type": "text", "text": "ok"}],
                "stop_reason": "end_turn", "stop_sequence": null,
                "usage": {"input_tokens": 1, "output_tokens": 1},
            });
            ("200 OK", "application/json", message.to_string())
        } else {
            match model.next(&body) {
                Reply::Hang => {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    return;
                }
                reply => (
                    "200 OK",
                    "text/event-stream",
                    stream_of(&reply, &body["model"]),
                ),
            }
        };
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
            response.len()
        );
        let stream = stream.get_mut();
        if stream.write_all(head.as_bytes()).await.is_err()
            || stream.write_all(response.as_bytes()).await.is_err()
        {
            return;
        }
    }
}

fn stream_of(reply: &Reply, model: &Value) -> String {
    let (block, deltas, stop) = match reply {
        Reply::Text(text) => (
            json!({"type": "text", "text": ""}),
            vec![json!({"type": "text_delta", "text": text})],
            "end_turn",
        ),
        Reply::Shell(command) => {
            let input = json!({"command": command, "description": "run a command"});
            (
                json!({"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {}}),
                vec![json!({"type": "input_json_delta", "partial_json": input.to_string()})],
                "tool_use",
            )
        }
        Reply::Tool(name, input) => (
            json!({"type": "tool_use", "id": "toolu_1", "name": name, "input": {}}),
            vec![json!({"type": "input_json_delta", "partial_json": input.to_string()})],
            "tool_use",
        ),
        Reply::Hang => return String::new(),
    };
    let mut events = vec![
        json!({"type": "message_start", "message": {
            "id": "msg_1", "type": "message", "role": "assistant", "model": model,
            "content": [], "stop_reason": null, "stop_sequence": null,
            "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0, "content_block": block}),
    ];
    events.extend(
        deltas
            .into_iter()
            .map(|delta| json!({"type": "content_block_delta", "index": 0, "delta": delta})),
    );
    events.extend([
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": stop, "stop_sequence": null},
               "usage": {"output_tokens": 3}}),
        json!({"type": "message_stop"}),
    ]);
    events
        .into_iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap_or("")
            )
        })
        .collect()
}

struct Setup {
    model: Arc<Model>,
    base_url: String,
    home: tempfile::TempDir,
    state: tempfile::TempDir,
    work: tempfile::TempDir,
}

impl Setup {
    async fn new(replies: Vec<Reply>) -> Self {
        Self::with_model(Model {
            replies: Mutex::new(replies),
            ..Model::default()
        })
        .await
    }

    async fn with_model(model: Model) -> Self {
        let model = Arc::new(model);
        let base_url = serve(model.clone()).await;
        let dir = || tempfile::tempdir().expect("a temporary folder");
        Self {
            model,
            base_url,
            home: dir(),
            state: dir(),
            work: dir(),
        }
    }

    /// An agent on a Claude Code of its own, sharing this setup's state.
    fn agent(&self) -> LocalAgent<ClaudeRunner> {
        let program = std::env::var_os("CLAUDE_CLI")
            .map(PathBuf::from)
            .expect("CLAUDE_CLI is the path to Claude Code");
        let home = self.home.path().to_string_lossy().into_owned();
        let runner = ClaudeRunner::new(ClaudeOptions {
            program: std::path::absolute(program).expect("an absolute path"),
            state: self.state.path().join("claude"),
            env: Env::new([
                ("HOME".into(), home.clone()),
                ("CLAUDE_CONFIG_DIR".into(), format!("{home}/claude")),
                ("ANTHROPIC_BASE_URL".into(), self.base_url.clone()),
                ("ANTHROPIC_API_KEY".into(), "sk-fake".into()),
                (
                    "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
                    "1".into(),
                ),
                ("DISABLE_AUTOUPDATER".into(), "1".into()),
            ]),
            mcp: None,
        });
        LocalAgent::open(self.state.path().join("sessions"), runner).expect("open the sessions")
    }

    /// Another host's setup, on the same model server.
    fn elsewhere(&self) -> Self {
        let dir = || tempfile::tempdir().expect("a temporary folder");
        Self {
            model: self.model.clone(),
            base_url: self.base_url.clone(),
            home: dir(),
            state: dir(),
            work: dir(),
        }
    }

    fn start_options(&self) -> StartOptions {
        StartOptions {
            name: "test".into(),
            working_directory: self.work.path().to_owned(),
            model: Some("fake".into()),
            ..StartOptions::default()
        }
    }
}

async fn wait_for(
    agent: &impl Agent,
    id: &SessionId,
    done: impl Fn(&[Event]) -> bool,
) -> Vec<Event> {
    let mut watch = agent.watch(id).await.expect("the session exists");
    let mut events = watch.history;
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while !done(&events) {
        match tokio::time::timeout_at(deadline, watch.live.recv()).await {
            Ok(Ok(event)) => events.push(event),
            Ok(Err(error)) => panic!("watch failed: {error}"),
            Err(_) => panic!("timed out; events so far: {events:#?}"),
        }
    }
    events
}

fn idle_turns(count: usize) -> impl Fn(&[Event]) -> bool {
    move |events| {
        events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::Idle))
            .count()
            >= count
    }
}

fn replies(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::AssistantMessage { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn failures(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            EventKind::Failed { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn request_id(events: &[Event]) -> String {
    events
        .iter()
        .rev()
        .find_map(|event| match &event.kind {
            EventKind::PermissionRequested { request_id, .. } => Some(request_id.clone()),
            _ => None,
        })
        .expect("a permission request")
}

fn event_with(events: &[Event], text: &str) -> Event {
    events
        .iter()
        .find(|event| matches!(&event.kind, EventKind::UserMessage { text: said, .. } if said == text))
        .cloned()
        .expect("the message is in the history")
}

#[tokio::test]
async fn a_message_is_answered() {
    let setup = Setup::new(vec![Reply::Text("Hello there.")]).await;
    let agent = setup.agent();
    let session = agent.start(setup.start_options()).await.expect("start");

    agent
        .send(&session.id, "apricot".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, idle_turns(1)).await;

    assert_eq!(failures(&events), Vec::<String>::new());
    assert_eq!(replies(&events), vec!["Hello there."]);
    assert!(setup.model.last_request().contains("apricot"));
}

#[tokio::test]
async fn a_project_turning_plugins_on_and_off_still_gets_answers() {
    let setup = Setup::new(vec![Reply::Text("Hello there.")]).await;
    let agent = setup.agent();
    let mut options = setup.start_options();
    options.plugins = [
        ("review@market".to_owned(), false),
        ("lint@market".to_owned(), true),
    ]
    .into();
    let session = agent.start(options).await.expect("start");

    agent
        .send(&session.id, "apricot".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, idle_turns(1)).await;

    assert_eq!(failures(&events), Vec::<String>::new());
    assert_eq!(replies(&events), vec!["Hello there."]);
}

#[tokio::test]
async fn an_approved_command_runs() {
    let setup = Setup::new(vec![Reply::Shell("echo hi > made.txt")]).await;
    let agent = setup.agent();
    let session = agent.start(setup.start_options()).await.expect("start");

    agent
        .send(&session.id, "make a file".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, |events| {
        SessionState::after(events) == SessionState::WaitingForApproval
    })
    .await;
    agent
        .approve(&session.id, &request_id(&events), true)
        .await
        .expect("approve");
    let events = wait_for(&agent, &session.id, idle_turns(1)).await;

    assert_eq!(failures(&events), Vec::<String>::new());
    assert!(setup.work.path().join("made.txt").is_file());
    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::ToolCompleted { success: true, .. }))
    );
}

#[tokio::test]
async fn a_denied_command_doesnt_run() {
    let setup = Setup::new(vec![Reply::Shell("echo hi > made.txt")]).await;
    let agent = setup.agent();
    let session = agent.start(setup.start_options()).await.expect("start");

    agent
        .send(&session.id, "make a file".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, |events| {
        SessionState::after(events) == SessionState::WaitingForApproval
    })
    .await;
    agent
        .approve(&session.id, &request_id(&events), false)
        .await
        .expect("deny");
    wait_for(&agent, &session.id, idle_turns(1)).await;

    assert!(!setup.work.path().join("made.txt").exists());
}

#[tokio::test]
async fn a_fork_forgets_what_came_after_its_event() {
    let setup = Setup::new(Vec::new()).await;
    let agent = setup.agent();
    let session = agent.start(setup.start_options()).await.expect("start");
    for (turn, text) in ["apricot", "banana"].into_iter().enumerate() {
        agent
            .send(&session.id, text.into(), SendMode::Enqueue)
            .await
            .expect("send");
        wait_for(&agent, &session.id, idle_turns(turn + 1)).await;
    }
    let history = agent.history(&session.id).await.expect("history");

    let fork = agent
        .fork(
            &session.id,
            ForkOptions {
                before: Some(event_with(&history, "banana").id),
                ..ForkOptions::default()
            },
        )
        .await
        .expect("fork");
    agent
        .send(&fork.id, "cherry".into(), SendMode::Enqueue)
        .await
        .expect("send to the fork");
    let events = wait_for(&agent, &fork.id, idle_turns(2)).await;

    assert_eq!(failures(&events), Vec::<String>::new());
    let seen = setup.model.last_request();
    assert!(seen.contains("apricot") && seen.contains("cherry"));
    assert!(!seen.contains("banana"));

    // The parent still has all of it.
    agent
        .send(&session.id, "damson".into(), SendMode::Enqueue)
        .await
        .expect("send to the parent");
    wait_for(&agent, &session.id, idle_turns(3)).await;
    let seen = setup.model.last_request();
    assert!(seen.contains("banana") && seen.contains("damson"));
    assert!(!seen.contains("cherry"));
}

#[tokio::test]
async fn a_session_restored_on_another_host_carries_on_with_its_context() {
    let setup = Setup::new(Vec::new()).await;
    let agent = setup.agent();
    let session = agent.start(setup.start_options()).await.expect("start");
    agent
        .send(&session.id, "apricot".into(), SendMode::Enqueue)
        .await
        .expect("send");
    wait_for(&agent, &session.id, idle_turns(1)).await;
    let backup = agent.backup(&session.id).await.expect("backed up");
    drop(agent);

    let elsewhere = setup.elsewhere();
    let agent = elsewhere.agent();
    agent.restore(backup, Vec::new()).await.expect("restored");
    agent
        .send(&session.id, "banana".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, idle_turns(2)).await;
    assert_eq!(failures(&events), Vec::<String>::new());
    let seen = setup.model.last_request();
    assert!(seen.contains("apricot") && seen.contains("banana"));
}

#[tokio::test]
async fn sessions_carry_on_after_a_restart() {
    let setup = Setup::new(Vec::new()).await;
    let session = {
        let agent = setup.agent();
        let session = agent.start(setup.start_options()).await.expect("start");
        agent
            .send(&session.id, "apricot".into(), SendMode::Enqueue)
            .await
            .expect("send");
        wait_for(&agent, &session.id, idle_turns(1)).await;
        session
    };

    let agent = setup.agent();
    let fork = agent
        .fork(&session.id, ForkOptions::default())
        .await
        .expect("fork");
    agent
        .send(&fork.id, "banana".into(), SendMode::Enqueue)
        .await
        .expect("send to the fork");
    let events = wait_for(&agent, &fork.id, idle_turns(2)).await;
    assert_eq!(failures(&events), Vec::<String>::new());
    let seen = setup.model.last_request();
    assert!(seen.contains("apricot") && seen.contains("banana"));

    agent
        .send(&session.id, "cherry".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, idle_turns(2)).await;
    assert_eq!(failures(&events), Vec::<String>::new());
    let seen = setup.model.last_request();
    assert!(seen.contains("apricot") && seen.contains("cherry"));
    assert!(!seen.contains("banana"));
}

#[tokio::test]
async fn an_aborted_turn_stops_and_the_next_one_runs() {
    let setup = Setup::with_model(Model {
        answers: vec![("apricot", Reply::Hang), ("banana", Reply::Text("Back."))],
        ..Model::default()
    })
    .await;
    let agent = setup.agent();
    let session = agent.start(setup.start_options()).await.expect("start");

    agent
        .send(&session.id, "apricot".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let deadline = tokio::time::Instant::now() + TIMEOUT;
    while setup.model.requests().is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the model was never asked"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    agent.abort(&session.id).await.expect("abort");
    agent
        .send(&session.id, "banana".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, idle_turns(2)).await;

    assert!(
        events
            .iter()
            .any(|event| matches!(event.kind, EventKind::TurnEnded { aborted: true }))
    );
    assert_eq!(replies(&events), vec!["Back."], "{events:#?}");
}

#[tokio::test]
async fn a_message_sent_during_a_turn_steers_it() {
    let setup = Setup::new(vec![Reply::Shell("echo hi > made.txt")]).await;
    let agent = setup.agent();
    let session = agent.start(setup.start_options()).await.expect("start");

    agent
        .send(&session.id, "make a file".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, |events| {
        SessionState::after(events) == SessionState::WaitingForApproval
    })
    .await;
    agent
        .send(&session.id, "elderberry".into(), SendMode::Immediate)
        .await
        .expect("steer");
    tokio::time::sleep(Duration::from_millis(500)).await;
    agent
        .approve(&session.id, &request_id(&events), true)
        .await
        .expect("approve");
    let events = wait_for(&agent, &session.id, idle_turns(1)).await;

    assert_eq!(failures(&events), Vec::<String>::new());
    assert!(setup.model.last_request().contains("elderberry"));
}

#[tokio::test]
async fn a_session_whose_conversation_claude_code_lost_fails_without_running_it() {
    let setup = Setup::new(vec![Reply::Text("First.")]).await;
    let agent = setup.agent();
    let session = agent.start(setup.start_options()).await.expect("start");
    agent
        .send(&session.id, "one".into(), SendMode::Enqueue)
        .await
        .expect("send");
    wait_for(&agent, &session.id, idle_turns(1)).await;
    let asked = setup.model.requests().len();

    // As Claude Code's clean-up of old sessions would.
    let projects = setup.home.path().join("claude/projects");
    for project in std::fs::read_dir(&projects).expect("projects") {
        std::fs::remove_dir_all(project.expect("a project").path()).expect("removed");
    }
    agent
        .send(&session.id, "two".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, idle_turns(2)).await;

    assert_eq!(failures(&events).len(), 1);
    assert_eq!(
        setup.model.requests().len(),
        asked,
        "Claude Code wasn't run"
    );
}

#[tokio::test]
async fn a_planning_session_never_runs_monitor_or_is_asked_about_it() {
    let setup = Setup::new(vec![Reply::Tool(
        "Monitor",
        json!({"command": "echo hi > made.txt", "description": "watch a command"}),
    )])
    .await;
    let agent = setup.agent();
    let options = StartOptions {
        access: Access::Plan,
        ..setup.start_options()
    };
    let session = agent.start(options).await.expect("start");

    agent
        .send(&session.id, "watch something".into(), SendMode::Enqueue)
        .await
        .expect("send");
    let events = wait_for(&agent, &session.id, idle_turns(1)).await;

    assert!(
        !events
            .iter()
            .any(|event| matches!(event.kind, EventKind::PermissionRequested { .. })),
        "{events:#?}"
    );
    assert!(!setup.work.path().join("made.txt").exists());
}
