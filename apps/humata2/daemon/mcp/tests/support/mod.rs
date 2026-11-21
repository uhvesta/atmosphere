//! A fake daemon and an in-memory MCP client for the server's tests.
//!
//! The fake decides each tool's outcome from a table the test sets (Allow
//! unless told otherwise), keeps one reply per idempotency key as the real
//! daemon does, and records every bind, call and withdrawal. Waits are
//! handed to the test, which answers them when it chooses, so no test
//! sleeps.

#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use humata2_mcp::{
    Call, Daemon, ErrorKind, Reply, RequestId, Retry, ServeError, Server, SessionToken,
    TokenError, ToolError, WaitEvent,
};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// What the fake daemon does with a tool's call.
#[derive(Debug, Clone)]
pub enum Rule {
    /// Runs it: `applied`, with `{"tool": <name>}` as the result.
    Allow,
    /// Makes an Inbox request: `pending`.
    Ask,
    /// Refuses it with this error.
    Refuse(ToolError),
    /// Answers with this reply.
    Reply(Reply),
}

/// A wait the server started, for the test to answer.
pub struct Wait {
    pub request: RequestId,
    pub wait: Duration,
    pub events: mpsc::Sender<WaitEvent>,
}

#[derive(Clone)]
pub struct FakeDaemon {
    inner: Arc<Inner>,
}

struct Inner {
    rules: Mutex<HashMap<String, Rule>>,
    bind_results: Mutex<VecDeque<Result<(), ToolError>>>,
    binds: Mutex<Vec<String>>,
    calls: Mutex<Vec<Call>>,
    replies: Mutex<HashMap<String, Result<Reply, ToolError>>>,
    withdrawals: Mutex<Vec<RequestId>>,
    waits_sender: mpsc::UnboundedSender<Wait>,
    waits: tokio::sync::Mutex<mpsc::UnboundedReceiver<Wait>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeDaemon {
    pub fn new() -> Self {
        let (waits_sender, waits) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Inner {
                rules: Mutex::new(HashMap::new()),
                bind_results: Mutex::new(VecDeque::new()),
                binds: Mutex::new(Vec::new()),
                calls: Mutex::new(Vec::new()),
                replies: Mutex::new(HashMap::new()),
                withdrawals: Mutex::new(Vec::new()),
                waits_sender,
                waits: tokio::sync::Mutex::new(waits),
            }),
        }
    }

    pub fn set_rule(&self, tool: &str, rule: Rule) {
        lock(&self.inner.rules).insert(tool.to_owned(), rule);
    }

    /// The next binds return these, in order; after them, binds succeed.
    pub fn fail_binds(&self, results: impl IntoIterator<Item = ToolError>) {
        lock(&self.inner.bind_results).extend(results.into_iter().map(Err));
    }

    /// The tokens presented so far.
    pub fn binds(&self) -> Vec<String> {
        lock(&self.inner.binds).clone()
    }

    pub fn calls(&self) -> Vec<Call> {
        lock(&self.inner.calls).clone()
    }

    pub fn withdrawals(&self) -> Vec<RequestId> {
        lock(&self.inner.withdrawals).clone()
    }

    /// The next wait the server starts.
    pub async fn next_wait(&self) -> Wait {
        let mut waits = self.inner.waits.lock().await;
        within(waits.recv()).await.expect("the fake daemon is alive")
    }
}

impl Daemon for FakeDaemon {
    async fn bind(&self, token: &SessionToken) -> Result<(), ToolError> {
        lock(&self.inner.binds).push(token.expose().to_owned());
        lock(&self.inner.bind_results).pop_front().unwrap_or(Ok(()))
    }

    async fn call(&self, call: Call) -> Result<Reply, ToolError> {
        let key = call.idempotency_key.as_str().to_owned();
        let tool = call.operation.tool_name();
        lock(&self.inner.calls).push(call);
        let mut replies = lock(&self.inner.replies);
        if let Some(reply) = replies.get(&key) {
            return reply.clone();
        }
        let rule = lock(&self.inner.rules)
            .get(tool)
            .cloned()
            .unwrap_or(Rule::Allow);
        let reply = match rule {
            Rule::Allow => Ok(Reply::Applied {
                result: json!({ "tool": tool }),
                request: None,
                stale_as_of: None,
            }),
            Rule::Ask => Ok(Reply::Pending {
                request: RequestId::new(format!("req-{}", replies.len() + 1)),
            }),
            Rule::Refuse(error) => Err(error),
            Rule::Reply(reply) => Ok(reply),
        };
        replies.insert(key, reply.clone());
        reply
    }

    async fn wait(
        &self,
        request: &RequestId,
        wait: Duration,
    ) -> Result<mpsc::Receiver<WaitEvent>, ToolError> {
        let (events, receiver) = mpsc::channel(16);
        let _ = self.inner.waits_sender.send(Wait {
            request: request.clone(),
            wait,
            events,
        });
        Ok(receiver)
    }

    async fn withdraw(&self, request: &RequestId) -> Result<(), ToolError> {
        lock(&self.inner.withdrawals).push(request.clone());
        Ok(())
    }
}

/// An error as the daemon would send it.
pub fn tool_error(kind: ErrorKind, details: Value) -> ToolError {
    ToolError {
        kind,
        message: "refused".to_owned(),
        fix: "do something else".to_owned(),
        retry: None,
        details: match details {
            Value::Object(details) => details,
            _ => Map::new(),
        },
    }
}

pub fn retryable(mut error: ToolError, after: Option<SystemTime>) -> ToolError {
    error.retry = Some(Retry {
        retryable: true,
        after,
        same_key: true,
    });
    error
}

/// Fails a test that would otherwise hang. Nothing in the tests waits on
/// time; this only bounds a bug.
pub async fn within<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(30), future)
        .await
        .expect("the server answered within 30 seconds")
}

/// An agent's MCP client, talking to a server over in-memory pipes.
pub struct Client {
    to_server: Option<DuplexStream>,
    from_server: Lines<BufReader<DuplexStream>>,
    server: JoinHandle<Result<(), ServeError>>,
    /// Notifications read while looking for a reply.
    pub notifications: Vec<Value>,
}

impl Client {
    /// A client for a session whose token is `token-1`.
    pub fn start(daemon: &FakeDaemon) -> Self {
        Self::start_with(daemon, SessionToken::new("token-1"))
    }

    pub fn start_with(daemon: &FakeDaemon, token: Result<SessionToken, TokenError>) -> Self {
        Self::start_reading(daemon, token, 1 << 20)
    }

    /// A client that takes at most `buffer` bytes of the server's output
    /// before it reads, as an agent that is busy elsewhere.
    pub fn start_slow(daemon: &FakeDaemon, buffer: usize) -> Self {
        Self::start_reading(daemon, SessionToken::new("token-1"), buffer)
    }

    fn start_reading(
        daemon: &FakeDaemon,
        token: Result<SessionToken, TokenError>,
        buffer: usize,
    ) -> Self {
        let (to_server, server_input) = tokio::io::duplex(1 << 20);
        let (server_output, from_server) = tokio::io::duplex(buffer);
        let server = Server::new(daemon.clone(), token);
        let server = tokio::spawn(server.serve(server_input, server_output));
        Self {
            to_server: Some(to_server),
            from_server: BufReader::new(from_server).lines(),
            server,
            notifications: Vec::new(),
        }
    }

    pub async fn send_line(&mut self, line: &str) {
        let to_server = self.to_server.as_mut().expect("the client is open");
        to_server
            .write_all(format!("{line}\n").as_bytes())
            .await
            .expect("the server reads");
    }

    pub async fn send(&mut self, message: Value) {
        self.send_line(&message.to_string()).await;
    }

    /// The next message from the server.
    pub async fn receive(&mut self) -> Option<Value> {
        let line = within(self.from_server.next_line())
            .await
            .expect("the server writes lines")?;
        Some(serde_json::from_str(&line).expect("the server writes JSON"))
    }

    /// Reads until the reply to `id`, keeping notifications.
    pub async fn reply_to(&mut self, id: &Value) -> Value {
        loop {
            let message = self.receive().await.expect("the server replies");
            if message.get("id") == Some(id) && message.get("method").is_none() {
                return message;
            }
            self.notifications.push(message);
        }
    }

    pub async fn request(&mut self, id: Value, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        self.reply_to(&id).await
    }

    /// `tools/call`, returning the JSON-RPC reply.
    pub async fn call_raw(&mut self, id: Value, tool: &str, arguments: Value) -> Value {
        self.request(id, "tools/call", json!({ "name": tool, "arguments": arguments }))
            .await
    }

    /// `tools/call`, returning the tool result.
    pub async fn call(&mut self, id: Value, tool: &str, arguments: Value) -> Value {
        let reply = self.call_raw(id, tool, arguments).await;
        reply
            .get("result")
            .cloned()
            .unwrap_or_else(|| panic!("{tool} returned a tool result, got {reply}"))
    }

    /// Sends `tools/call` without waiting for its reply.
    pub async fn start_call(&mut self, id: Value, tool: &str, params: Value) {
        let mut params = match params {
            Value::Object(params) => params,
            _ => Map::new(),
        };
        params.insert("name".to_owned(), json!(tool));
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params }))
            .await;
    }

    pub async fn tools(&mut self) -> Vec<Value> {
        let reply = self.request(json!("list"), "tools/list", json!({})).await;
        reply["result"]["tools"]
            .as_array()
            .cloned()
            .expect("tools/list returns tools")
    }

    pub async fn humata_list(&mut self, arguments: Value) -> Value {
        let result = self.call(json!("hlist"), "humata_list", arguments).await;
        assert_eq!(result["isError"], false, "{result}");
        result["structuredContent"].clone()
    }

    pub async fn humata_describe(&mut self, names: &[&str]) -> Value {
        let result = self
            .call(json!("hdesc"), "humata_describe", json!({ "names": names }))
            .await;
        assert_eq!(result["isError"], false, "{result}");
        result["structuredContent"].clone()
    }

    pub async fn humata_call_raw(&mut self, id: Value, name: &str, arguments: Value) -> Value {
        self.call_raw(
            id,
            "humata_call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    pub async fn humata_call(&mut self, id: Value, name: &str, arguments: Value) -> Value {
        self.call(
            id,
            "humata_call",
            json!({ "name": name, "arguments": arguments }),
        )
        .await
    }

    /// Discovers every offered tool via `humata_list` and describes them in
    /// batches of 20 via `humata_describe`.
    pub async fn all_described_tools(&mut self) -> Vec<Value> {
        let listed = self.humata_list(json!({})).await;
        let names: Vec<String> = listed["tools"]
            .as_array()
            .expect("humata_list tools")
            .iter()
            .filter_map(|line| line.as_str()?.split_whitespace().next().map(str::to_owned))
            .collect();
        let mut described = Vec::with_capacity(names.len());
        for chunk in names.chunks(20) {
            let refs: Vec<&str> = chunk.iter().map(String::as_str).collect();
            let batch = self.humata_describe(&refs).await;
            described.extend(
                batch["tools"]
                    .as_array()
                    .expect("humata_describe tools")
                    .iter()
                    .cloned(),
            );
        }
        described
    }

    /// Closes the agent's side, as when the agent exits, and returns every
    /// message the server sent after the last one read, once it stopped.
    pub async fn close(mut self) -> (Result<(), ServeError>, Vec<Value>) {
        self.to_server = None;
        let mut rest = Vec::new();
        while let Some(message) = self.receive().await {
            rest.push(message);
        }
        let served = within(self.server).await.expect("the server didn't panic");
        (served, rest)
    }
}

/// A tool result's structured content.
pub fn structured(result: &Value) -> &Value {
    &result["structuredContent"]
}

/// The smallest arguments `schema` accepts: required fields only, the
/// first value of each choice.
pub fn minimal(schema: &Value) -> Value {
    if let Some(choices) = schema.get("oneOf").and_then(Value::as_array) {
        // An object's oneOf lists which of its fields must be given.
        if schema.get("type") != Some(&json!("object")) {
            return minimal(&choices[0]);
        }
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return values[0].clone();
    }
    let kind = match schema.get("type") {
        Some(Value::Array(kinds)) => kinds[0].as_str().unwrap_or("null"),
        Some(Value::String(kind)) => kind.as_str(),
        _ => "object",
    };
    match kind {
        "string" => json!("x"),
        "integer" => schema.get("minimum").cloned().unwrap_or(json!(0)),
        "boolean" => json!(false),
        "array" => json!([]),
        _ => {
            let mut required: Vec<String> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|names| names.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            if let Some(first) = schema
                .get("oneOf")
                .and_then(|choices| choices.get(0))
                .and_then(|choice| choice.get("required"))
                .and_then(Value::as_array)
            {
                required.extend(first.iter().filter_map(Value::as_str).map(str::to_owned));
            }
            let mut out = Map::new();
            for name in required {
                let property = &schema["properties"][&name];
                out.insert(name, minimal(property));
            }
            Value::Object(out)
        }
    }
}
