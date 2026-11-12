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
/// minute for the Mac.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(90);

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

    #[error("the MCP server didn't answer within {seconds}s")]
    TimedOut { seconds: u64 },

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
        };
        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "humata", "version": env!("CARGO_PKG_VERSION")},
                }),
            )
            .await?;
        client
            .send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await?;
        let listed = client.request("tools/list", json!({})).await?;
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
        let result = match self
            .request("tools/call", json!({"name": tool, "arguments": arguments}))
            .await
        {
            Err(McpClientError::Refused { message, .. }) => return Ok((false, message)),
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

    /// Sends a request and waits for its answer, passing over anything else
    /// the server says.
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpClientError> {
        self.next += 1;
        let id = self.next;
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await?;
        let answer = tokio::time::timeout(ANSWER_TIMEOUT, async {
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
                    return Ok(message);
                }
            }
        })
        .await
        .map_err(|_| McpClientError::TimedOut {
            seconds: ANSWER_TIMEOUT.as_secs(),
        })??;
        if let Some(error) = answer.get("error") {
            return Err(McpClientError::Refused {
                method: method.to_owned(),
                message: error["message"].as_str().unwrap_or_default().to_owned(),
            });
        }
        Ok(answer["result"].clone())
    }
}
