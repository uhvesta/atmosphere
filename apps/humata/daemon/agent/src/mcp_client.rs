//! A turn's connection to an MCP server, such as Humata's own, for backends
//! that run the model themselves: the server is started for the turn, its
//! tools are offered to the model, and the model's calls to them are passed
//! on. JSON-RPC messages go one per line, as MCP's stdio transport says.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::claude::McpServer;
use crate::env::Env;

/// How long the server has to answer; Humata's tools may wait as long as a
/// minute for the Mac. A call that says how long it waits gets that long as
/// well.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(90);
/// The longest wait of its own a call is given time for, whatever it asks.
const LONGEST_WAIT: Duration = Duration::from_secs(60 * 60);

/// How long a call with these arguments has to answer: the usual time, and
/// its own `wait_seconds` on top when it gives one.
fn allowed(usual: Duration, arguments: &Value) -> Duration {
    let wait = arguments["wait_seconds"]
        .as_f64()
        .and_then(|wait| Duration::try_from_secs_f64(wait).ok())
        .map_or(Duration::ZERO, |wait| wait.min(LONGEST_WAIT));
    usual.saturating_add(wait)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum McpClientError {
    #[error("couldn't start the MCP server {program}")]
    Start {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("the MCP server's pipe failed")]
    Pipe {
        #[source]
        source: std::io::Error,
    },

    #[error("the MCP server ended")]
    Ended,

    #[error(
        "the MCP server didn't answer {method} within {seconds}s; check that humatad is running, then send the message again"
    )]
    TimedOut { method: String, seconds: u64 },

    #[error(
        "{tool} didn't answer within {seconds}s, so Humata stopped waiting and told its server to give the call up. If it only reads, call it again; if it takes wait_seconds, give a shorter one and call again for the rest. If it starts or sends something (start_session, send_message, a proposal, a question), it may have been done all the same: check with read_session or board before calling it again, or it may be done twice"
    )]
    CallTimedOut { tool: String, seconds: u64 },

    #[error("the MCP server answered {method} with an error: {message}")]
    Refused { method: String, message: String },
}

pub(crate) struct McpClient {
    /// Killed when the turn drops the client.
    _child: Child,
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    next: u64,
    /// The names of the server's tools.
    names: Vec<String>,
    /// How long the server has to answer, before a call's own wait.
    patience: Duration,
}

impl McpClient {
    /// Starts `server` for `session` and lists its tools, as the
    /// chat-completions API takes them.
    pub(crate) async fn start(
        server: &McpServer,
        session: &str,
        env: &Env,
    ) -> Result<(Self, Vec<Value>), McpClientError> {
        let mut child = Command::new(&server.command)
            .args(&server.args)
            .envs(env.vars())
            .envs(server.env.iter().cloned())
            .env("HUMATA_SESSION", session)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| McpClientError::Start {
                program: server.command.clone(),
                source,
            })?;
        let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(McpClientError::Ended);
        };
        let mut client = Self {
            _child: child,
            input,
            output: BufReader::new(output).lines(),
            next: 0,
            names: Vec::new(),
            patience: ANSWER_TIMEOUT,
        };
        let patience = client.patience;
        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "humata", "version": env!("CARGO_PKG_VERSION")},
                }),
                patience,
            )
            .await?;
        client
            .send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await?;
        let listed = client.request("tools/list", json!({}), patience).await?;
        let tools: Vec<Value> = listed["tools"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|tool| {
                let name = tool["name"].as_str()?;
                Some(json!({"type": "function", "function": {
                    "name": name,
                    "description": tool["description"].as_str().unwrap_or_default(),
                    "parameters": tool.get("inputSchema").cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                }}))
            })
            .collect();
        client.names = tools
            .iter()
            .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
            .collect();
        Ok((client, tools))
    }

    pub(crate) fn owns(&self, tool: &str) -> bool {
        self.names.iter().any(|name| name == tool)
    }

    /// Calls `tool` with the model's arguments, giving whether it succeeded
    /// and its text.
    pub(crate) async fn call(
        &mut self,
        tool: &str,
        arguments: &str,
    ) -> Result<(bool, String), McpClientError> {
        let arguments: Value = if arguments.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str(arguments) {
                Ok(arguments) => arguments,
                Err(error) => return Ok((false, format!("Bad arguments for {tool}: {error}"))),
            }
        };
        let allowed = allowed(self.patience, &arguments);
        let result = match self
            .request(
                "tools/call",
                json!({"name": tool, "arguments": arguments}),
                allowed,
            )
            .await
        {
            Err(McpClientError::Refused { message, .. }) => return Ok((false, message)),
            Err(McpClientError::TimedOut { seconds, .. }) => {
                return Err(McpClientError::CallTimedOut {
                    tool: tool.to_owned(),
                    seconds,
                });
            }
            result => result?,
        };
        let text: Vec<&str> = result["content"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| part["text"].as_str())
            .collect();
        let failed = result["isError"].as_bool().unwrap_or(false);
        Ok((!failed, text.join("\n")))
    }

    async fn send(&mut self, message: &Value) -> Result<(), McpClientError> {
        let mut line = message.to_string();
        line.push('\n');
        self.input
            .write_all(line.as_bytes())
            .await
            .map_err(|source| McpClientError::Pipe { source })?;
        self.input
            .flush()
            .await
            .map_err(|source| McpClientError::Pipe { source })
    }

    /// Sends a request and waits up to `allowed` for its answer, passing
    /// over anything else the server says. A request given up on is
    /// cancelled, so the server stops working on it.
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        allowed: Duration,
    ) -> Result<Value, McpClientError> {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        let answer = tokio::time::timeout(allowed, async {
            loop {
                let line = self
                    .output
                    .next_line()
                    .await
                    .map_err(|source| McpClientError::Pipe { source })?
                    .ok_or(McpClientError::Ended)?;
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message["id"].as_u64() == Some(id) {
                    return Ok::<_, McpClientError>(message);
                }
            }
        })
        .await;
        let Ok(answer) = answer else {
            // The id as a number, which is how the server keys its calls.
            // If this can't be written the pipe is gone, and the next
            // request says so.
            let _ = self
                .send(&json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/cancelled",
                    "params": {"requestId": id, "reason": "the caller stopped waiting"},
                }))
                .await;
            return Err(McpClientError::TimedOut {
                method: method.to_owned(),
                seconds: allowed.as_secs(),
            });
        };
        let answer = answer?;
        if let Some(error) = answer.get("error") {
            return Err(McpClientError::Refused {
                method: method.to_owned(),
                message: error["message"].as_str().unwrap_or_default().to_owned(),
            });
        }
        Ok(answer["result"].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_that_waits_is_allowed_longer_than_its_wait() {
        let usual = Duration::from_secs(90);

        let long = allowed(usual, &json!({"session": "s", "wait_seconds": 600}));
        assert!(long > Duration::from_secs(600));
        assert!(allowed(usual, &json!({"wait_seconds": 0.5})) > usual);
        assert_eq!(allowed(usual, &json!({"session": "s"})), usual);
        assert_eq!(allowed(usual, &json!({"wait_seconds": 0})), usual);
        assert_eq!(allowed(usual, &json!({"wait_seconds": "soon"})), usual);
        assert_eq!(allowed(usual, &json!({"wait_seconds": -5})), usual);
        // A wait without end isn't waited for without end.
        let endless = allowed(usual, &json!({"wait_seconds": 1e300}));
        assert!(endless < Duration::from_secs(24 * 60 * 60));
        let days = allowed(usual, &json!({"wait_seconds": 1_000_000}));
        assert!(days < Duration::from_secs(24 * 60 * 60));
    }

    /// A server with one tool, `slow`, that it never answers; whatever it
    /// is sent after its tools are listed, it writes to `heard`.
    const NEVER_ANSWERS: &str = r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*) echo '{"jsonrpc":"2.0","id":1,"result":{}}' ;;
    *'"method":"tools/list"'*) echo '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"slow"}]}}' ;;
    *'"method":"notifications/initialized"'*) ;;
    *) printf '%s\n' "$line" >> "$1" ;;
  esac
done
"#;

    #[tokio::test]
    async fn a_call_given_up_on_is_cancelled_by_its_id() {
        let folder = tempfile::tempdir().expect("a temporary folder");
        let script = folder.path().join("server.sh");
        let heard = folder.path().join("heard");
        std::fs::write(&script, NEVER_ANSWERS).expect("a script");
        let server = McpServer {
            command: "/bin/sh".into(),
            args: vec![
                script.to_string_lossy().into_owned(),
                heard.to_string_lossy().into_owned(),
            ],
            env: Vec::new(),
        };
        let env = Env::new(std::env::vars().filter(|(name, _)| name == "PATH"));
        let (mut client, _) = McpClient::start(&server, "s", &env)
            .await
            .expect("the server starts");
        client.patience = Duration::from_millis(200);

        let result = client.call("slow", "{}").await;

        assert!(matches!(result, Err(McpClientError::CallTimedOut { .. })));
        // The server reads the cancellation a moment after it was written.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let said = loop {
            let said: Vec<Value> = std::fs::read_to_string(&heard)
                .unwrap_or_default()
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            if said.len() >= 2 {
                break said;
            }
            assert!(tokio::time::Instant::now() < deadline, "heard {said:?}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        assert_eq!(said[0]["method"], "tools/call");
        assert_eq!(said[1]["method"], "notifications/cancelled");
        assert_eq!(said[1]["params"]["requestId"], said[0]["id"]);
        assert!(said[1]["params"]["requestId"].is_u64());
    }
}
