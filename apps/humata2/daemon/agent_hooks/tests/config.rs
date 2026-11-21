//! The hook config humatad writes for each agent: every hook has an explicit
//! timeout, the permission hook holds for an hour, and no hook can fail the
//! agent's tool call when the daemon or the CLI is gone (D192, D193).

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use humata2_agent_hooks::{
    Agent, CLI_HOLD, DAEMON_HOLD, HOLD_SECONDS, claude_permission_hook, codex_hooks,
    copilot_plugin, permission_command,
};
use serde_json::Value;

const SOCKET: &str = "/state dir/humatad.sock";

/// A stand-in for the `humata` binary that runs `body`.
fn humata(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("humata");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("write a test file");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

/// Runs a hook command the way the agents do: `sh -c`, the payload on stdin.
fn run(command: &str, stdin: &[u8]) -> (i32, Vec<u8>) {
    let mut child = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the hook");
    child.stdin.take().expect("piped stdin").write_all(stdin).expect("write hook stdin");
    let out = child.wait_with_output().expect("hook process exits");
    (out.status.code().unwrap_or(-1), out.stdout)
}

#[test]
fn the_daemon_ends_the_hold_before_the_cli_and_the_cli_before_the_agent() {
    assert_eq!(HOLD_SECONDS, 3600);
    assert!(DAEMON_HOLD < CLI_HOLD);
    assert!(CLI_HOLD.as_secs() < HOLD_SECONDS);
}

#[test]
fn the_permission_command_passes_the_answer_through() {
    let dir = tempfile::tempdir().expect("create a temp dir");
    // Echoes what it was asked, so the test sees the args and stdin arrive.
    let bin = humata(
        dir.path(),
        r#"[ "$HUMATA_SOCKET" = "/state dir/humatad.sock" ] || exit 1
echo "$1 $2 $3"; cat"#,
    );
    for agent in [Agent::Claude, Agent::Codex, Agent::Copilot] {
        let (code, out) = run(
            &permission_command(agent, &bin, Path::new(SOCKET)),
            b"{\"payload\":1}",
        );
        assert_eq!(code, 0);
        let out = String::from_utf8(out).expect("output is UTF-8");
        assert!(out.starts_with(&format!("hook {} permission", agent.name())), "{out}");
        assert!(out.ends_with("{\"payload\":1}"), "{out}");
    }
}

#[test]
fn the_permission_command_never_fails_the_tool_call() {
    let dir = tempfile::tempdir().expect("create a temp dir");
    // An older CLI without `humata hook` exits 2 with usage on stderr, which
    // Codex and Copilot read as a deny; a missing binary exits 127.
    let old = humata(dir.path(), "echo 'unknown subcommand' >&2; exit 2");
    let missing = dir.path().join("gone/humata");
    for bin in [old, missing] {
        for agent in [Agent::Claude, Agent::Codex, Agent::Copilot] {
            let (code, out) = run(&permission_command(agent, &bin, Path::new(SOCKET)), b"{}");
            assert_eq!(code, 0, "{agent:?} {bin:?}");
            assert!(out.is_empty(), "{agent:?}: {}", String::from_utf8_lossy(&out));
        }
    }
}

#[test]
fn claude_holds_the_permission_hook_for_the_hour() {
    let hook = claude_permission_hook(Path::new("/opt/humata/bin/humata"), Path::new(SOCKET));
    assert_eq!(hook["type"], "command");
    assert_eq!(hook["timeout"], HOLD_SECONDS);
    // It blocks: Claude waits for its answer.
    assert_ne!(hook.get("async"), Some(&Value::Bool(true)));
    assert!(hook["command"].as_str().expect("string").contains("hook claude permission"));
}

fn codex_table(humata: &Path) -> toml::Table {
    let value = codex_hooks(humata, Path::new(SOCKET));
    // Passed as `-c hooks=<value>`, so `hooks = <value>` must be TOML.
    let parsed: toml::Table = toml::from_str(&format!("hooks = {value}")).expect("TOML");
    parsed["hooks"].as_table().expect("TOML table").clone()
}

fn codex_commands(table: &toml::Table, event: &str) -> Vec<(String, i64)> {
    table[event]
        .as_array()
        .expect("event is an array")
        .iter()
        .flat_map(|group| group["hooks"].as_array().expect("JSON array").clone())
        .map(|hook| {
            assert_eq!(hook["type"].as_str(), Some("command"));
            (
                hook["command"].as_str().expect("string").to_owned(),
                hook["timeout"].as_integer().expect("TOML integer"),
            )
        })
        .collect()
}

#[test]
fn codex_gets_turn_and_permission_hooks_each_with_a_timeout() {
    let table = codex_table(Path::new("/opt/humata/bin/humata"));
    for event in ["UserPromptSubmit", "Stop", "Interrupt", "PermissionRequest"] {
        assert!(!codex_commands(&table, event).is_empty(), "{event}");
    }
    let permission = codex_commands(&table, "PermissionRequest");
    assert!(permission.iter().any(|(command, timeout)| {
        command.contains("hook codex permission") && *timeout == HOLD_SECONDS as i64
    }));
    // Codex caps Interrupt at 3 s.
    for (_, timeout) in codex_commands(&table, "Interrupt") {
        assert!(timeout <= 3);
    }
}

#[test]
fn codex_stop_prints_json_even_with_humata_gone() {
    let dir = tempfile::tempdir().expect("create a temp dir");
    let table = codex_table(&dir.path().join("gone/humata"));
    for (command, _) in codex_commands(&table, "Stop") {
        let (code, out) = run(&command, b"{}");
        assert_eq!(code, 0);
        let out: Value = serde_json::from_slice(&out).expect("JSON");
        assert!(out.is_object());
    }
}

#[test]
fn copilot_gets_a_plugin_with_turn_and_permission_hooks() {
    let files = copilot_plugin(Path::new("/opt/humata/bin/humata"), Path::new(SOCKET));
    let file = |name: &str| -> Value {
        let found = files
            .iter()
            .find(|file| file.path == name)
            .unwrap_or_else(|| panic!("no {name}"));
        serde_json::from_str(&found.contents).expect("JSON")
    };
    let plugin = file("plugin.json");
    assert!(!plugin["name"].as_str().expect("string").is_empty());
    let hooks = file("hooks.json");
    assert_eq!(hooks["version"], 1);
    for event in [
        "sessionStart",
        "preToolUse",
        "userPromptSubmitted",
        "postToolUse",
        "agentStop",
        "errorOccurred",
    ] {
        let list = hooks["hooks"][event].as_array().expect("JSON array");
        assert!(!list.is_empty(), "{event}");
        for hook in list {
            assert_eq!(hook["type"], "command");
            assert!(hook["bash"].is_string());
            assert!(hook["timeoutSec"].is_u64(), "{event}: every hook says its timeout");
        }
    }
    assert!(
        hooks["hooks"].get("permissionRequest").is_none(),
        "copilot plugin must not register a generic permissionRequest hook"
    );
    let question = &hooks["hooks"]["preToolUse"][0];
    assert_eq!(question["matcher"], "ask_user");
    assert_eq!(question["timeoutSec"], HOLD_SECONDS);
    assert!(question["bash"].as_str().expect("string").contains("hook copilot question"));
}

#[test]
fn every_turn_hook_exits_0_with_humata_gone() {
    let dir = tempfile::tempdir().expect("create a temp dir");
    let gone = dir.path().join("gone/humata");
    let table = codex_table(&gone);
    for event in ["UserPromptSubmit", "Stop", "Interrupt", "PermissionRequest"] {
        for (command, _) in codex_commands(&table, event) {
            assert_eq!(run(&command, b"{}").0, 0, "codex {event}");
        }
    }
    let files = copilot_plugin(&gone, Path::new(SOCKET));
    let hooks: Value = serde_json::from_str(
        &files.iter().find(|f| f.path == "hooks.json").expect("plugin has hooks.json").contents,
    )
    .expect("hooks.json is JSON");
    for (event, list) in hooks["hooks"].as_object().expect("JSON object") {
        for hook in list.as_array().expect("JSON array") {
            let (code, out) = run(hook["bash"].as_str().expect("string"), b"{}");
            assert_eq!(code, 0, "copilot {event}");
            assert!(out.is_empty(), "copilot {event}");
        }
    }
}
