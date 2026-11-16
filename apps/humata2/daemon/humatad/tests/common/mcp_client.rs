//! One `humatad2 mcp` process spoken to as an agent's client would, for
//! tests that drive the daemon through its MCP tools.

use std::path::Path;
use std::process::Stdio;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::common::WAIT;

pub struct Mcp {
    _child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
}

impl Mcp {
    /// `humatad2 mcp` (the `HUMATAD2` binary) on `socket` as the agent
    /// holding `token`, initialized.
    pub async fn start(socket: &Path, token: &str) -> Self {
        let binary = std::env::var_os("HUMATAD2").expect("set HUMATAD2 to the binary's path");
        let mut child = Command::new(binary)
            .arg("mcp")
            .arg("--socket")
            .arg(socket)
            .env("HUMATA_SESSION_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut mcp = Self { _child: child, stdin, stdout };
        let init = json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0" },
        });
        mcp.request(0, "initialize", init).await;
        mcp.send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).await;
        mcp
    }

    async fn send(&mut self, message: Value) {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await.unwrap();
        self.stdin.flush().await.unwrap();
    }

    /// Sends a request and returns its response's result; notifications on
    /// the way are skipped.
    pub async fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })).await;
        loop {
            let line = tokio::time::timeout(WAIT, self.stdout.next_line())
                .await
                .expect("humatad2 mcp went quiet")
                .unwrap()
                .expect("humatad2 mcp closed its output");
            let message: Value = serde_json::from_str(&line).unwrap();
            if message.get("method").is_none() && message["id"] == json!(id) {
                assert!(message.get("error").is_none(), "{message}");
                return message["result"].clone();
            }
        }
    }

    pub async fn call(&mut self, id: u64, tool: &str, arguments: Value) -> Value {
        self.request(id, "tools/call", json!({ "name": tool, "arguments": arguments })).await
    }
}
