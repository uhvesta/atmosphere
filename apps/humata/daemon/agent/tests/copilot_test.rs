//! The Copilot backend on the real Copilot CLI, pinned in
//! `//apps/humata/copilot`, run offline against a fake model server.
//! `COPILOT_CLI` is the path to the CLI.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use humata_agent::{
    Agent, CopilotOptions, CopilotProvider, CopilotRunner, Env, Event, EventKind, ForkOptions,
    LocalAgent, SendMode, SessionId, SessionState, StartOptions,
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
    /// Never answers.
    Hang,
}

#[derive(Default)]
struct Model {
    replies: Mutex<Vec<Reply>>,
    /// Replies by the last user message's text, ahead of `replies`; for
    /// tests that mustn't depend on how many requests the CLI makes.
    answers: Vec<(&'static str, Reply)>,
    /// The messages of each chat request, as JSON.
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
        let messages = body["messages"].to_string();
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(messages);
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

/// An OpenAI-compatible server on loopback; its base URL.
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
    format!("http://127.0.0.1:{port}/v1")
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
        let (content_type, response) = if request_line.starts_with("GET") {
            let models = json!({"object": "list", "data": [{"id": "fake", "object": "model"}]});
            ("application/json", models.to_string())
        } else {
            let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            match model.next(&body) {
                Reply::Hang => {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    return;
                }
                reply => ("text/event-stream", stream_of(&reply)),
            }
        };
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\r\n",
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

fn stream_of(reply: &Reply) -> String {
    let chunks = match reply {
        Reply::Text(text) => vec![
            json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": text}}]}),
            json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
                   "usage": {"prompt_tokens": 10, "completion_tokens": 2, "total_tokens": 12}}),
        ],
        Reply::Shell(command) => {
            let arguments = json!({"command": command, "description": "run a command"});
            vec![
                json!({"choices": [{"index": 0, "delta": {"role": "assistant", "tool_calls": [
                    {"index": 0, "id": "call_1", "type": "function",
                     "function": {"name": "bash", "arguments": arguments.to_string()}}]}}]}),
                json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
            ]
        }
        Reply::Hang => Vec::new(),
    };
    let mut body = String::new();
    for mut chunk in chunks {
        chunk["id"] = json!("x");
        chunk["object"] = json!("chat.completion.chunk");
        chunk["model"] = json!("fake");
        body.push_str(&format!("data: {chunk}\n\n"));
    }
    body.push_str("data: [DONE]\n\n");
    body
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

    /// An agent on a Copilot CLI of its own, sharing this setup's state.
    fn agent(&self) -> LocalAgent<CopilotRunner> {
        let program = std::env::var_os("COPILOT_CLI")
            .map(PathBuf::from)
            .expect("COPILOT_CLI is the path to the Copilot CLI");
        let home = self.home.path().to_string_lossy().into_owned();
        let runner = CopilotRunner::new(CopilotOptions {
            program: std::path::absolute(program).expect("an absolute path"),
            state: self.state.path().join("copilot"),
            env: Env::new([
                ("HOME".into(), home.clone()),
                ("COPILOT_HOME".into(), format!("{home}/copilot")),
                // Offline, it makes no network calls but the model's, and
                // won't start without a model server and model.
                ("COPILOT_OFFLINE".into(), "true".into()),
                ("COPILOT_PROVIDER_BASE_URL".into(), self.base_url.clone()),
                ("COPILOT_MODEL".into(), "fake".into()),
            ]),
            provider: Some(CopilotProvider {
                kind: "openai".into(),
                base_url: self.base_url.clone(),
                api_key: None,
            }),
            mcp: None,
        });
        LocalAgent::open(self.state.path().join("sessions"), runner).expect("open the sessions")
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
        .find(|event| matches!(&event.kind, EventKind::UserMessage { text: said } if said == text))
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
