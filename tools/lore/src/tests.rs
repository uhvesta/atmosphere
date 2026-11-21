use lore_context::EnvHints;
use serde_json::Value;
use tempfile::TempDir;

use crate::run_with_io;

fn test_env(session_id: &str) -> EnvHints {
    EnvHints {
        machine_id: Some("cli-test-mac".to_string()),
        channel_id: Some("chan-cli".to_string()),
        session_id: Some(session_id.to_string()),
        agent_kind: Some("agy".to_string()),
        session_repos: None,
        os: Some("macos".to_string()),
        arch: Some("aarch64".to_string()),
    }
}

fn run_cli(
    root: &std::path::Path,
    args: &[&str],
    stdin: &[u8],
    env: &EnvHints,
) -> (i32, String, String) {
    let mut full: Vec<String> = vec![
        "lore".to_string(),
        "--root".to_string(),
        root.to_string_lossy().to_string(),
    ];
    full.extend(args.iter().map(|s| s.to_string()));

    let mut out = Vec::new();
    let mut err = Vec::new();
    let code = run_with_io(full, stdin, root, env, &mut out, &mut err);
    (
        code,
        String::from_utf8_lossy(&out).to_string(),
        String::from_utf8_lossy(&err).to_string(),
    )
}

#[test]
fn cli_full_lifecycle_and_global_kill_switch_enforcement() {
    let dir = TempDir::new().unwrap();
    let env = test_env("sess-cli-1");

    // 1. remember
    let (code, out, err) = run_cli(
        dir.path(),
        &[
            "remember",
            "--kind",
            "solution",
            "Use --config=fast for fast local Bazel loops",
            "--body",
            "Pass --fast to tools/bq/bq to skip clippy during iteration.",
            "--about",
            "tool:bq",
            "--json",
        ],
        b"",
        &env,
    );
    assert_eq!(code, 0, "remember stderr: {err}");
    let rem: Value = serde_json::from_str(out.trim()).unwrap();
    let claim_id = rem["id"].as_str().unwrap().to_string();

    // 2. recall
    let (code, out, err) = run_cli(dir.path(), &["recall", "Bazel loops", "--json"], b"", &env);
    assert_eq!(code, 0, "recall stderr: {err}");
    let rec: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(rec["cards"].as_array().unwrap().len(), 1);
    assert_eq!(rec["cards"][0]["id"], claim_id);

    // 3. show
    let (code, out, err) = run_cli(
        dir.path(),
        &["show", &claim_id, "--lineage", "--json"],
        b"",
        &env,
    );
    assert_eq!(code, 0, "show stderr: {err}");
    let shown: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(shown["claim"]["id"], claim_id);

    // 4. feedback
    let (code, out, err) = run_cli(
        dir.path(),
        &["feedback", &claim_id, "helpful", "--note", "worked", "--json"],
        b"",
        &env,
    );
    assert_eq!(code, 0, "feedback stderr: {err}");
    let fb: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(fb["counters"]["helpful_count"], 1);

    // 5. gc --dry-run
    let (code, out, err) = run_cli(dir.path(), &["gc", "--dry-run", "--json"], b"", &env);
    assert_eq!(code, 0, "gc stderr: {err}");
    let gc_rep: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(gc_rep["dry_run"], true);

    // 6. Disable globally with a custom reason (`SW-1`, `SW-2`, `SW-4`, `SW-5`)
    let (code, out, err) = run_cli(
        dir.path(),
        &[
            "config",
            "set",
            "memory.enabled",
            "false",
            "--reason",
            "incident freeze",
        ],
        b"",
        &env,
    );
    assert_eq!(code, 0, "config set stderr: {err}");
    assert!(out.contains("memory.enabled = false"));

    // All agent-facing CLI commands MUST fail non-zero with disabled_reason & directive
    for cmd in [
        vec!["recall", "Bazel"],
        vec!["show", &claim_id],
        vec!["remember", "--kind", "fact", "blocked"],
        vec!["feedback", &claim_id, "helpful"],
        vec!["pulse", "--intent", "blocked"],
        vec!["guide"],
    ] {
        let (code, out, err) = run_cli(dir.path(), &cmd, b"", &env);
        assert_ne!(code, 0, "command {cmd:?} must exit non-zero when disabled");
        assert!(out.is_empty(), "command {cmd:?} should not write stdout when disabled");
        assert!(
            err.contains("incident freeze") && err.contains("Do not attempt to re-enable"),
            "command {cmd:?} stderr missing reason/directive: {err}"
        );
    }

    // Human admin commands remain operable when disabled (`SW-5`)
    let (code, out, err) = run_cli(dir.path(), &["status"], b"", &env);
    assert_eq!(code, 0, "status stderr: {err}");
    assert!(out.contains("memory.enabled = false"));
    assert!(out.contains("incident freeze"));

    let (code, out, err) = run_cli(dir.path(), &["config", "get", "memory.enabled"], b"", &env);
    assert_eq!(code, 0, "config get stderr: {err}");
    assert_eq!(out.trim(), "false");

    let (code, out, err) = run_cli(dir.path(), &["export", "--jsonl"], b"", &env);
    assert_eq!(code, 0, "export stderr: {err}");
    assert!(out.lines().count() >= 2);

    let (code, _out, err) = run_cli(dir.path(), &["verify"], b"", &env);
    assert_eq!(code, 0, "verify stderr: {err}");

    let (code, _out, err) = run_cli(dir.path(), &["reindex"], b"", &env);
    assert_eq!(code, 0, "reindex stderr: {err}");

    // Hooks MUST exit 0 with empty stdout when disabled
    let (code, out, err) = run_cli(
        dir.path(),
        &["hook", "prompt"],
        br#"{"prompt":"Bazel loops"}"#,
        &env,
    );
    assert_eq!(code, 0);
    assert!(out.is_empty());
    assert!(err.is_empty());

    // 7. Re-enable and verify existing data is intact (`SW-2`)
    let (code, _out, err) = run_cli(
        dir.path(),
        &["config", "set", "memory.enabled", "true"],
        b"",
        &env,
    );
    assert_eq!(code, 0, "re-enable stderr: {err}");

    let (code, out, err) = run_cli(dir.path(), &["recall", "Bazel loops", "--json"], b"", &env);
    assert_eq!(code, 0, "recall after re-enable stderr: {err}");
    let rec2: Value = serde_json::from_str(out.trim()).unwrap();
    assert_eq!(rec2["cards"].as_array().unwrap().len(), 1);
    assert_eq!(rec2["cards"][0]["id"], claim_id);
}

#[test]
fn cli_hooks_fail_open_on_garbage_stdin() {
    let dir = TempDir::new().unwrap();
    let env = test_env("sess-garbage");

    for sub in ["prompt", "cue"] {
        let (code, out, err) = run_cli(
            dir.path(),
            &["hook", sub],
            b"\xff\xfe not valid utf8 or json {{{",
            &env,
        );
        assert_eq!(code, 0, "hook {sub} must exit 0 on garbage stdin");
        assert!(out.is_empty(), "hook {sub} must emit empty stdout on garbage stdin");
        assert!(err.is_empty(), "hook {sub} must emit empty stderr on garbage stdin");
    }
}

#[test]
fn cli_verify_is_read_only_and_hooks_do_not_block_on_write_lock() {
    let dir = TempDir::new().unwrap();
    let env = test_env("sess-ro");

    // Seed one claim so hot segment and index.db exist.
    let (code, _out, err) = run_cli(
        dir.path(),
        &[
            "remember",
            "--kind",
            "solution",
            "Hold write lock without blocking read-only hooks",
            "--about",
            "err:rustc:e0382",
        ],
        b"",
        &env,
    );
    assert_eq!(code, 0, "remember stderr: {err}");

    // #7c: Acquire exclusive write.lock and verify hooks complete immediately without blocking.
    let _held_lock = lore_engine::log::WriteLock::acquire(dir.path()).unwrap();
    let (code, out, err) = run_cli(
        dir.path(),
        &["hook", "prompt"],
        br#"{"prompt":"write lock hooks","session_id":"sess-hook-ro"}"#,
        &env,
    );
    assert_eq!(code, 0, "prompt hook stderr: {err}");
    assert!(out.contains("Hold write lock"));

    let (code, out, err) = run_cli(
        dir.path(),
        &["hook", "cue"],
        br#"{"exit_code":1,"stderr":"error[E0382]: moved value","session_id":"sess-hook-ro2"}"#,
        &env,
    );
    assert_eq!(code, 0, "cue hook stderr: {err}");
    assert!(out.contains("Hold write lock"));

    // #7b: Append garbage torn-tail bytes to the hot segment and confirm `lore verify` does NOT
    // silently truncate the file before reporting.
    let segs = lore_engine::log::list_hot_segments(dir.path()).unwrap();
    assert_eq!(segs.len(), 1);
    let seg_path = &segs[0];
    let orig_len = std::fs::metadata(seg_path).unwrap().len();
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().append(true).open(seg_path).unwrap();
        f.write_all(&[0xde, 0xad, 0xbe, 0xef, 0x01]).unwrap();
        f.sync_all().unwrap();
    }
    let torn_len = std::fs::metadata(seg_path).unwrap().len();
    assert_eq!(torn_len, orig_len + 5);

    let (_code, _out, _err) = run_cli(dir.path(), &["verify"], b"", &env);
    // `lore verify` must NOT mutate or truncate the torn segment on disk.
    assert_eq!(
        std::fs::metadata(seg_path).unwrap().len(),
        torn_len,
        "lore verify must not truncate torn-tail bytes"
    );
}

#[test]
fn parse_when_predicate_supports_colon_axes() {
    let p1 = crate::parse_when_predicate("dep:cargo/serde:eq:1.0.210").unwrap();
    assert_eq!(p1.axis, "dep:cargo/serde");
    assert_eq!(p1.op, lore_proto::PredicateOp::Eq as i32);
    assert_eq!(p1.value, "1.0.210");

    let p2 = crate::parse_when_predicate("dep:maven/com.google.guava:guava:semver:>=33.0").unwrap();
    assert_eq!(p2.axis, "dep:maven/com.google.guava:guava");
    assert_eq!(p2.op, lore_proto::PredicateOp::SemverReq as i32);
    assert_eq!(p2.value, ">=33.0");

    let p3 = crate::parse_when_predicate("os:eq:macos").unwrap();
    assert_eq!(p3.axis, "os");
    assert_eq!(p3.value, "macos");
}
