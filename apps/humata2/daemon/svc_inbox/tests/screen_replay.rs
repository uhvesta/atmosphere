//! Fast-tier deterministic replay tests for the SCREENPARSE recorded corpus.
//!
//! Feeds every real recorded terminal screen in `tests/screens/` through
//! `screen_question` and `ScreenAsk` with zero model calls and zero sleeps.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use humata2_daemon_runtime::screen_question::{
    self, PromptKind, ScreenQuestion,
};
use humata2_screenparse::{Expect, Snapshot, read_snapshot};
use humata2_store::{AgentLaunch, AgentRecord, AgentState, SessionRecord, Store, TabKind, TabRecord};
use humata2_svc_inbox::{
    AskingGuardPort, HookAskPort, NoopAsking, NoopHookAsk, ScreenAsk, ScreenHoldersPort,
};
use tokio::sync::Notify;
use tonic::Status;

const HOLDER_PID: u32 = 4242;

fn screens_root() -> PathBuf {
    if let (Ok(srcdir), Ok(workspace)) = (
        std::env::var("TEST_SRCDIR"),
        std::env::var("TEST_WORKSPACE"),
    ) {
        let candidate = PathBuf::from(srcdir)
            .join(workspace)
            .join("apps/humata2/daemon/svc_inbox/tests/screens");
        if candidate.is_dir() {
            return candidate;
        }
    }
    let rel = PathBuf::from("apps/humata2/daemon/svc_inbox/tests/screens");
    if rel.is_dir() {
        return rel;
    }
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let candidate = PathBuf::from(manifest).join("tests/screens");
        if candidate.is_dir() {
            return candidate;
        }
    }
    panic!("could not locate tests/screens directory in runfiles or workspace");
}

fn collect_snapshot_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_snapshot_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("txt") {
            out.push(path);
        }
    }
}

fn load_snapshot(rel: &str) -> Snapshot {
    let path = screens_root().join(rel);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    read_snapshot(&text)
        .unwrap_or_else(|e| panic!("parse snapshot {}: {e}", path.display()))
}

#[test]
fn every_recorded_screen_matches_its_expected_prompt_kind_and_choices() {
    let root = screens_root();
    let mut files = Vec::new();
    collect_snapshot_files(&root, &mut files);
    assert!(
        !files.is_empty(),
        "expected recorded screens under {}",
        root.display()
    );

    let mut checked = 0usize;
    let mut prompt_count = 0usize;
    let mut none_count = 0usize;

    for path in &files {
        let rel = path.strip_prefix(&root).unwrap_or(path);
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", rel.display()));
        let snap = read_snapshot(&text)
            .unwrap_or_else(|e| panic!("parse {}: {e}", rel.display()));

        assert!(
            !snap.screen.trim().is_empty(),
            "{}: empty screen capture should be pruned from corpus",
            rel.display()
        );

        let parsed = screen_question::prompt_on(&snap.screen);
        let active = screen_question::has_active_prompt(&snap.screen);
        assert_eq!(
            active,
            parsed.is_some(),
            "{}: has_active_prompt must agree with prompt_on",
            rel.display()
        );

        match &snap.expect {
            Expect::None => {
                none_count += 1;
                assert!(
                    parsed.is_none(),
                    "{}: expected no prompt, got {parsed:?}",
                    rel.display()
                );
                assert!(
                    screen_question::asked_on(&snap.screen).is_none(),
                    "{}: expected no ScreenQuestion on idle screen",
                    rel.display()
                );
            }
            Expect::Offer => {
                assert_eq!(
                    screen_question::asked_on(&snap.screen),
                    Some(ScreenQuestion::Offer),
                    "{}: expected ScreenQuestion::Offer",
                    rel.display()
                );
                assert!(
                    parsed.is_none(),
                    "{}: offers must not trigger active prompt",
                    rel.display()
                );
            }
            Expect::Prompt(expected_kind) => {
                prompt_count += 1;
                let prompt = parsed.unwrap_or_else(|| {
                    panic!(
                        "{}: expected prompt:{expected_kind}, got None",
                        rel.display()
                    )
                });
                assert_eq!(
                    prompt.kind.as_str(),
                    expected_kind.as_str(),
                    "{}: wrong prompt kind",
                    rel.display()
                );
                if prompt.kind == PromptKind::FolderTrust {
                    assert_eq!(
                        screen_question::asked_on(&snap.screen),
                        Some(ScreenQuestion::FolderTrust),
                        "{}: folder trust must also be recognized by asked_on",
                        rel.display()
                    );
                }
                assert!(
                    prompt.choices.len() >= 2,
                    "{}: expected at least 2 choices for prompt:{expected_kind}, got {:?}",
                    rel.display(),
                    prompt.choices
                );
            }
            Expect::Unknown => {
                panic!(
                    "{}: committed corpus snapshot still has `expect: unknown`",
                    rel.display()
                );
            }
        }
        checked += 1;
    }

    assert!(checked >= 100, "expected full multi-agent corpus, got {checked}");
    assert!(prompt_count > 0, "expected at least one prompt snapshot");
    assert!(none_count > 0, "expected at least one non-prompt snapshot");
}

#[test]
fn representative_recorded_prompts_extract_expected_choices_across_widths() {
    for width in [60, 80, 120, 200] {
        // Claude Code 2.1.329 folder trust.
        let claude_trust = load_snapshot(&format!(
            "claude/2.1.329/bash.w{width}.01-launch.txt"
        ));
        let p = screen_question::prompt_on(&claude_trust.screen).expect("claude trust");
        assert_eq!(p.kind, PromptKind::FolderTrust);
        assert_eq!(p.choices, vec!["No, exit", "Yes, I trust this folder"]);

        // Claude Code 2.1.329 theme onboarding.
        let claude_onboarding = load_snapshot(&format!(
            "claude/2.1.329/login.w{width}.01-no_login.txt"
        ));
        let p = screen_question::prompt_on(&claude_onboarding.screen).expect("claude onboarding");
        assert_eq!(p.kind, PromptKind::Onboarding);
        assert_eq!(p.choices.first().map(String::as_str), Some("Auto (match terminal)"));
        assert!(p.choices.len() >= 5, "w{width}: {:?}", p.choices);

        // Codex 0.169.0 folder trust.
        let codex_trust = load_snapshot(&format!(
            "codex/0.169.0/bash.w{width}.02-bash_permission.txt"
        ));
        let p = screen_question::prompt_on(&codex_trust.screen).expect("codex trust");
        assert_eq!(p.kind, PromptKind::FolderTrust);
        assert_eq!(
            p.choices,
            vec!["Trust and continue", "Back to Agent Command Center"]
        );

        // Codex 0.169.0 command permission.
        let codex_perm = load_snapshot(&format!(
            "codex/0.169.0/edit.w{width}.03-after_answer.txt"
        ));
        let p = screen_question::prompt_on(&codex_perm.screen).expect("codex perm");
        assert_eq!(p.kind, PromptKind::Permission);
        assert_eq!(p.choices.len(), 3, "w{width}: {:?}", p.choices);
        assert_eq!(p.choices[0], "Yes, proceed (y)");

        // Codex 0.169.0 MCP tool permission.
        let codex_mcp = load_snapshot(&format!(
            "codex/0.169.0/mcp.w{width}.02-mcp_permission.txt"
        ));
        let p = screen_question::prompt_on(&codex_mcp.screen).expect("codex mcp");
        assert_eq!(p.kind, PromptKind::Mcp);
        assert_eq!(p.choices.len(), 4, "w{width}: {:?}", p.choices);
        assert!(p.choices[0].starts_with("Allow"));
        assert!(p.choices[3].starts_with("Cancel"));

        // Codex 0.169.0 login picker.
        let codex_login = load_snapshot(&format!(
            "codex/0.169.0/login.w{width}.02-after_escape.txt"
        ));
        let p = screen_question::prompt_on(&codex_login.screen).expect("codex login");
        assert_eq!(p.kind, PromptKind::Login);
        assert_eq!(
            p.choices,
            vec![
                "Sign in with ChatGPT",
                "Sign in with Device Code",
                "Sign in another way",
            ]
        );

        // Copilot 1.0.115-1 folder trust.
        let copilot_trust = load_snapshot(&format!(
            "copilot/1.0.115-1/probe.w{width}.01-launch.txt"
        ));
        let p = screen_question::prompt_on(&copilot_trust.screen).expect("copilot trust");
        assert_eq!(p.kind, PromptKind::FolderTrust);
        assert_eq!(
            p.choices,
            vec![
                "Yes",
                "Yes, and remember this folder for future sessions",
                "No (Esc)",
            ]
        );
    }
}

struct ReplayHolders {
    state: std::sync::Mutex<(u64, String)>,
    fetches: AtomicUsize,
    notify: Notify,
}

impl ReplayHolders {
    fn new(end: u64, screen: String) -> Self {
        Self {
            state: std::sync::Mutex::new((end, screen)),
            fetches: AtomicUsize::new(0),
            notify: Notify::new(),
        }
    }

    fn set(&self, end: u64, screen: String) {
        *self.state.lock().unwrap() = (end, screen);
    }

    async fn wait_for_fetches(&self, target: usize) {
        loop {
            let notified = self.notify.notified();
            if self.fetches.load(Ordering::SeqCst) >= target {
                return;
            }
            notified.await;
        }
    }
}

#[tonic::async_trait]
impl ScreenHoldersPort for ReplayHolders {
    async fn screen_at(&self, _socket: &Path) -> Result<String, Status> {
        Ok(self.state.lock().unwrap().1.clone())
    }

    async fn screen_for_tab(
        &self,
        _tab: &str,
        _socket: &Path,
        last_end: Option<u64>,
    ) -> Result<Option<(u64, String)>, Status> {
        let (end, screen) = self.state.lock().unwrap().clone();
        if last_end == Some(end) {
            return Ok(None);
        }
        self.fetches.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
        Ok(Some((end, screen)))
    }
}

struct ReplayAsking {
    held: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

struct ReplayGuard {
    held: Arc<AtomicUsize>,
    notify: Arc<Notify>,
}

impl Drop for ReplayGuard {
    fn drop(&mut self) {
        self.held.fetch_sub(1, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
}

impl ReplayAsking {
    fn new() -> Self {
        Self {
            held: Arc::new(AtomicUsize::new(0)),
            notify: Arc::new(Notify::new()),
        }
    }

    fn is_held(&self) -> bool {
        self.held.load(Ordering::SeqCst) > 0
    }

    async fn wait_held(&self) {
        loop {
            let notified = self.notify.notified();
            if self.is_held() {
                return;
            }
            notified.await;
        }
    }

    async fn wait_released(&self) {
        loop {
            let notified = self.notify.notified();
            if !self.is_held() {
                return;
            }
            notified.await;
        }
    }
}

impl AskingGuardPort for ReplayAsking {
    fn hold(&self, _session: &str) -> Box<dyn Send + Sync> {
        self.held.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
        Box::new(ReplayGuard {
            held: Arc::clone(&self.held),
            notify: Arc::clone(&self.notify),
        })
    }
}

struct ReplayHookAsk {
    pending: AtomicBool,
    notify: Notify,
}

impl ReplayHookAsk {
    fn new(pending: bool) -> Self {
        Self {
            pending: AtomicBool::new(pending),
            notify: Notify::new(),
        }
    }

    fn is_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst)
    }

    async fn wait_resolved(&self) {
        loop {
            let notified = self.notify.notified();
            if !self.is_pending() {
                return;
            }
            notified.await;
        }
    }
}

#[tonic::async_trait]
impl HookAskPort for ReplayHookAsk {
    async fn has_pending_hook_ask(&self, _session: &str) -> bool {
        self.pending.load(Ordering::SeqCst)
    }

    async fn resolve_pending_hook_asks(&self, _session: &str) {
        self.pending.store(false, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
}

fn setup_store(session: &str, agent_kind: &str) -> (tempfile::TempDir, Arc<Store>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(Store::open(dir.path().join("state")).expect("open store"));
    store
        .put_session(&SessionRecord {
            id: session.to_owned(),
            channel: "ch_1".to_owned(),
            machine: "local".to_owned(),
            title: "replay".to_owned(),
            agent: agent_kind.to_owned(),
            folder: dir.path().to_path_buf(),
            created_unix_ms: 1,
        })
        .expect("put_session");
    let tab = TabRecord {
        id: format!("tab_{session}"),
        session: Some(session.to_owned()),
        kind: TabKind::Agent,
        title: agent_kind.to_owned(),
        holder_socket: PathBuf::from(format!("/tmp/{session}.sock")),
        holder_pid: HOLDER_PID,
        created_unix_ms: 1,
        holder_generation: 0,
    };
    let agent = AgentRecord {
        session: session.to_owned(),
        tab: tab.id.clone(),
        launch: AgentLaunch {
            agent: agent_kind.to_owned(),
            ..AgentLaunch::default()
        },
        state: AgentState::Running {
            holder_pid: HOLDER_PID,
        },
        generation: 1,
        interruptions: Vec::new(),
        updated_unix_ms: 1,
    };
    store
        .put_agent_and_tab(&agent, &tab)
        .expect("put_agent_and_tab");
    (dir, store)
}

#[tokio::test(start_paused = true)]
async fn screen_ask_raises_and_clears_folder_trust_on_recorded_transitions() {
    let transitions = [
        (
            "claude",
            "claude/2.1.329/bash.w120.01-launch.txt",
            "claude/2.1.329/bash.w120.02-bash_permission.txt",
        ),
        (
            "codex",
            "codex/0.169.0/bash.w120.02-bash_permission.txt",
            "codex/0.169.0/bash.w120.03-after_escape.txt",
        ),
        (
            "copilot",
            "copilot/1.0.115-1/edit.w120.01-launch.txt",
            "copilot/1.0.115-1/edit.w120.02-edit_permission.txt",
        ),
    ];

    for (agent, prompt_file, cleared_file) in transitions {
        let prompt_snap = load_snapshot(prompt_file);
        let cleared_snap = load_snapshot(cleared_file);
        let session = format!("se_trust_{agent}");
        let (_dir, store) = setup_store(&session, agent);
        let holders = Arc::new(ReplayHolders::new(100, prompt_snap.screen));
        let asking = Arc::new(ReplayAsking::new());
        let watcher = ScreenAsk::new_with_hook_asks(
            holders.clone(),
            asking.clone(),
            store,
            Arc::new(|_| false),
            Arc::new(|_| {}),
            Arc::new(NoopHookAsk),
        );

        // Prompt appears on screen -> ScreenAsk holds Needs you.
        watcher.quiet(&session, HOLDER_PID);
        asking.wait_held().await;
        assert!(asking.is_held(), "{agent}: folder trust should hold Needs you");

        // User answers or presses Escape -> screen clears, output advances.
        holders.set(200, cleared_snap.screen);
        watcher.quiet(&session, HOLDER_PID);
        holders.wait_for_fetches(2).await;
        tokio::task::yield_now().await;

        // Debounce elapses -> ScreenAsk releases the hold.
        tokio::time::advance(Duration::from_secs(3)).await;
        asking.wait_released().await;
        assert!(
            !asking.is_held(),
            "{agent}: folder trust hold should clear once prompt is gone"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn screen_ask_clears_hook_ask_when_recorded_mcp_prompt_is_answered() {
    let prompt_snap = load_snapshot("codex/0.169.0/mcp.w120.02-mcp_permission.txt");
    let cleared_snap = load_snapshot("codex/0.169.0/mcp.w120.03-after_answer.txt");

    let session = "se_codex_mcp";
    let (_dir, store) = setup_store(session, "codex");
    let holders = Arc::new(ReplayHolders::new(100, prompt_snap.screen));
    let hook_asks = Arc::new(ReplayHookAsk::new(true));
    let watcher = ScreenAsk::new_with_hook_asks(
        holders.clone(),
        Arc::new(NoopAsking),
        store,
        Arc::new(|_| true),
        Arc::new(|_| {}),
        hook_asks.clone(),
    );

    // MCP permission dialog is visible -> Signal B sees it and keeps ask open.
    watcher.track_hook_ask(session);
    watcher.quiet(session, HOLDER_PID);
    holders.wait_for_fetches(1).await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(5)).await;
    tokio::task::yield_now().await;
    assert!(
        hook_asks.is_pending(),
        "MCP permission prompt must stay pending while visible on screen"
    );

    // User answers -> screen moves to tool output + ready input.
    holders.set(200, cleared_snap.screen);
    watcher.quiet(session, HOLDER_PID);
    holders.wait_for_fetches(2).await;
    tokio::task::yield_now().await;

    // Debounce elapses -> Signal B resolves the stale hook ask.
    tokio::time::advance(Duration::from_secs(3)).await;
    hook_asks.wait_resolved().await;
    assert!(
        !hook_asks.is_pending(),
        "MCP hook ask must auto-clear once the prompt is gone"
    );
}
