use std::io::Cursor;

use lore_context::EnvHints;
use lore_engine::{PulseRequest, RememberRequest};
use lore_proto::{ClaimKind, ManualClock};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::{
    CUE_HOOK_TOKEN_BUDGET, LocalBackend, LoreBackend, McpServer, PROMPT_HOOK_TOKEN_BUDGET,
    handle_cue_hook, handle_prompt_hook,
};

fn test_env(session_id: &str) -> EnvHints {
    EnvHints {
        machine_id: Some("test-mac".to_string()),
        channel_id: Some("chan-1".to_string()),
        session_id: Some(session_id.to_string()),
        agent_kind: Some("agy".to_string()),
        session_repos: None,
        os: Some("macos".to_string()),
        arch: Some("aarch64".to_string()),
    }
}

fn rpc_call<B: LoreBackend>(server: &mut McpServer<B>, req: Value) -> Value {
    let reply = server
        .handle_line(&req.to_string())
        .expect("expected JSON-RPC response");
    serde_json::from_str(&reply).expect("valid JSON-RPC response")
}

fn tool_json(resp: &Value) -> Value {
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("tool content text");
    serde_json::from_str(text).expect("tool JSON payload")
}

#[test]
fn mcp_initialize_list_and_all_five_tools() {
    let dir = TempDir::new().unwrap();
    let clock = ManualClock::new(1_700_000_000_000);
    let backend = LocalBackend::with_clock(dir.path(), clock).unwrap();
    let mut server = McpServer::new(backend, dir.path().to_path_buf(), test_env("sess-a"));

    // 1. initialize
    let init = rpc_call(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "lore");

    // notifications/initialized returns None
    assert!(
        server
            .handle_line(
                &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string()
            )
            .is_none()
    );

    // 2. tools/list exposes exact 5 tools
    let list = rpc_call(
        &mut server,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }),
    );
    let names: Vec<&str> = list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["recall", "inspect", "remember", "feedback", "pulse"]);

    // 3. remember
    let rem = rpc_call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "remember",
                "arguments": {
                    "kind": "solution",
                    "statement": "Use tools/bq/bq instead of bare bazel for shared builds",
                    "body": "Running tools/bq/bq coalesces concurrent agent builds.",
                    "about": ["tool:bq"]
                }
            }
        }),
    );
    let rem_body = tool_json(&rem);
    assert_eq!(rem_body["enabled"], true);
    let claim_id = rem_body["id"].as_str().unwrap().to_string();

    // 4. recall
    let rec = rpc_call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "recall",
                "arguments": { "query": "bq bazel" }
            }
        }),
    );
    let rec_body = tool_json(&rec);
    assert_eq!(rec_body["enabled"], true);
    let cards = rec_body["cards"].as_array().unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["id"], claim_id);

    // 5. inspect
    let ins = rpc_call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "inspect",
                "arguments": { "id": claim_id, "include_lineage": true }
            }
        }),
    );
    let ins_body = tool_json(&ins);
    assert_eq!(ins_body["enabled"], true);
    assert_eq!(ins_body["claim"]["id"], claim_id);
    assert_eq!(ins_body["hydrated_from_cold"], false);

    // 6. feedback
    let fb = rpc_call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "feedback",
                "arguments": { "id": claim_id, "outcome": "helpful", "note": "saved 5m" }
            }
        }),
    );
    let fb_body = tool_json(&fb);
    assert_eq!(fb_body["enabled"], true);
    assert_eq!(fb_body["counters"]["helpful_count"], 1);

    // 7. pulse
    let pls = rpc_call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {
                "name": "pulse",
                "arguments": {
                    "intent": "implementing lore MCP server",
                    "touched_paths": ["tools/lore/mcp/src/server.rs"]
                }
            }
        }),
    );
    let pls_body = tool_json(&pls);
    assert_eq!(pls_body["enabled"], true);
    assert!(pls_body["peers"].is_array());
}

#[test]
fn mcp_disabled_mode_refuses_all_tools_with_reason_and_instruction() {
    let dir = TempDir::new().unwrap();
    let clock = ManualClock::new(1_700_000_000_000);
    let mut backend = LocalBackend::with_clock(dir.path(), clock).unwrap();
    backend
        .set_enabled(false, Some("benchmarking without memory".to_string()))
        .unwrap();

    let mut server = McpServer::new(backend, dir.path().to_path_buf(), test_env("sess-b"));

    for (idx, (tool, args)) in [
        ("recall", json!({ "query": "anything" })),
        ("inspect", json!({ "id": "clm_1" })),
        ("remember", json!({ "kind": "fact", "statement": "hello" })),
        ("feedback", json!({ "id": "clm_1", "outcome": "helpful" })),
        ("pulse", json!({ "intent": "working" })),
    ]
    .into_iter()
    .enumerate()
    {
        let resp = rpc_call(
            &mut server,
            json!({
                "jsonrpc": "2.0",
                "id": idx + 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": args }
            }),
        );
        let body = tool_json(&resp);
        assert_eq!(body["enabled"], false, "tool {tool} should return enabled=false");
        assert_eq!(body["disabled_reason"], "benchmarking without memory");
        assert!(
            body["instruction"]
                .as_str()
                .unwrap()
                .contains("Do NOT attempt to re-enable"),
            "tool {tool} missing anti-circumvention instruction"
        );
    }

    // Verify stdio loop works end-to-end as well.
    let input = format!(
        "{}\n",
        json!({ "jsonrpc": "2.0", "id": 99, "method": "ping" })
    );
    let mut out = Vec::new();
    server
        .run_stdio(Cursor::new(input.as_bytes()), &mut out)
        .unwrap();
    let parsed: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["id"], 99);
}

#[test]
fn prompt_and_cue_hooks_fail_open_and_deduplicate_session_exposures() {
    let dir = TempDir::new().unwrap();
    let clock = ManualClock::new(1_700_000_000_000);
    let mut backend = LocalBackend::with_clock(dir.path(), clock).unwrap();
    let env = test_env("sess-hook-1");

    // Malformed stdin fails open (None).
    assert!(handle_prompt_hook(&mut backend, b"not json {{{", dir.path(), &env).is_none());
    assert!(handle_cue_hook(&mut backend, b"not json {{{", dir.path(), &env).is_none());
    assert!(handle_prompt_hook(&mut backend, b"", dir.path(), &env).is_none());

    // Record a SOLUTION claim tied to rustc E0382.
    let mut rem = RememberRequest::default();
    rem.kind = ClaimKind::Solution;
    rem.statement = "Clone the Arc handle before spawning the worker to fix E0382".to_string();
    rem.scope = "global".to_string();
    rem.about = vec!["err:rustc:e0382".to_string(), "file:src/worker.rs".to_string()];
    backend.remember(rem).unwrap();

    // First prompt hook surfaces the claim within 350 tokens.
    let prompt_in = json!({ "prompt": "fix E0382 worker clone", "session_id": "sess-hook-1" })
        .to_string();
    let first = handle_prompt_hook(&mut backend, prompt_in.as_bytes(), dir.path(), &env)
        .expect("first prompt hook should surface claim");
    assert!(first.contains("E0382"));
    assert!(first.len().div_ceil(4) <= PROMPT_HOOK_TOKEN_BUDGET);

    // Second prompt hook in the same session suppresses already-surfaced claims (`RET-3`).
    let second = handle_prompt_hook(&mut backend, prompt_in.as_bytes(), dir.path(), &env);
    assert!(second.is_none(), "second prompt hook should suppress exposed claim");

    // Cue hook in a fresh session matches E0382 stderr and stays within 250 tokens.
    let env2 = test_env("sess-hook-2");
    let cue_in = json!({
        "exit_code": 1,
        "stderr": "error[E0382]: borrow of moved value: `handle` at src/worker.rs:42:9",
        "session_id": "sess-hook-2"
    })
    .to_string();
    let cue_out = handle_cue_hook(&mut backend, cue_in.as_bytes(), dir.path(), &env2)
        .expect("cue hook should match E0382 solution");
    assert!(cue_out.contains("E0382"));
    assert!(cue_out.len().div_ceil(4) <= CUE_HOOK_TOKEN_BUDGET);

    // Cue hook with exit_code 0 returns None.
    let ok_in = json!({ "exit_code": 0, "stderr": "error[E0382]: ignored" }).to_string();
    assert!(handle_cue_hook(&mut backend, ok_in.as_bytes(), dir.path(), &env2).is_none());

    // Add a peer failure overlap and verify disabled mode returns None for both hooks.
    let peer_env = test_env("sess-peer");
    let mut pulse_req = PulseRequest::new(lore_context::detect(dir.path(), &peer_env));
    pulse_req.recent_failures = vec!["err:rustc:e0382".to_string()];
    backend.pulse(pulse_req).unwrap();

    backend
        .set_enabled(false, Some("paused".to_string()))
        .unwrap();
    assert!(handle_prompt_hook(&mut backend, prompt_in.as_bytes(), dir.path(), &env2).is_none());
    assert!(handle_cue_hook(&mut backend, cue_in.as_bytes(), dir.path(), &env2).is_none());
}

#[test]
fn mcp_lazy_startup_and_live_config_toggle_both_directions() {
    let dir = TempDir::new().unwrap();

    // Pre-write config with `memory.enabled = false` before starting McpServer.
    let mut cfg = lore_engine::LoreConfig::default();
    cfg.memory.enabled = false;
    cfg.memory.disabled_reason = Some("maintenance".to_string());
    lore_engine::save_config(dir.path(), &cfg).unwrap();

    let clock = ManualClock::new(1_700_000_000_000);
    let backend = LocalBackend::with_clock_lazy(dir.path(), clock);
    let mut server = McpServer::new(backend, dir.path().to_path_buf(), test_env("sess-lazy"));

    // Calling recall while disabled MUST refuse and MUST NOT create index.db or write.lock.
    let resp = rpc_call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "recall", "arguments": { "query": "test" } }
        }),
    );
    let body = tool_json(&resp);
    assert_eq!(body["enabled"], false);
    assert_eq!(body["disabled_reason"], "maintenance");
    assert!(!dir.path().join("index.db").exists());
    assert!(!dir.path().join("write.lock").exists());

    // Flip `memory.enabled = true` on disk while McpServer is still running.
    cfg.memory.enabled = true;
    cfg.memory.disabled_reason = None;
    lore_engine::save_config(dir.path(), &cfg).unwrap();

    let rem = rpc_call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "remember",
                "arguments": { "kind": "fact", "statement": "live re-enable works" }
            }
        }),
    );
    assert_eq!(tool_json(&rem)["enabled"], true);

    // Flip back to `false` on disk and confirm immediate refusal.
    cfg.memory.enabled = false;
    cfg.memory.disabled_reason = Some("re-paused".to_string());
    lore_engine::save_config(dir.path(), &cfg).unwrap();

    let rec = rpc_call(
        &mut server,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": { "name": "recall", "arguments": { "query": "live" } }
        }),
    );
    let rec_body = tool_json(&rec);
    assert_eq!(rec_body["enabled"], false);
    assert_eq!(rec_body["disabled_reason"], "re-paused");
}

#[test]
fn resolve_write_scope_handles_sandboxed_dir_and_multi_repo_cross_writes() {
    use std::path::PathBuf;
    use lore_context::{ContextFrame, RepoMount, permitted_scopes};
    use crate::resolve_write_scope;

    // #8a: Sandboxed worktree fallback `dir:<m>:<h>` must NOT become `repo:dir:<m>:<h>`.
    let dir_repo_id = "dir:mac-1:0123456789abcdef".to_string();
    let sandbox_frame = ContextFrame {
        cwd: PathBuf::from("/tmp/wt"),
        machine_id: "mac-1".to_string(),
        os: "macos".to_string(),
        arch: "aarch64".to_string(),
        channel_id: None,
        session_id: Some("s1".to_string()),
        agent_kind: Some("agy".to_string()),
        mounted_repos: vec![RepoMount {
            repo_id: dir_repo_id.clone(),
            root_path: PathBuf::from("/tmp/wt"),
            mount_subpath: PathBuf::from("."),
            branch: None,
            head_sha: None,
            sandboxed_worktree: true,
            languages: Vec::new(),
            lockfiles: Vec::new(),
        }],
        cwd_dir_scope: dir_repo_id.clone(),
    };
    let mut about_sandbox = Vec::new();
    let scope = resolve_write_scope(None, &sandbox_frame, &mut about_sandbox);
    assert_eq!(scope, dir_repo_id);
    assert!(permitted_scopes(&sandbox_frame).contains(&scope));

    // #8b-write: Multi-repo write citing files in 2 distinct repos binds to channel/global + repo tags.
    let multi_frame = ContextFrame {
        cwd: PathBuf::from("/ws"),
        machine_id: "mac-1".to_string(),
        os: "macos".to_string(),
        arch: "aarch64".to_string(),
        channel_id: Some("chan-multi".to_string()),
        session_id: Some("s2".to_string()),
        agent_kind: Some("agy".to_string()),
        mounted_repos: vec![
            RepoMount {
                repo_id: "github.com/org/repo-a".to_string(),
                root_path: PathBuf::from("/ws/a"),
                mount_subpath: PathBuf::from("a"),
                branch: None,
                head_sha: None,
                sandboxed_worktree: false,
                languages: Vec::new(),
                lockfiles: Vec::new(),
            },
            RepoMount {
                repo_id: "github.com/org/repo-b".to_string(),
                root_path: PathBuf::from("/ws/b"),
                mount_subpath: PathBuf::from("b"),
                branch: None,
                head_sha: None,
                sandboxed_worktree: false,
                languages: Vec::new(),
                lockfiles: Vec::new(),
            },
        ],
        cwd_dir_scope: "dir:mac-1:fedcba9876543210".to_string(),
    };
    let mut about_multi = vec!["file:a/src/lib.rs".to_string(), "file:b/pkg/main.go".to_string()];
    let multi_scope = resolve_write_scope(None, &multi_frame, &mut about_multi);
    assert_eq!(multi_scope, "channel:chan-multi");
    assert!(about_multi.contains(&"file:src/lib.rs".to_string()));
    assert!(about_multi.contains(&"file:pkg/main.go".to_string()));
    assert!(about_multi.contains(&"repo:github.com/org/repo-a".to_string()));
    assert!(about_multi.contains(&"repo:github.com/org/repo-b".to_string()));
}
