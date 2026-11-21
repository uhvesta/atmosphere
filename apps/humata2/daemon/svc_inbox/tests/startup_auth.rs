use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use humata2_svc_inbox::ScreenAsk;
use humata2_svc_inbox::startup_auth::{
    AuditOutcome, KeyInput, KeyMappingStyle, OwnerConsentScope, PauseReason, Reversibility,
    StartupAuthError, StartupAuthRunner, StartupConfig, StartupDecision, StartupPromptClass,
    StartupSchedule, StartupStatus, StartupTerminalPort, decide_startup_action,
    parse_startup_prompt, seed_for_session,
};

struct ScriptedTerminal {
    screens: Mutex<Vec<Option<(u64, String)>>>,
    cursor: Mutex<usize>,
    sent_keys: Mutex<Vec<Vec<KeyInput>>>,
    fail_capture: bool,
    fail_send: bool,
}

impl ScriptedTerminal {
    fn new(screens: Vec<Option<(u64, String)>>) -> Self {
        Self {
            screens: Mutex::new(screens),
            cursor: Mutex::new(0),
            sent_keys: Mutex::new(Vec::new()),
            fail_capture: false,
            fail_send: false,
        }
    }

    fn sent(&self) -> Vec<Vec<KeyInput>> {
        self.sent_keys.lock().expect("sent_keys lock").clone()
    }
}

#[tonic::async_trait]
impl StartupTerminalPort for ScriptedTerminal {
    async fn capture_screen(&self) -> Result<Option<(u64, String)>, StartupAuthError> {
        if self.fail_capture {
            return Err(StartupAuthError::ScreenRead {
                detail: "simulated holder disconnect".to_owned(),
            });
        }
        let screens = self.screens.lock().expect("screens lock");
        let mut cursor = self.cursor.lock().expect("cursor lock");
        if screens.is_empty() {
            return Ok(None);
        }
        let idx = (*cursor).min(screens.len() - 1);
        *cursor += 1;
        Ok(screens[idx].clone())
    }

    async fn send_keys(&self, keys: &[KeyInput]) -> Result<(), StartupAuthError> {
        if self.fail_send {
            return Err(StartupAuthError::KeySend {
                detail: "simulated pty write error".to_owned(),
            });
        }
        self.sent_keys
            .lock()
            .expect("sent_keys lock")
            .push(keys.to_vec());
        Ok(())
    }
}

#[test]
fn agy_startup_authorization_variants_and_strict_scope() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/Users/owner/work"));

    // 1. Inline [y/N]
    let agy_yn = "Antigravity (agy)\nDo you authorize me to work in this workspace? [y/N]";
    let prompt = parse_startup_prompt(agy_yn).expect("agy [y/N]");
    assert_eq!(prompt.class, StartupPromptClass::StartupAuthorization);
    assert!(matches!(
        prompt.key_mapping,
        KeyMappingStyle::InlineYesNo {
            full_word: false,
            ..
        }
    ));
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PressKeys { keys, .. } => {
            assert_eq!(keys, vec![KeyInput::Char('y'), KeyInput::Enter]);
        }
        other => panic!("expected PressKeys, got {other:?}"),
    }

    // 2. Inline (yes/no)
    let agy_yesno = "Do you authorize me for this session? (yes/no)";
    let prompt = parse_startup_prompt(agy_yesno).expect("agy (yes/no)");
    assert_eq!(prompt.class, StartupPromptClass::StartupAuthorization);
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PressKeys { keys, .. } => {
            assert_eq!(
                keys,
                vec![KeyInput::Text("yes".to_owned()), KeyInput::Enter]
            );
        }
        other => panic!("expected PressKeys, got {other:?}"),
    }

    // 3. Numbered menu
    let agy_numbered = "\
Antigravity startup
Do you authorize me for this session?
❯ 1. Yes, authorize for this session
  2. No, exit";
    let prompt = parse_startup_prompt(agy_numbered).expect("agy numbered");
    assert_eq!(prompt.class, StartupPromptClass::StartupAuthorization);
    assert_eq!(prompt.key_mapping, KeyMappingStyle::NumberedMenu);
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PressKeys { keys, .. } => {
            assert_eq!(keys, vec![KeyInput::Enter]);
        }
        other => panic!("expected PressKeys, got {other:?}"),
    }

    // 4. Hotkey (y)/(n)
    let agy_hotkey = "Authorize Antigravity for this session? (y) Yes / (n) No";
    let prompt = parse_startup_prompt(agy_hotkey).expect("agy hotkey");
    assert_eq!(prompt.class, StartupPromptClass::StartupAuthorization);
    assert_eq!(prompt.key_mapping, KeyMappingStyle::HotkeyHint);
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PressKeys { keys, .. } => {
            assert_eq!(keys, vec![KeyInput::Char('y'), KeyInput::Enter]);
        }
        other => panic!("expected PressKeys, got {other:?}"),
    }

    // 5. Denied under strict scope
    let strict = OwnerConsentScope::strict();
    match decide_startup_action(&prompt, &strict) {
        StartupDecision::PauseForReview(summary) => {
            assert_eq!(summary.reason, PauseReason::OutOfGrantedScope);
            assert!(summary.requires_smart_agent_review);
        }
        other => panic!("expected PauseForReview, got {other:?}"),
    }
}

#[test]
fn claude_copilot_and_codex_folder_trust_key_selection() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/tmp/work"));

    // Claude Code unnumbered cursor list: focused on "No, exit", must move Down + Enter.
    let claude_trust = "\
 Accessing workspace:
 /tmp/work
 Quick safety check: Is this a project you created or one you trust?
 ❯ No, exit
   Yes, I trust this folder
 Enter to confirm · Esc to cancel";
    let prompt = parse_startup_prompt(claude_trust).expect("claude trust");
    assert_eq!(prompt.class, StartupPromptClass::FolderTrust);
    assert_eq!(prompt.key_mapping, KeyMappingStyle::CursorList);
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PressKeys {
            keys, chosen_label, ..
        } => {
            assert_eq!(keys, vec![KeyInput::Down, KeyInput::Enter]);
            assert_eq!(chosen_label, "Yes, I trust this folder");
        }
        other => panic!("expected Down+Enter, got {other:?}"),
    }

    // Copilot numbered folder trust: must pick "1. Yes" (Enter), NEVER "2. Yes, and remember..."
    let copilot_trust = "\
╭──────────────────────────────────────────────────────────╮
│ Confirm folder trust                                     │
│ /tmp/work                                                │
│ Do you trust the files in this folder?                   │
│ ❯ 1. Yes                                                 │
│   2. Yes, and remember this folder for future sessions   │
│   3. No (Esc)                                            │
╰──────────────────────────────────────────────────────────╯";
    let prompt = parse_startup_prompt(copilot_trust).expect("copilot trust");
    assert_eq!(prompt.class, StartupPromptClass::FolderTrust);
    assert_eq!(prompt.key_mapping, KeyMappingStyle::NumberedMenu);
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PressKeys {
            keys, chosen_label, ..
        } => {
            assert_eq!(keys, vec![KeyInput::Enter]);
            assert_eq!(chosen_label, "Yes");
        }
        other => panic!("expected Enter for Yes, got {other:?}"),
    }

    // Codex folder trust: focused on "1. Trust and continue" -> Enter.
    let codex_trust = "\
  Folder access
  /tmp/work
  Trust this folder? Codex can read, edit, and run files here, subject to your permission settings.
› 1. Trust and continue
  2. Back to Agent Command Center
  enter continue · esc back";
    let prompt = parse_startup_prompt(codex_trust).expect("codex trust");
    assert_eq!(prompt.class, StartupPromptClass::FolderTrust);
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PressKeys {
            keys, chosen_label, ..
        } => {
            assert_eq!(keys, vec![KeyInput::Enter]);
            assert_eq!(chosen_label, "Trust and continue");
        }
        other => panic!("expected Enter, got {other:?}"),
    }
}

#[test]
fn workspace_scope_mismatch_pauses_for_review() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/Users/owner/work"));
    let other_repo_trust = "\
 Accessing workspace:
 /other/repo
 Quick safety check: Is this a project you created or one you trust?
 ❯ No, exit
   Yes, I trust this folder";
    let prompt = parse_startup_prompt(other_repo_trust).expect("trust prompt");
    assert_eq!(prompt.detected_workspace.as_deref(), Some("/other/repo"));
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PauseForReview(summary) => {
            assert_eq!(summary.reason, PauseReason::OutOfGrantedScope);
            assert_eq!(summary.target, "/other/repo");
            assert!(summary.requires_smart_agent_review);
        }
        other => panic!("expected OutOfGrantedScope pause, got {other:?}"),
    }
}

#[test]
fn destructive_external_mcp_and_login_prompts_never_auto_approve() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/tmp/work"));
    let dangerous_screens = [
        (
            "Do you authorize me to run `rm -rf /tmp/work`? [y/N]",
            PauseReason::DestructiveOrExternalSignal,
            Reversibility::Irreversible,
        ),
        (
            "Do you authorize me to run `git push --force origin main`? [y/N]",
            PauseReason::DestructiveOrExternalSignal,
            Reversibility::Irreversible,
        ),
        (
            "Do you authorize sudo access for this session? [y/N]",
            PauseReason::DestructiveOrExternalSignal,
            Reversibility::Irreversible,
        ),
        (
            "Do you authorize connecting to https://api.example.com? [y/N]",
            PauseReason::DestructiveOrExternalSignal,
            Reversibility::Irreversible,
        ),
        (
            "Do you authorize always allow for all workspaces? [y/N]",
            PauseReason::DestructiveOrExternalSignal,
            Reversibility::Irreversible,
        ),
        (
            "Allow the screenparse MCP server to run tool \"echo\"?\n› 1. Allow\n  2. Cancel",
            PauseReason::OutOfGrantedScope,
            Reversibility::Unknown,
        ),
        (
            "Welcome to Codex\nChoose how you want to use Codex.\n> 1. Sign in with ChatGPT\n  2. Sign in with Device Code",
            PauseReason::LoginOrExternalAuth,
            Reversibility::PartiallyReversible,
        ),
    ];

    for (screen, expected_reason, expected_rev) in dangerous_screens {
        let prompt = parse_startup_prompt(screen).expect("prompt should parse");
        match decide_startup_action(&prompt, &scope) {
            StartupDecision::PauseForReview(summary) => {
                assert_eq!(summary.reason, expected_reason, "screen: {screen}");
                assert_eq!(summary.reversibility, expected_rev, "screen: {screen}");
                assert!(!summary.action.is_empty());
                assert!(!summary.target.is_empty());
                assert!(!summary.scope.is_empty());
                assert!(!summary.duration.is_empty());
                assert!(!summary.changed_risk_factors.is_empty());
                assert!(summary.requires_smart_agent_review);
            }
            other => panic!("must never auto-approve `{screen}`, got {other:?}"),
        }
    }
}

#[test]
fn ambiguous_prompt_fails_safe_by_pausing() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/tmp/work"));
    let ambiguous = "Continue with experimental migration? [y/N]";
    let prompt = parse_startup_prompt(ambiguous).expect("ambiguous prompt");
    match decide_startup_action(&prompt, &scope) {
        StartupDecision::PauseForReview(summary) => {
            assert_eq!(summary.reason, PauseReason::AmbiguousPrompt);
            assert!(summary.requires_smart_agent_review);
        }
        other => panic!("expected AmbiguousPrompt pause, got {other:?}"),
    }
}

#[test]
fn deterministic_jittered_schedule_and_bounds() {
    let seed_a = seed_for_session("se_123", 4242);
    let seed_b = seed_for_session("se_123", 4242);
    let seed_c = seed_for_session("se_999", 4242);
    assert_eq!(seed_a, seed_b);
    assert_ne!(seed_a, seed_c);

    let cfg_a = StartupConfig {
        initial_wait: Duration::from_millis(100),
        base_retry_delay: Duration::from_millis(200),
        max_retry_delay: Duration::from_millis(1_000),
        max_total_wait: Duration::from_millis(2_500),
        max_attempts: 6,
        jitter_ratio_bps: 2_500,
        seed: seed_a,
    };
    let sched_a1 = StartupSchedule::new(cfg_a.clone()).expect("valid config");
    let sched_a2 = StartupSchedule::new(cfg_a.clone()).expect("valid config");
    let sched_c = StartupSchedule::new(StartupConfig {
        seed: seed_c,
        ..cfg_a
    })
    .expect("valid config");

    let seq_a1: Vec<Duration> = (0..6).map(|i| sched_a1.delay_for_attempt(i)).collect();
    let seq_a2: Vec<Duration> = (0..6).map(|i| sched_a2.delay_for_attempt(i)).collect();
    let seq_c: Vec<Duration> = (0..6).map(|i| sched_c.delay_for_attempt(i)).collect();

    assert_eq!(seq_a1, seq_a2);
    assert_ne!(seq_a1, seq_c);
    for d in seq_a1.iter().chain(seq_c.iter()) {
        assert!(*d <= Duration::from_millis(1_000));
    }

    assert!(sched_a1.bounded_delay(6, Duration::ZERO).is_none());
    assert!(sched_a1
        .bounded_delay(0, Duration::from_millis(2_500))
        .is_none());
    assert_eq!(
        sched_a1.bounded_delay(0, Duration::from_millis(2_450)),
        Some(Duration::from_millis(50))
    );
}

#[tokio::test(start_paused = true)]
async fn runner_handles_startup_prompt_and_records_structured_audit_trail() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/tmp/work"));
    let terminal = ScriptedTerminal::new(vec![
        None,
        Some((
            10,
            "Antigravity (agy)\nDo you authorize me for this session? [y/N]".to_owned(),
        )),
        Some((20, "❯ ".to_owned())),
    ]);
    let config = StartupConfig {
        initial_wait: Duration::from_millis(100),
        base_retry_delay: Duration::from_millis(150),
        max_retry_delay: Duration::from_millis(500),
        max_total_wait: Duration::from_secs(5),
        max_attempts: 6,
        jitter_ratio_bps: 2_000,
        seed: seed_for_session("se_agy", 1001),
    };
    let runner = StartupAuthRunner::new("se_agy", "agy", config).expect("runner");
    let report = runner.run(&terminal, &scope).await;

    assert_eq!(report.status, StartupStatus::Ready);
    assert_eq!(report.total_keys_sent, 2);
    assert_eq!(
        terminal.sent(),
        vec![vec![KeyInput::Char('y'), KeyInput::Enter]]
    );
    assert_eq!(report.audit.entries.len(), 3);
    assert_eq!(report.audit.entries[0].outcome, AuditOutcome::NoPromptYet);
    assert_eq!(report.audit.entries[1].outcome, AuditOutcome::KeysSent);
    assert!(!report.audit.entries[1].screen_hash.is_empty());
    assert!(!report.audit.entries[1].screen_excerpt.is_empty());
    assert_eq!(
        report.audit.entries[1].classification,
        Some(StartupPromptClass::StartupAuthorization)
    );
    assert_eq!(
        report.audit.entries[1].keys_pressed,
        vec![KeyInput::Char('y'), KeyInput::Enter]
    );
    assert_eq!(report.audit.entries[2].outcome, AuditOutcome::Ready);
}

#[tokio::test(start_paused = true)]
async fn runner_pauses_on_stuck_screen_instead_of_looping() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/tmp/work"));
    let stuck_prompt = "Antigravity (agy)\nDo you authorize me for this session? [y/N]".to_owned();
    let terminal = ScriptedTerminal::new(vec![
        Some((10, stuck_prompt.clone())),
        Some((10, stuck_prompt)),
    ]);
    let runner = StartupAuthRunner::new("se_stuck", "agy", StartupConfig::default()).expect("runner");
    let report = runner.run(&terminal, &scope).await;

    match report.status {
        StartupStatus::PausedForReview(summary) => {
            assert_eq!(summary.reason, PauseReason::StuckAfterKeyPress);
            assert!(summary.requires_smart_agent_review);
        }
        other => panic!("expected StuckAfterKeyPress, got {other:?}"),
    }
    assert_eq!(terminal.sent().len(), 1);
    assert_eq!(
        report.audit.entries.last().map(|e| e.outcome),
        Some(AuditOutcome::PausedForReview)
    );
}

#[tokio::test(start_paused = true)]
async fn runner_times_out_cleanly_when_never_ready() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/tmp/work"));
    let terminal = ScriptedTerminal::new(vec![Some((1, "Loading...".to_owned()))]);
    let config = StartupConfig {
        initial_wait: Duration::from_millis(50),
        base_retry_delay: Duration::from_millis(100),
        max_retry_delay: Duration::from_millis(200),
        max_total_wait: Duration::from_millis(400),
        max_attempts: 4,
        jitter_ratio_bps: 1_000,
        seed: 7,
    };
    let runner = StartupAuthRunner::new("se_timeout", "agy", config).expect("runner");
    let report = runner.run(&terminal, &scope).await;

    match report.status {
        StartupStatus::PausedForReview(summary) => {
            assert_eq!(summary.reason, PauseReason::TimeoutExceeded);
            assert!(summary.requires_smart_agent_review);
        }
        other => panic!("expected TimeoutExceeded, got {other:?}"),
    }
}

#[test]
fn screen_ask_inspect_startup_screen_hook_delegates_to_startup_auth() {
    let scope = OwnerConsentScope::local_workspace(PathBuf::from("/tmp/work"));
    let screen = "Do you authorize me for this session? [y/N]";
    let (prompt, decision) =
        ScreenAsk::inspect_startup_screen(screen, &scope).expect("startup prompt detected");
    assert_eq!(prompt.class, StartupPromptClass::StartupAuthorization);
    assert!(matches!(decision, StartupDecision::PressKeys { .. }));
    assert!(ScreenAsk::inspect_startup_screen("❯ ", &scope).is_none());
}
