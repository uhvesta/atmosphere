//! Humata's MCP server as each agent's CLI is told of it (D748): the
//! server runs `humatad2 mcp --socket <socket>`, and the session's token
//! is never in what Humata writes for it.

use std::path::Path;

use humata2_agent_hooks::{
    MCP_SERVER, agy_mcp_config, claude_mcp_config, codex_mcp, copilot_mcp_config,
};
use serde_json::Value;

const HUMATAD: &str = "/opt/humata dir/humatad2";
const SOCKET: &str = "/state dir/humatad.sock";

fn server(config: &str) -> Value {
    let config: Value = serde_json::from_str(config).expect("the config is JSON");
    config["mcpServers"][MCP_SERVER].clone()
}

#[test]
fn claude_and_copilot_run_the_daemons_mcp_server_on_this_socket() {
    for config in [
        claude_mcp_config(Path::new(HUMATAD), Path::new(SOCKET)),
        copilot_mcp_config(Path::new(HUMATAD), Path::new(SOCKET)),
    ] {
        let server = server(&config);
        assert_eq!(server["command"], HUMATAD, "{config}");
        assert_eq!(server["args"], serde_json::json!(["mcp", "--socket", SOCKET]), "{config}");
        assert_eq!(server["type"], "stdio", "{config}");
        assert!(server.get("env").is_none(), "the token stays in the environment: {config}");
    }
}

#[test]
fn copilot_is_offered_every_tool_of_the_server() {
    let server = server(&copilot_mcp_config(Path::new(HUMATAD), Path::new(SOCKET)));
    assert_eq!(server["tools"], serde_json::json!(["*"]));
}

#[test]
fn codex_runs_the_same_server_handed_the_token_by_name_and_approved() {
    let value = codex_mcp(Path::new(HUMATAD), Path::new(SOCKET));
    let table: toml::Table = toml::from_str(&format!("server = {value}")).expect("the value is TOML");
    let server = table["server"].as_table().expect("an inline table");
    assert_eq!(server["command"].as_str(), Some(HUMATAD));
    let args: Vec<&str> = server["args"].as_array().unwrap().iter().filter_map(toml::Value::as_str).collect();
    assert_eq!(args, ["mcp", "--socket", SOCKET]);
    let names: Vec<&str> = server["env_vars"].as_array().unwrap().iter().filter_map(toml::Value::as_str).collect();
    assert_eq!(names, ["HUMATA_SESSION_TOKEN"], "the variable's name, never its value");
    assert!(server.get("env").is_none());
    assert_eq!(server["default_tools_approval_mode"].as_str(), Some("approve"));
}

#[test]
fn agy_runs_the_daemons_mcp_server_without_embedding_the_token() {
    let config = agy_mcp_config(Path::new(HUMATAD), Path::new(SOCKET));
    let server = server(&config);
    assert_eq!(server["command"], HUMATAD, "{config}");
    assert_eq!(server["args"], serde_json::json!(["mcp", "--socket", SOCKET]), "{config}");
    assert!(server.get("env").is_none(), "the token stays in the environment: {config}");
}
