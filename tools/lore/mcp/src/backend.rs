//! Narrow [`LoreBackend`] trait (`BR-1`) and [`LocalBackend`] wrapping
//! [`lore_engine::LoreEngine`].

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lore_engine::{
    EngineStatus, FeedbackRequest, FeedbackResponse, GcReport, InspectRequest, InspectResponse,
    LoreConfig, LoreEngine, PulseRequest, PulseResponse, RecallRequest, RecallResponse,
    RememberRequest, RememberResponse, VerifyReport, ensure_enabled, export_store_events,
    load_config, read_only_status, save_config, verify_store,
};
use lore_proto::{Clock, LogEvent, Principal, Result, SystemClock};

/// Transport-agnostic backend interface shared by `lore_mcp` and `lore` CLI (`BR-1`).
pub trait LoreBackend {
    fn status(&self) -> Result<EngineStatus>;
    fn recall(&mut self, req: RecallRequest) -> Result<RecallResponse>;
    fn inspect(&self, req: InspectRequest) -> Result<InspectResponse>;
    fn remember(&mut self, req: RememberRequest) -> Result<RememberResponse>;
    fn feedback(&mut self, req: FeedbackRequest) -> Result<FeedbackResponse>;
    fn forget(&mut self, claim_id: &str, reason: &str, actor: Principal) -> Result<()>;
    fn pulse(&mut self, req: PulseRequest) -> Result<PulseResponse>;
    fn gc(&mut self, dry_run: bool) -> Result<GcReport>;
    fn reindex(&mut self) -> Result<u64>;
    fn verify(&self) -> Result<VerifyReport>;
    fn export_events(&self, since_seq: u64) -> Result<Vec<LogEvent>>;
    fn config(&self) -> LoreConfig;
    fn set_enabled(&mut self, enabled: bool, reason: Option<String>) -> Result<LoreConfig>;
}

/// Internal `Clone`-able wrapper around any [`Clock`] so [`LocalBackend`] can
/// lazily open [`LoreEngine`] without requiring `C: Clone` (e.g. `ManualClock`).
struct SharedClock<C: Clock>(Arc<C>);

impl<C: Clock> Clone for SharedClock<C> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<C: Clock> Clock for SharedClock<C> {
    fn now_ms(&self) -> i64 {
        self.0.now_ms()
    }
}

/// Direct local-disk backend backed by [`LoreEngine`].
///
/// Supports eager, lazy (zero storage touch when `memory.enabled == false`),
/// and non-blocking read-only (for fail-open hooks) open modes.
pub struct LocalBackend<C: Clock = SystemClock> {
    root: PathBuf,
    clock: SharedClock<C>,
    engine: RefCell<Option<LoreEngine<SharedClock<C>>>>,
}

impl LocalBackend<SystemClock> {
    /// Opens a local `lore` store at `root` eagerly using [`SystemClock`].
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Self::with_clock(root, SystemClock)
    }

    /// Creates a lazy local backend that does NOT open `index.db`, acquire
    /// `write.lock`, or touch `.pblog` until an enabled operation runs (`SW-4`).
    pub fn open_lazy(root: impl AsRef<Path>) -> Self {
        Self::with_clock_lazy(root, SystemClock)
    }

    /// Opens a read-only local backend with a short SQLite `busy_timeout_ms`
    /// and zero `write.lock` acquisition — safe for fail-open hooks (`HOOK-3`).
    pub fn open_read_only(root: impl AsRef<Path>, busy_timeout_ms: u64) -> Result<Self> {
        Self::with_clock_read_only(root, SystemClock, busy_timeout_ms)
    }
}

impl<C: Clock> LocalBackend<C> {
    /// Opens a local `lore` store at `root` with an injected [`Clock`] (for deterministic tests).
    pub fn with_clock(root: impl AsRef<Path>, clock: C) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let shared = SharedClock(Arc::new(clock));
        let engine = LoreEngine::open(&root, shared.clone())?;
        Ok(Self {
            root,
            clock: shared,
            engine: RefCell::new(Some(engine)),
        })
    }

    /// Creates a lazy local backend with an injected [`Clock`] (zero storage touch on creation).
    pub fn with_clock_lazy(root: impl AsRef<Path>, clock: C) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
            clock: SharedClock(Arc::new(clock)),
            engine: RefCell::new(None),
        }
    }

    /// Opens a read-only local backend with an injected [`Clock`] and short `busy_timeout_ms`.
    pub fn with_clock_read_only(
        root: impl AsRef<Path>,
        clock: C,
        busy_timeout_ms: u64,
    ) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let shared = SharedClock(Arc::new(clock));
        let engine = LoreEngine::open_read_only(&root, shared.clone(), busy_timeout_ms)?;
        Ok(Self {
            root,
            clock: shared,
            engine: RefCell::new(Some(engine)),
        })
    }

    /// Reads the latest `config.toml` from disk without touching storage (`SW-4`).
    fn live_config(&self) -> LoreConfig {
        load_config(&self.root).unwrap_or_default()
    }

    /// Lazily opens [`LoreEngine`] if not yet opened.
    fn ensure_engine(&self) -> Result<()> {
        let mut slot = self.engine.borrow_mut();
        if slot.is_none() {
            *slot = Some(LoreEngine::open(&self.root, self.clock.clone())?);
        }
        Ok(())
    }
}

impl<C: Clock> LoreBackend for LocalBackend<C> {
    fn status(&self) -> Result<EngineStatus> {
        read_only_status(&self.root, &self.clock)
    }

    fn recall(&mut self, req: RecallRequest) -> Result<RecallResponse> {
        ensure_enabled(&self.live_config())?;
        self.ensure_engine()?;
        self.engine
            .borrow_mut()
            .as_mut()
            .expect("engine initialized")
            .recall(req)
    }

    fn inspect(&self, req: InspectRequest) -> Result<InspectResponse> {
        ensure_enabled(&self.live_config())?;
        self.ensure_engine()?;
        self.engine
            .borrow()
            .as_ref()
            .expect("engine initialized")
            .inspect(req)
    }

    fn remember(&mut self, req: RememberRequest) -> Result<RememberResponse> {
        ensure_enabled(&self.live_config())?;
        self.ensure_engine()?;
        self.engine
            .borrow_mut()
            .as_mut()
            .expect("engine initialized")
            .remember(req)
    }

    fn feedback(&mut self, req: FeedbackRequest) -> Result<FeedbackResponse> {
        ensure_enabled(&self.live_config())?;
        self.ensure_engine()?;
        self.engine
            .borrow_mut()
            .as_mut()
            .expect("engine initialized")
            .feedback(req)
    }

    fn forget(&mut self, claim_id: &str, reason: &str, actor: Principal) -> Result<()> {
        self.ensure_engine()?;
        self.engine
            .borrow_mut()
            .as_mut()
            .expect("engine initialized")
            .forget(claim_id, reason, actor)
    }

    fn pulse(&mut self, req: PulseRequest) -> Result<PulseResponse> {
        ensure_enabled(&self.live_config())?;
        self.ensure_engine()?;
        self.engine
            .borrow_mut()
            .as_mut()
            .expect("engine initialized")
            .pulse(req)
    }

    fn gc(&mut self, dry_run: bool) -> Result<GcReport> {
        self.ensure_engine()?;
        self.engine
            .borrow_mut()
            .as_mut()
            .expect("engine initialized")
            .gc(dry_run)
    }

    fn reindex(&mut self) -> Result<u64> {
        self.ensure_engine()?;
        self.engine
            .borrow_mut()
            .as_mut()
            .expect("engine initialized")
            .reindex()
    }

    fn verify(&self) -> Result<VerifyReport> {
        verify_store(&self.root)
    }

    fn export_events(&self, since_seq: u64) -> Result<Vec<LogEvent>> {
        export_store_events(&self.root, since_seq)
    }

    fn config(&self) -> LoreConfig {
        self.live_config()
    }

    fn set_enabled(&mut self, enabled: bool, reason: Option<String>) -> Result<LoreConfig> {
        let mut slot = self.engine.borrow_mut();
        if let Some(engine) = slot.as_mut() {
            engine.set_enabled(enabled, reason)
        } else {
            let mut cfg = load_config(&self.root)?;
            cfg.memory.enabled = enabled;
            cfg.memory.disabled_reason = if enabled { None } else { reason };
            save_config(&self.root, &cfg)?;
            Ok(cfg)
        }
    }
}
