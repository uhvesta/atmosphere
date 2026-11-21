//! Humata's own MCP server as each agent's CLI is told of it for one
//! session (D748): `humatad2 mcp --socket <socket>`, run by the CLI.
//!
//! Only the command and the socket are written. The session's token stays
//! in the environment the agent was started with (`HUMATA_SESSION_TOKEN`),
//! which the CLI hands its MCP servers, so no file Humata writes holds it.

use std::path::Path;

use serde_json::json;

/// The name every agent's CLI knows Humata's server by: Copilot's
/// `--allow-tool=humata`, Claude's `mcp__humata` tools, Codex's
/// `mcp_servers.humata`.
pub const MCP_SERVER: &str = "humata";

/// The variable the session's token is in, which the server reads.
const TOKEN_VAR: &str = "HUMATA_SESSION_TOKEN";

fn server_args(socket: &Path) -> [String; 3] {
    ["mcp".to_owned(), "--socket".to_owned(), socket.to_string_lossy().into_owned()]
}

/// The MCP config file Claude Code loads with `--mcp-config=<file>`, on top
/// of the person's own servers: one stdio server, `humatad` run as `mcp`.
pub fn claude_mcp_config(humatad: &Path, socket: &Path) -> String {
    let config = json!({
        "mcpServers": {
            MCP_SERVER: {
                "type": "stdio",
                "command": humatad.to_string_lossy(),
                "args": server_args(socket),
            },
        },
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

/// The MCP config file Copilot loads with `--additional-mcp-config=@<file>`,
/// on top of the person's own: the same server, every tool of it offered.
pub fn copilot_mcp_config(humatad: &Path, socket: &Path) -> String {
    let config = json!({
        "mcpServers": {
            MCP_SERVER: {
                "type": "stdio",
                "command": humatad.to_string_lossy(),
                "args": server_args(socket),
                "tools": ["*"],
            },
        },
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}

/// The TOML value Codex is launched with as `-c mcp_servers.humata=<value>`:
/// the same server, handed the session's token by name (`env_vars`; Codex
/// passes its servers only the variables it is told to), its tools
/// approved for the session.
pub fn codex_mcp(humatad: &Path, socket: &Path) -> String {
    let string = |text: &str| toml::Value::String(text.to_owned()).to_string();
    let args: Vec<String> = server_args(socket).iter().map(|arg| string(arg)).collect();
    format!(
        "{{ command = {}, args = [{}], env_vars = [{}], default_tools_approval_mode = \"approve\" }}",
        string(&humatad.to_string_lossy()),
        args.join(", "),
        string(TOKEN_VAR),
    )
}

/// The session-only settings file Antigravity (`agy`) loads with
/// `--settings=<file>` (never saved and never touching `~/.gemini`):
/// one stdio MCP server, `humatad` run as `mcp`, with the session token
/// inherited from the environment.
pub fn agy_mcp_config(humatad: &Path, socket: &Path) -> String {
    let config = json!({
        "mcpServers": {
            MCP_SERVER: {
                "command": humatad.to_string_lossy(),
                "args": server_args(socket),
            },
        },
    });
    serde_json::to_string_pretty(&config).unwrap_or_default()
}
