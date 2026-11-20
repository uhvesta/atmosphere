//! D845 / 0.6.1 HOOKCLEAR: watches supported agents' live terminal screens
//! (`claude`, `codex`, `copilot`, `agy`) for:
//!
//! 1. Screen-raised dialogs that fire no hook (such as Copilot's folder trust):
//!    when the agent's output goes quiet ([`ScreenAsk::quiet`]),
//!    [`screen_question::asked_on`] checks whether a `needs_you` dialog is up
//!    and holds `NEEDS_INPUT` until the dialog clears.
//! 2. Stale hook-raised and screen-raised ask auto-clear (0.6.1 HOOKCLEAR):
//!    - **Signal A (Lifecycle hook):** a subsequent lifecycle event for a
//!      supported agent immediately resolves pending hook-raised asks as
//!      `Resolved` (never `Denied`) and cancels any pending debounce.
//!    - **Signal B (Live TUI prompt-gone + progress debounce):** for supported
//!      agents (`claude`, `codex`, `copilot`, `agy`), if a pending hook-raised
//!      or screen-raised ask is no longer visible on the live terminal screen
//!      AND the terminal output `end` offset has advanced, a debounce timer
//!      ([`CLEAR_DEBOUNCE`], 2.5s) starts. If the prompt reappears before the
//!      debounce expires (e.g., transient redraw flicker), the debounce is
//!      cancelled immediately. Only if the prompt remains absent after the
//!      debounce is the stale ask auto-cleared.
//!
//! Zero performance regression: reuses `Watching::last_seen` `(holder_pid, end)`
//! and live `Link` screen text via [`ScreenHoldersPort::screen_for_tab`]. When
//! `(holder_pid, end)` is unchanged and no debounce timer is pending, 0 work is
//! performed.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock, PoisonError};
use std::time::Duration;

use humata2_daemon_runtime::screen_question::{self, ScreenQuestion};
use humata2_store::{AgentState, Store, StoreError, TabKind, TabRecord};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

use crate::ports::{AskingGuardPort, HookAskPort, NoopHookAsk, ScreenHoldersPort};

/// The most often an agent's output has its screen read again while an ask is
/// active. A throttle, not a wait for quiet: output that never stops still gets
/// a read this often.
const RELOOK_EVERY: Duration = Duration::from_millis(300);

/// How long a pending ask's prompt must stay absent on the live terminal screen
/// (after terminal output has advanced) before auto-clearing (0.6.1 HOOKCLEAR).
pub(crate) const CLEAR_DEBOUNCE: Duration = Duration::from_millis(2500);

/// Agent kinds strictly scoped to 0.6.1 HOOKCLEAR auto-clear.
pub(crate) const SUPPORTED_AGENTS: &[&str] = &["claude", "codex", "copilot", "agy"];

/// Returns `true` when `agent` is one of the 0.6.1 HOOKCLEAR supported kinds
/// (`claude`, `codex`, `copilot`, `agy`).
pub(crate) fn is_supported_agent(agent: &str) -> bool {
    SUPPORTED_AGENTS.contains(&agent)
}

static ASKS: OnceLock<ScreenAsk> = OnceLock::new();

/// What looks at agents' screens, for the daemon's life.
#[derive(Clone)]
pub struct ScreenAsk(Arc<Asks>);

struct Asks {
    holders: Arc<dyn ScreenHoldersPort>,
    asking: Arc<dyn AskingGuardPort>,
    store: Arc<Store>,
    is_heard: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    forget: Arc<dyn Fn(&str) + Send + Sync>,
    hook_asks: Arc<dyn HookAskPort>,
    clear_debounce: Duration,
    /// The dialog each session's screen shows, by session.
    open: Mutex<HashMap<String, Open>>,
    /// Which sessions' output reads their screen again. Never held across
    /// an await.
    watching: std::sync::Mutex<Watching>,
}

/// Sessions whose ask is active, so their output reads the screen again, and
/// those with a read already due. Also tracks the last `(holder_pid, end)`
/// inspected per session so unchanged screens are not rebuilt, plus any
/// pending prompt-gone debounce timer.
#[derive(Default)]
struct Watching {
    asking: HashSet<String>,
    hook_asking: HashSet<String>,
    due: HashSet<String>,
    last_seen: HashMap<String, (u32, u64)>,
    pending_clear: HashMap<String, PendingClear>,
    next_generation: u64,
}

/// An armed prompt-gone debounce timer for a session. Dropping it while armed
/// aborts the sleeping timer immediately.
struct PendingClear {
    generation: u64,
    task: Option<JoinHandle<()>>,
}

impl PendingClear {
    fn disarm(&mut self) {
        self.task.take();
    }
}

impl Drop for PendingClear {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

/// A dialog a session's screen shows, and the hold on its Needs you.
struct Open {
    holder_pid: u32,
    question: ScreenQuestion,
    /// Holds the session's Needs you; aborted (and so let go) when a look
    /// sees the dialog gone or the agent is.
    task: JoinHandle<()>,
}

impl Drop for Open {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl ScreenAsk {
    /// Creates a new [`ScreenAsk`] watcher.
    pub fn new(
        holders: Arc<dyn ScreenHoldersPort>,
        asking: Arc<dyn AskingGuardPort>,
        store: Arc<Store>,
        is_heard: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        forget: Arc<dyn Fn(&str) + Send + Sync>,
    ) -> Self {
        Self::new_with_hook_asks(holders, asking, store, is_heard, forget, Arc::new(NoopHookAsk))
    }

    /// Creates a new [`ScreenAsk`] watcher wired to `hook_asks` for 0.6.1
    /// HOOKCLEAR stale hook-ask auto-clear.
    pub fn new_with_hook_asks(
        holders: Arc<dyn ScreenHoldersPort>,
        asking: Arc<dyn AskingGuardPort>,
        store: Arc<Store>,
        is_heard: Arc<dyn Fn(&str) -> bool + Send + Sync>,
        forget: Arc<dyn Fn(&str) + Send + Sync>,
        hook_asks: Arc<dyn HookAskPort>,
    ) -> Self {
        Self(Arc::new(Asks {
            holders,
            asking,
            store,
            is_heard,
            forget,
            hook_asks,
            clear_debounce: CLEAR_DEBOUNCE,
            open: Mutex::new(HashMap::new()),
            watching: std::sync::Mutex::new(Watching::default()),
        }))
    }

    /// Marks `session` as having a pending hook-raised ask so `output()` checks
    /// its live screen on output progress.
    pub fn track_hook_ask(&self, session: &str) {
        self.0.watching().hook_asking.insert(session.to_owned());
    }

    /// Stops watching `session` for pending hook-raised asks and cancels any
    /// pending debounce timer.
    pub fn untrack_hook_ask(&self, session: &str) {
        let mut watching = self.0.watching();
        watching.hook_asking.remove(session);
        if !watching.asking.contains(session) {
            watching.pending_clear.remove(session);
        }
    }

    /// 0.6.1 HOOKCLEAR (Signal A): a subsequent lifecycle event for `session`
    /// arrived for `agent_kind`. For supported agents (`claude`, `codex`,
    /// `copilot`, `agy`), immediately resolves pending hook-raised asks and
    /// cancels any pending debounce. Unsupported agents are left untouched.
    pub async fn on_lifecycle(&self, session: &str, agent_kind: &str) {
        if !is_supported_agent(agent_kind) {
            return;
        }
        self.0.cancel_clear(session);
        if self.0.hook_asks.has_pending_hook_ask(session).await {
            self.0.hook_asks.resolve_pending_hook_asks(session).await;
        }
        self.0.watching().hook_asking.remove(session);
    }

    /// `session`'s agent, under holder `holder_pid`, wrote output: while a
    /// screen-raised or hook-raised ask is active, reads the screen again soon.
    /// Returns at once.
    pub fn output(&self, session: &str, holder_pid: u32) {
        let asks = &self.0;
        {
            let mut watching = asks.watching();
            let active =
                watching.asking.contains(session) || watching.hook_asking.contains(session);
            if !active || !watching.due.insert(session.to_owned()) {
                return;
            }
        }
        let asks = Arc::clone(asks);
        let session = session.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(RELOOK_EVERY).await;
            // Due no more before the read, so output during it asks again.
            asks.watching().due.remove(&session);
            asks.look(&session, holder_pid).await;
        });
    }

    /// `session`'s agent, under holder `holder_pid`, went quiet: looks at its
    /// screen for a dialog or stale ask. Returns at once.
    pub fn quiet(&self, session: &str, holder_pid: u32) {
        let asks = Arc::clone(&self.0);
        let session = session.to_owned();
        tokio::spawn(async move { asks.look(&session, holder_pid).await });
    }

    /// `session`'s agent exited or its holder is gone: ends its hold and any
    /// pending debounce.
    pub fn gone(&self, session: &str) {
        (self.0.forget)(session);
        let asks = Arc::clone(&self.0);
        let session = session.to_owned();
        tokio::spawn(async move {
            let mut open = asks.open.lock().await;
            open.remove(&session);
            let mut watching = asks.watching();
            watching.asking.remove(&session);
            watching.hook_asking.remove(&session);
            watching.due.remove(&session);
            watching.last_seen.remove(&session);
            watching.pending_clear.remove(&session);
        });
    }

    /// Performs a single screen inspection synchronously (awaitable). Useful
    /// for deterministic testing.
    #[cfg(test)]
    async fn look_now(&self, session: &str, holder_pid: u32) {
        self.0.look(session, holder_pid).await;
    }

    #[cfg(test)]
    async fn has_open_screen_ask(&self, session: &str) -> bool {
        self.0.open.lock().await.contains_key(session)
    }

    #[cfg(test)]
    fn has_pending_clear(&self, session: &str) -> bool {
        self.0.watching().pending_clear.contains_key(session)
    }
}

/// Starts looking at agents' screens. Called once, when the daemon serves.
pub fn start(
    holders: Arc<dyn ScreenHoldersPort>,
    asking: Arc<dyn AskingGuardPort>,
    store: Arc<Store>,
    is_heard: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    forget: Arc<dyn Fn(&str) + Send + Sync>,
    hook_asks: Arc<dyn HookAskPort>,
) {
    let _ = ASKS.set(ScreenAsk::new_with_hook_asks(
        holders, asking, store, is_heard, forget, hook_asks,
    ));
}

/// Marks `session` as having a pending hook-raised ask for Signal B tracking.
pub fn track_hook_ask(session: &str) {
    if let Some(asks) = ASKS.get() {
        asks.track_hook_ask(session);
    }
}

/// Clears Signal B tracking for `session`'s hook-raised asks.
pub fn untrack_hook_ask(session: &str) {
    if let Some(asks) = ASKS.get() {
        asks.untrack_hook_ask(session);
    }
}

/// `session`'s agent, under holder `holder_pid`, wrote output: while an ask is
/// up, reads the screen again soon. Returns at once.
pub fn output(session: &str, holder_pid: u32) {
    if let Some(asks) = ASKS.get() {
        asks.output(session, holder_pid);
    }
}

/// `session`'s agent, under holder `holder_pid`, went quiet: looks at its
/// screen for a dialog. Returns at once.
pub fn quiet(session: &str, holder_pid: u32) {
    if let Some(asks) = ASKS.get() {
        asks.quiet(session, holder_pid);
    }
}

/// `session`'s agent exited or its holder is gone: ends its hold.
pub fn gone(session: &str) {
    if let Some(asks) = ASKS.get() {
        asks.gone(session);
    }
}

/// Keeps `asking` (the session's Needs you) until the task running it is
/// aborted.
async fn hold(asking: Box<dyn Send + Sync>) {
    let _asking = asking;
    std::future::pending::<()>().await;
}

impl Asks {
    fn watching(&self) -> std::sync::MutexGuard<'_, Watching> {
        self.watching.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn cancel_clear(&self, session: &str) {
        self.watching().pending_clear.remove(session);
    }

    fn schedule_clear(self: &Arc<Self>, session: &str, holder_pid: u32) {
        let mut watching = self.watching();
        if watching.pending_clear.contains_key(session) {
            // Keep the original debounce deadline while the prompt stays
            // absent so continuous streaming output cannot starve auto-clear.
            return;
        }
        watching.next_generation = watching.next_generation.wrapping_add(1);
        let generation = watching.next_generation;
        let deadline = tokio::time::Instant::now() + self.clear_debounce;
        let asks = Arc::clone(self);
        let session_owned = session.to_owned();
        let task = tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            asks.confirm_and_clear(&session_owned, holder_pid, generation)
                .await;
        });
        watching.pending_clear.insert(
            session.to_owned(),
            PendingClear {
                generation,
                task: Some(task),
            },
        );
    }

    async fn confirm_and_clear(
        self: &Arc<Self>,
        session: &str,
        holder_pid: u32,
        generation: u64,
    ) {
        let last_end = {
            let mut watching = self.watching();
            let matches = watching
                .pending_clear
                .get(session)
                .is_some_and(|pending| pending.generation == generation);
            if !matches {
                return;
            }
            if let Some(mut pending) = watching.pending_clear.remove(session) {
                pending.disarm();
            }
            watching
                .last_seen
                .get(session)
                .filter(|(pid, _)| *pid == holder_pid)
                .map(|(_, end)| *end)
        };

        let Some((tab, agent_kind)) = self.agent_tab(session, holder_pid).await else {
            return;
        };
        if !is_supported_agent(&agent_kind) {
            return;
        }

        // Re-check the live screen if output moved during the debounce window:
        // if the prompt reappeared, keep the ask open.
        match self
            .holders
            .screen_for_tab(&tab.id, &tab.holder_socket, last_end)
            .await
        {
            Ok(Some((end, screen))) => {
                self.watching()
                    .last_seen
                    .insert(session.to_owned(), (holder_pid, end));
                let seen = screen_question::asked_on(&screen).filter(|q| q.needs_you());
                if seen.is_some() || screen_question::has_active_prompt(&screen) {
                    return;
                }
            }
            Ok(None) => {}
            Err(_) => {
                // Could not verify live screen; err on the side of keeping ask open.
                return;
            }
        }

        {
            let mut open = self.open.lock().await;
            open.remove(session);
        }
        if self.hook_asks.has_pending_hook_ask(session).await {
            self.hook_asks.resolve_pending_hook_asks(session).await;
        }
        let mut watching = self.watching();
        watching.asking.remove(session);
        watching.hook_asking.remove(session);
    }

    async fn look(self: &Arc<Self>, session: &str, holder_pid: u32) {
        let heard = (self.is_heard)(session);
        let has_screen_open = self.open.lock().await.contains_key(session);
        let has_hook_open = self.hook_asks.has_pending_hook_ask(session).await;
        {
            let mut watching = self.watching();
            if has_hook_open {
                watching.hook_asking.insert(session.to_owned());
            } else {
                watching.hook_asking.remove(session);
            }
        }

        // D798: an agent heard on its structured channel asks there; when it
        // has neither an open screen hold nor a pending hook ask, do 0 work.
        if heard && !has_screen_open && !has_hook_open {
            self.cancel_clear(session);
            return;
        }

        let Some((tab, agent_kind)) = self.agent_tab(session, holder_pid).await else {
            return;
        };
        let supported = is_supported_agent(&agent_kind);

        // Strictly scoped to supported agents: when heard on structured channel
        // and only a hook ask is pending for an unsupported agent, do 0 work.
        if heard && !has_screen_open && !supported {
            self.cancel_clear(session);
            return;
        }

        // Zero-work fast path: reuse `Watching::last_seen` `(holder_pid, end)`.
        // When `end` is unchanged, `screen_for_tab` returns `Ok(None)` without
        // fetching or rebuilding the terminal screen.
        let prev_end = self
            .watching()
            .last_seen
            .get(session)
            .filter(|(pid, _)| *pid == holder_pid)
            .map(|(_, end)| *end);

        let (end, screen) = match self
            .holders
            .screen_for_tab(&tab.id, &tab.holder_socket, prev_end)
            .await
        {
            Ok(Some((end, screen))) => {
                self.watching()
                    .last_seen
                    .insert(session.to_owned(), (holder_pid, end));
                (end, screen)
            }
            Ok(None) | Err(_) => return,
        };

        let output_advanced = prev_end.is_some_and(|prev| end > prev);

        // D864: an offer (Claude's auto-mode tip) only stops the input
        // gate; it holds no Needs you.
        let seen = screen_question::asked_on(&screen).filter(|question| question.needs_you());
        let live_prompt = seen.is_some() || screen_question::has_active_prompt(&screen);

        if live_prompt {
            // Prompt is visibly present on the live screen: cancel any pending
            // debounce immediately so genuine asks never flicker or disappear.
            self.cancel_clear(session);
            if let Some(question) = seen.filter(|_| !heard) {
                let mut open = self.open.lock().await;
                if open
                    .get(session)
                    .is_some_and(|now| (now.holder_pid, now.question) == (holder_pid, question))
                {
                    return;
                }
                open.insert(
                    session.to_owned(),
                    Open {
                        holder_pid,
                        question,
                        task: tokio::spawn(hold(self.asking.hold(session))),
                    },
                );
                self.watching().asking.insert(session.to_owned());
            }
            return;
        }

        // Prompt is not visible on the live screen.
        // Unsupported/unknown agents are left untouched by 0.6.1 HOOKCLEAR.
        if !supported {
            return;
        }

        // Signal B: for supported agents (`claude`, `codex`, `copilot`, `agy`),
        // if a pending hook-raised or screen-raised ask is no longer on screen
        // AND terminal output has advanced, arm the debounce timer.
        if (has_screen_open || has_hook_open) && output_advanced {
            self.schedule_clear(session, holder_pid);
        }
    }

    /// The session's agent tab and agent kind, while it runs under `holder_pid`.
    async fn agent_tab(&self, session: &str, holder_pid: u32) -> Option<(TabRecord, String)> {
        let (store, key) = (self.store.clone(), session.to_owned());
        let read =
            tokio::task::spawn_blocking(move || -> Result<Option<(TabRecord, String)>, StoreError> {
                let Some(agent) = store.agent(&key)? else {
                    return Ok(None);
                };
                if agent.state != (AgentState::Running { holder_pid }) {
                    return Ok(None);
                }
                let Some(tab) = store
                    .tab(&agent.tab)?
                    .filter(|tab| tab.kind == TabKind::Agent)
                else {
                    return Ok(None);
                };
                let agent_kind = store
                    .session(&key)?
                    .map(|record| record.agent)
                    .unwrap_or_default();
                Ok(Some((tab, agent_kind)))
            })
            .await;
        read.ok()?.ok()?
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use humata2_store::{AgentLaunch, AgentRecord, SessionRecord};
    use tonic::Status;

    use super::*;
    use crate::ports::NoopAsking;

    const HOLDER_PID: u32 = 4242;
    const FOLDER_TRUST_SCREEN: &str = "\
╭──────────────────────────────────────────────────────────╮
│ Do you trust the files in this folder?                   │
│ ❯ 1. Yes                                                 │
│   2. Yes, and remember this folder for future sessions   │
│   3. No (Esc)                                            │
╰──────────────────────────────────────────────────────────╯";

    const PERMISSION_PROMPT_SCREEN: &str = "\
Bash command
  rm -rf target/tmp
Do you want to proceed?
❯ 1. Yes
  2. Yes, and don't ask again
  3. No";

    const QUESTION_PROMPT_SCREEN: &str = "\
Which database should the service use?
❯ 1. Postgres
  2. SQLite";

    const INPUT_PROMPT_SCREEN: &str = "Allow tool execution `cargo test`? [y/N]";

    const STREAMING_OUTPUT_SCREEN: &str = "\
Running cargo test...
test result: ok. 42 passed; 0 failed;";

    struct MockHolders {
        state: std::sync::Mutex<(u64, String)>,
        screen_fetches: AtomicUsize,
        offset_checks: AtomicUsize,
    }

    impl MockHolders {
        fn new(end: u64, screen: &str) -> Self {
            Self {
                state: std::sync::Mutex::new((end, screen.to_owned())),
                screen_fetches: AtomicUsize::new(0),
                offset_checks: AtomicUsize::new(0),
            }
        }

        fn set(&self, end: u64, screen: &str) {
            let mut guard = self.state.lock().unwrap();
            *guard = (end, screen.to_owned());
        }

        fn fetches(&self) -> usize {
            self.screen_fetches.load(Ordering::SeqCst)
        }
    }

    #[tonic::async_trait]
    impl ScreenHoldersPort for MockHolders {
        async fn screen_at(&self, _socket: &Path) -> Result<String, Status> {
            self.screen_fetches.fetch_add(1, Ordering::SeqCst);
            Ok(self.state.lock().unwrap().1.clone())
        }

        async fn screen_for_tab(
            &self,
            _tab: &str,
            _socket: &Path,
            last_end: Option<u64>,
        ) -> Result<Option<(u64, String)>, Status> {
            self.offset_checks.fetch_add(1, Ordering::SeqCst);
            let (end, screen) = self.state.lock().unwrap().clone();
            if last_end == Some(end) {
                return Ok(None);
            }
            self.screen_fetches.fetch_add(1, Ordering::SeqCst);
            Ok(Some((end, screen)))
        }
    }

    struct MockHookAsk {
        pending: AtomicBool,
        resolved_count: AtomicUsize,
    }

    impl MockHookAsk {
        fn new(pending: bool) -> Self {
            Self {
                pending: AtomicBool::new(pending),
                resolved_count: AtomicUsize::new(0),
            }
        }

        fn is_pending(&self) -> bool {
            self.pending.load(Ordering::SeqCst)
        }

        fn resolved(&self) -> usize {
            self.resolved_count.load(Ordering::SeqCst)
        }
    }

    #[tonic::async_trait]
    impl HookAskPort for MockHookAsk {
        async fn has_pending_hook_ask(&self, _session: &str) -> bool {
            self.pending.load(Ordering::SeqCst)
        }

        async fn resolve_pending_hook_asks(&self, _session: &str) {
            if self.pending.swap(false, Ordering::SeqCst) {
                self.resolved_count.fetch_add(1, Ordering::SeqCst);
            }
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
                title: "test".to_owned(),
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
        store.put_agent_and_tab(&agent, &tab).expect("put_agent_and_tab");
        (dir, store)
    }

    /// Drives spawned tasks (including `spawn_blocking`) to completion without
    /// advancing Tokio's paused virtual clock.
    async fn settle_tasks() {
        for _ in 0..24 {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn genuine_prompts_stay_open_indefinitely_across_multiple_polls() {
        for (agent_kind, prompt_screen) in [
            ("claude", PERMISSION_PROMPT_SCREEN),
            ("codex", QUESTION_PROMPT_SCREEN),
            ("copilot", FOLDER_TRUST_SCREEN),
            ("agy", INPUT_PROMPT_SCREEN),
        ] {
            let session = format!("se_{agent_kind}");
            let (_dir, store) = setup_store(&session, agent_kind);
            let holders = Arc::new(MockHolders::new(100, prompt_screen));
            let hook_asks = Arc::new(MockHookAsk::new(true));
            let watcher = ScreenAsk::new_with_hook_asks(
                holders.clone(),
                Arc::new(NoopAsking),
                store,
                Arc::new(|_| true),
                Arc::new(|_| {}),
                hook_asks.clone(),
            );

            // Repeated polls with advancing output (e.g. animated spinner) while
            // the genuine prompt stays on screen must NEVER clear the ask.
            for step in 0..5 {
                holders.set(100 + step * 25, prompt_screen);
                watcher.look_now(&session, HOLDER_PID).await;
                tokio::time::advance(Duration::from_secs(5)).await;
                settle_tasks().await;
                assert!(
                    hook_asks.is_pending(),
                    "{agent_kind}: genuine prompt should remain open at step {step}"
                );
                assert!(!watcher.has_pending_clear(&session));
            }
            assert_eq!(hook_asks.resolved(), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn transient_redraw_flicker_cancels_debounce_and_keeps_ask_open() {
        // Test both a hook-raised ask (claude) and a screen-raised ask (copilot).
        let (_dir1, store1) = setup_store("se_claude", "claude");
        let holders1 = Arc::new(MockHolders::new(100, PERMISSION_PROMPT_SCREEN));
        let hook_asks1 = Arc::new(MockHookAsk::new(true));
        let watcher1 = ScreenAsk::new_with_hook_asks(
            holders1.clone(),
            Arc::new(NoopAsking),
            store1,
            Arc::new(|_| true),
            Arc::new(|_| {}),
            hook_asks1.clone(),
        );

        // Baseline: prompt visible.
        watcher1.look_now("se_claude", HOLDER_PID).await;
        assert!(hook_asks1.is_pending());

        // Brief redraw flicker: screen blanks and end advances -> arms debounce.
        holders1.set(120, "");
        watcher1.look_now("se_claude", HOLDER_PID).await;
        assert!(watcher1.has_pending_clear("se_claude"));
        assert!(hook_asks1.is_pending());

        // Before 2.5s debounce expires, prompt redraws back on screen -> cancels debounce!
        tokio::time::advance(Duration::from_millis(1000)).await;
        holders1.set(140, PERMISSION_PROMPT_SCREEN);
        watcher1.look_now("se_claude", HOLDER_PID).await;
        assert!(!watcher1.has_pending_clear("se_claude"));

        // Well past the original 2.5s deadline, the ask is still open.
        tokio::time::advance(Duration::from_secs(5)).await;
        settle_tasks().await;
        assert!(hook_asks1.is_pending());
        assert_eq!(hook_asks1.resolved(), 0);

        // Same flicker test for a screen-raised FolderTrust dialog on copilot.
        let (_dir2, store2) = setup_store("se_copilot", "copilot");
        let holders2 = Arc::new(MockHolders::new(100, FOLDER_TRUST_SCREEN));
        let watcher2 = ScreenAsk::new_with_hook_asks(
            holders2.clone(),
            Arc::new(NoopAsking),
            store2,
            Arc::new(|_| false),
            Arc::new(|_| {}),
            Arc::new(NoopHookAsk),
        );

        watcher2.look_now("se_copilot", HOLDER_PID).await;
        assert!(watcher2.has_open_screen_ask("se_copilot").await);

        // Brief flicker -> debounce armed, hold stays open.
        holders2.set(110, "");
        watcher2.look_now("se_copilot", HOLDER_PID).await;
        assert!(watcher2.has_pending_clear("se_copilot"));
        assert!(watcher2.has_open_screen_ask("se_copilot").await);

        // Prompt returns before 2.5s -> debounce cancelled, hold stays open.
        tokio::time::advance(Duration::from_millis(1200)).await;
        holders2.set(130, FOLDER_TRUST_SCREEN);
        watcher2.look_now("se_copilot", HOLDER_PID).await;
        assert!(!watcher2.has_pending_clear("se_copilot"));

        tokio::time::advance(Duration::from_secs(5)).await;
        settle_tasks().await;
        assert!(watcher2.has_open_screen_ask("se_copilot").await);
    }

    #[tokio::test(start_paused = true)]
    async fn stale_hook_ask_auto_clears_after_debounce_when_prompt_gone_and_output_advances() {
        let (_dir, store) = setup_store("se_stale", "claude");
        let holders = Arc::new(MockHolders::new(100, STREAMING_OUTPUT_SCREEN));
        let hook_asks = Arc::new(MockHookAsk::new(true));
        let watcher = ScreenAsk::new_with_hook_asks(
            holders.clone(),
            Arc::new(NoopAsking),
            store,
            Arc::new(|_| true),
            Arc::new(|_| {}),
            hook_asks.clone(),
        );

        // Poll 1: baseline observation (output hasn't advanced yet) -> does NOT clear.
        watcher.look_now("se_stale", HOLDER_PID).await;
        assert!(!watcher.has_pending_clear("se_stale"));
        assert!(hook_asks.is_pending());

        // Poll 2: output advances and no prompt is present -> arms 2.5s debounce.
        holders.set(160, STREAMING_OUTPUT_SCREEN);
        watcher.look_now("se_stale", HOLDER_PID).await;
        assert!(watcher.has_pending_clear("se_stale"));

        // Still open before 2.5s elapses (even if more streaming arrives).
        tokio::time::advance(Duration::from_millis(2000)).await;
        holders.set(220, STREAMING_OUTPUT_SCREEN);
        watcher.look_now("se_stale", HOLDER_PID).await;
        settle_tasks().await;
        assert!(hook_asks.is_pending());

        // After full 2.5s debounce (500ms more, because streaming did not starve the timer),
        // the stale hook ask auto-clears cleanly.
        tokio::time::advance(Duration::from_millis(600)).await;
        settle_tasks().await;
        assert!(!hook_asks.is_pending());
        assert_eq!(hook_asks.resolved(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn lifecycle_hook_signal_immediately_auto_clears_stale_hook_ask() {
        for agent_kind in SUPPORTED_AGENTS {
            let session = format!("se_lc_{agent_kind}");
            let (_dir, store) = setup_store(&session, agent_kind);
            let holders = Arc::new(MockHolders::new(100, STREAMING_OUTPUT_SCREEN));
            let hook_asks = Arc::new(MockHookAsk::new(true));
            let watcher = ScreenAsk::new_with_hook_asks(
                holders,
                Arc::new(NoopAsking),
                store,
                Arc::new(|_| true),
                Arc::new(|_| {}),
                hook_asks.clone(),
            );

            assert!(hook_asks.is_pending());
            watcher.on_lifecycle(&session, agent_kind).await;
            assert!(!hook_asks.is_pending(), "{agent_kind}: should clear immediately");
            assert_eq!(hook_asks.resolved(), 1);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unsupported_and_unknown_agents_are_left_untouched() {
        for agent_kind in ["opencode", "zsh", "unknown_agent", ""] {
            let session = format!("se_unsup_{agent_kind}");
            let (_dir, store) = setup_store(&session, agent_kind);
            let holders = Arc::new(MockHolders::new(100, STREAMING_OUTPUT_SCREEN));
            let hook_asks = Arc::new(MockHookAsk::new(true));
            let watcher = ScreenAsk::new_with_hook_asks(
                holders.clone(),
                Arc::new(NoopAsking),
                store,
                Arc::new(|_| true),
                Arc::new(|_| {}),
                hook_asks.clone(),
            );

            // Signal A leaves unsupported agents untouched.
            watcher.on_lifecycle(&session, agent_kind).await;
            assert!(hook_asks.is_pending());

            // Signal B leaves unsupported agents untouched even as output advances.
            watcher.look_now(&session, HOLDER_PID).await;
            holders.set(200, STREAMING_OUTPUT_SCREEN);
            watcher.look_now(&session, HOLDER_PID).await;
            tokio::time::advance(Duration::from_secs(10)).await;
            settle_tasks().await;
            assert!(
                hook_asks.is_pending(),
                "unsupported agent '{agent_kind}' must remain untouched"
            );
            assert_eq!(hook_asks.resolved(), 0);
            assert_eq!(holders.fetches(), 0);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn zero_work_fast_path_skips_screen_fetches_when_end_unchanged() {
        let (_dir, store) = setup_store("se_fast", "claude");
        let holders = Arc::new(MockHolders::new(500, PERMISSION_PROMPT_SCREEN));
        let hook_asks = Arc::new(MockHookAsk::new(true));
        let watcher = ScreenAsk::new_with_hook_asks(
            holders.clone(),
            Arc::new(NoopAsking),
            store,
            Arc::new(|_| true),
            Arc::new(|_| {}),
            hook_asks.clone(),
        );

        // First poll populates `last_seen = (HOLDER_PID, 500)` with 1 screen fetch.
        watcher.look_now("se_fast", HOLDER_PID).await;
        assert_eq!(holders.fetches(), 1);
        assert!(!watcher.has_pending_clear("se_fast"));

        // 50 subsequent polls with unchanged `(holder_pid, end)` do ZERO extra screen fetches.
        for _ in 0..50 {
            watcher.look_now("se_fast", HOLDER_PID).await;
        }
        assert_eq!(
            holders.fetches(),
            1,
            "unchanged (holder_pid, end) must not perform any extra screen fetches"
        );
        assert!(hook_asks.is_pending());
    }
}
