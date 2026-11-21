//! Regression tests ensuring packaged hooks never gate generic agent permissions (ATM-62).

use std::path::{Path, PathBuf};
use std::process::Command;

use humata2_agent_hooks::{
    claude_settings_has_permission_hook, copilot_plugin, copilot_plugin_has_permission_hook,
};
use serde_json::json;

const SOCKET: &str = "/state dir/humatad.sock";

fn verify_script_path() -> PathBuf {
    let workspace_rel = Path::new("apps/humata2/daemon/agent_hooks/verify_packaged_hooks.sh");
    if workspace_rel.exists() {
        return workspace_rel.to_path_buf();
    }
    PathBuf::from("verify_packaged_hooks.sh")
}

#[test]
fn copilot_plugin_never_includes_permission_hook() {
    let files = copilot_plugin(Path::new("/opt/humata/bin/humata"), Path::new(SOCKET));
    for file in &files {
        assert!(
            !file.contents.contains("permissionRequest"),
            "{} must not contain permissionRequest",
            file.path
        );
        assert!(
            !file.contents.contains("hook copilot permission"),
            "{} must not invoke hook copilot permission",
            file.path
        );
    }
    assert!(!copilot_plugin_has_permission_hook(&files));
}

#[test]
fn claude_settings_permission_hook_detection() {
    let clean = json!({
        "hooks": {
            "Notification": [{"hooks": [{"type": "command", "command": "humata hook claude notification"}]}],
            "SessionStart": [{"hooks": [{"type": "command", "command": "humata hook claude conversation"}]}],
            "UserPromptSubmit": [{"hooks": [{"type": "command", "command": "humata turn working"}]}],
            "Stop": [{"hooks": [{"type": "command", "command": "humata turn idle"}]}],
            "PreToolUse": [{
                "matcher": "AskUserQuestion",
                "hooks": [{"type": "command", "command": "humata hook claude question"}]
            }]
        }
    });
    assert!(!claude_settings_has_permission_hook(&clean));

    let with_permission = json!({
        "hooks": {
            "SessionStart": [{"hooks": [{"type": "command", "command": "humata hook claude conversation"}]}],
            "PermissionRequest": [{"hooks": [{"type": "command", "command": "humata hook claude permission"}]}]
        }
    });
    assert!(claude_settings_has_permission_hook(&with_permission));
}

#[test]
fn verify_packaged_hooks_script_passes_on_clean_dir_and_fails_on_permission_hooks() {
    let script = verify_script_path();

    // Clean directory with generated Copilot plugin files passes --verify.
    let clean_dir = tempfile::tempdir().expect("create clean temp dir");
    for file in copilot_plugin(Path::new("/opt/humata/bin/humata"), Path::new(SOCKET)) {
        std::fs::write(clean_dir.path().join(file.path), file.contents).expect("write plugin file");
    }
    let clean_out = Command::new("/bin/sh")
        .arg(&script)
        .arg("--verify")
        .arg(clean_dir.path())
        .output()
        .expect("run verify_packaged_hooks.sh on clean dir");
    assert!(
        clean_out.status.success(),
        "expected exit 0 on clean dir, stderr: {}",
        String::from_utf8_lossy(&clean_out.stderr)
    );

    // Directory with Copilot permissionRequest fails --verify.
    let copilot_bad = tempfile::tempdir().expect("create bad copilot temp dir");
    std::fs::write(
        copilot_bad.path().join("hooks.json"),
        r#"{"version": 1, "hooks": {"permissionRequest": []}}"#,
    )
    .expect("write bad hooks.json");
    let copilot_out = Command::new("/bin/sh")
        .arg(&script)
        .arg("--verify")
        .arg(copilot_bad.path())
        .output()
        .expect("run verify_packaged_hooks.sh on bad copilot dir");
    assert!(!copilot_out.status.success(), "expected non-zero exit for permissionRequest");
    let copilot_err = String::from_utf8_lossy(&copilot_out.stderr);
    assert!(
        copilot_err.contains("permissionRequest") || copilot_err.contains("hooks.json"),
        "expected visible error message, got: {copilot_err}"
    );

    // Directory with Claude PermissionRequest fails --verify.
    let claude_bad = tempfile::tempdir().expect("create bad claude temp dir");
    std::fs::write(
        claude_bad.path().join("settings.json"),
        r#"{"hooks": {"PermissionRequest": []}}"#,
    )
    .expect("write bad settings.json");
    let claude_out = Command::new("/bin/sh")
        .arg(&script)
        .arg("--verify")
        .arg(claude_bad.path())
        .output()
        .expect("run verify_packaged_hooks.sh on bad claude dir");
    assert!(!claude_out.status.success(), "expected non-zero exit for PermissionRequest");
    let claude_err = String::from_utf8_lossy(&claude_out.stderr);
    assert!(
        claude_err.contains("PermissionRequest") || claude_err.contains("settings.json"),
        "expected visible error message, got: {claude_err}"
    );
}
