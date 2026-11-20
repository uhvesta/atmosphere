//! T7: Peer chunk transport with `AWAY` autonomy, coalescing fire-and-forget
//! invalidation ring, host-relayed remote-to-remote routing, and Interactive vs.
//! Bulk QoS prioritization.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

use fusefs_cas::{
    ChunkHash, ChunkTransport, CommitResult, FileManifest, TransportError, TransportQoS,
};
use serde::{Deserialize, Serialize};

/// Routing target for reaching a remote machine's CAS chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RouteTarget {
    /// Direct authenticated peer link to `machine`.
    Direct(String),
    /// Remote-to-remote traffic relayed through `host` to `target`.
    ViaHost { host: String, target: String },
}

impl RouteTarget {
    /// Returns `true` if this route depends on host reachability.
    pub fn requires_host(&self) -> bool {
        matches!(self, Self::ViaHost { .. })
    }

    pub fn destination_machine(&self) -> &str {
        match self {
            Self::Direct(m) => m,
            Self::ViaHost { target, .. } => target,
        }
    }
}

/// Thread-safe machine routing table supporting direct and host-relayed hops.
#[derive(Debug, Default)]
pub struct RoutingTable {
    routes: RwLock<BTreeMap<String, RouteTarget>>,
}

impl RoutingTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_route(&self, machine_id: impl Into<String>, route: RouteTarget) {
        self.routes
            .write()
            .expect("routing table lock poisoned")
            .insert(machine_id.into(), route);
    }

    pub fn resolve(&self, machine_id: &str) -> Option<RouteTarget> {
        self.routes
            .read()
            .expect("routing table lock poisoned")
            .get(machine_id)
            .cloned()
    }

    pub fn remove_route(&self, machine_id: &str) -> Option<RouteTarget> {
        self.routes
            .write()
            .expect("routing table lock poisoned")
            .remove(machine_id)
    }

    pub fn list_routes(&self) -> Vec<(String, RouteTarget)> {
        self.routes
            .read()
            .expect("routing table lock poisoned")
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// Outbound cache invalidation event queued after local commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationEvent {
    pub path: String,
    pub generation: u64,
    pub machine_id: String,
}

/// Telemetry counters for [`InvalidationRing`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidationRingStats {
    pub capacity: usize,
    pub queued: usize,
    pub enqueued_total: u64,
    pub coalesced_total: u64,
    pub dropped_total: u64,
}

struct InvalidationRingInner {
    capacity: usize,
    order: VecDeque<String>,
    pending: BTreeMap<String, InvalidationEvent>,
    enqueued_total: u64,
    coalesced_total: u64,
    dropped_total: u64,
}

/// Bounded coalescing fire-and-forget invalidation ring buffer.
///
/// Local writes enqueue invalidation notices here and return immediately without
/// waiting on peer or host delivery:
/// - Duplicate paths are coalesced in-place to the highest generation.
/// - When full, the oldest entry is evicted so local writes never block.
pub struct InvalidationRing {
    inner: Mutex<InvalidationRingInner>,
}

impl InvalidationRing {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            inner: Mutex::new(InvalidationRingInner {
                capacity: cap,
                order: VecDeque::with_capacity(cap),
                pending: BTreeMap::new(),
                enqueued_total: 0,
                coalesced_total: 0,
                dropped_total: 0,
            }),
        }
    }

    /// Enqueues an invalidation event without ever blocking on I/O.
    pub fn push(&self, event: InvalidationEvent) {
        let mut guard = self.inner.lock().expect("invalidation ring lock poisoned");
        guard.enqueued_total = guard.enqueued_total.saturating_add(1);

        if let Some(existing) = guard.pending.get_mut(&event.path) {
            existing.generation = existing.generation.max(event.generation);
            existing.machine_id = event.machine_id;
            guard.coalesced_total = guard.coalesced_total.saturating_add(1);
            return;
        }

        if guard.order.len() >= guard.capacity
            && let Some(oldest_path) = guard.order.pop_front()
        {
            guard.pending.remove(&oldest_path);
            guard.dropped_total = guard.dropped_total.saturating_add(1);
        }

        guard.order.push_back(event.path.clone());
        guard.pending.insert(event.path.clone(), event);
    }

    /// Drains up to `max` coalesced invalidation events in FIFO order.
    pub fn drain(&self, max: usize) -> Vec<InvalidationEvent> {
        let mut guard = self.inner.lock().expect("invalidation ring lock poisoned");
        let count = max.min(guard.order.len());
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(path) = guard.order.pop_front()
                && let Some(ev) = guard.pending.remove(&path)
            {
                out.push(ev);
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .expect("invalidation ring lock poisoned")
            .order
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> InvalidationRingStats {
        let guard = self.inner.lock().expect("invalidation ring lock poisoned");
        InvalidationRingStats {
            capacity: guard.capacity,
            queued: guard.order.len(),
            enqueued_total: guard.enqueued_total,
            coalesced_total: guard.coalesced_total,
            dropped_total: guard.dropped_total,
        }
    }
}

/// Telemetry counters for [`QosGate`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QosStats {
    pub interactive_dispatched: u64,
    pub bulk_dispatched: u64,
    pub bulk_yields: u64,
    pub active_interactive: usize,
    pub active_bulk: usize,
}

#[derive(Default)]
struct QosState {
    waiting_interactive: usize,
    active_interactive: usize,
    active_bulk: usize,
}

/// Priority gate ensuring [`TransportQoS::Interactive`] operations are never
/// starved by background [`TransportQoS::Bulk`] transfers (e.g. 1 GiB streams).
pub struct QosGate {
    state: Mutex<QosState>,
    cvar: Condvar,
    bulk_concurrency_limit: usize,
    interactive_dispatched: AtomicU64,
    bulk_dispatched: AtomicU64,
    bulk_yields: AtomicU64,
    active_interactive_gauge: AtomicUsize,
    active_bulk_gauge: AtomicUsize,
}

/// RAII permit acquired from [`QosGate`].
pub struct QosPermit<'a> {
    gate: &'a QosGate,
    qos: TransportQoS,
}

impl Drop for QosPermit<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.state.lock().expect("qos gate lock poisoned");
        match self.qos {
            TransportQoS::Interactive => {
                state.active_interactive = state.active_interactive.saturating_sub(1);
                self.gate
                    .active_interactive_gauge
                    .store(state.active_interactive, Ordering::Relaxed);
            }
            TransportQoS::Bulk => {
                state.active_bulk = state.active_bulk.saturating_sub(1);
                self.gate
                    .active_bulk_gauge
                    .store(state.active_bulk, Ordering::Relaxed);
            }
        }
        self.gate.cvar.notify_all();
    }
}

impl QosGate {
    pub fn new(bulk_concurrency_limit: usize) -> Self {
        Self {
            state: Mutex::new(QosState::default()),
            cvar: Condvar::new(),
            bulk_concurrency_limit: bulk_concurrency_limit.max(1),
            interactive_dispatched: AtomicU64::new(0),
            bulk_dispatched: AtomicU64::new(0),
            bulk_yields: AtomicU64::new(0),
            active_interactive_gauge: AtomicUsize::new(0),
            active_bulk_gauge: AtomicUsize::new(0),
        }
    }

    /// Acquires a QoS permit.
    ///
    /// - `Interactive` never waits on `Bulk` transfers; it preempts queued bulk work.
    /// - `Bulk` yields whenever any `Interactive` work is waiting or active, or
    ///   when `bulk_concurrency_limit` is reached.
    pub fn acquire(&self, qos: TransportQoS) -> QosPermit<'_> {
        let mut state = self.state.lock().expect("qos gate lock poisoned");
        match qos {
            TransportQoS::Interactive => {
                state.waiting_interactive += 1;
                state.waiting_interactive -= 1;
                state.active_interactive += 1;
                self.active_interactive_gauge
                    .store(state.active_interactive, Ordering::Relaxed);
                self.interactive_dispatched.fetch_add(1, Ordering::Relaxed);
            }
            TransportQoS::Bulk => {
                while state.waiting_interactive > 0
                    || state.active_interactive > 0
                    || state.active_bulk >= self.bulk_concurrency_limit
                {
                    self.bulk_yields.fetch_add(1, Ordering::Relaxed);
                    state = self.cvar.wait(state).expect("qos gate lock poisoned");
                }
                state.active_bulk += 1;
                self.active_bulk_gauge
                    .store(state.active_bulk, Ordering::Relaxed);
                self.bulk_dispatched.fetch_add(1, Ordering::Relaxed);
            }
        }
        QosPermit { gate: self, qos }
    }

    pub fn stats(&self) -> QosStats {
        QosStats {
            interactive_dispatched: self.interactive_dispatched.load(Ordering::Relaxed),
            bulk_dispatched: self.bulk_dispatched.load(Ordering::Relaxed),
            bulk_yields: self.bulk_yields.load(Ordering::Relaxed),
            active_interactive: self.active_interactive_gauge.load(Ordering::Relaxed),
            active_bulk: self.active_bulk_gauge.load(Ordering::Relaxed),
        }
    }
}

/// Telemetry counters exposed by [`PeerChunkTransport`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerTransportStats {
    pub host_reachable: bool,
    pub away_revalidation_skips: u64,
    pub offline_fast_fails: u64,
    pub remote_round_trips: u64,
    pub invalidations: InvalidationRingStats,
    pub qos: QosStats,
}

/// Humata peer chunk transport implementing [`fusefs_cas::ChunkTransport`] with
/// `AWAY` autonomy, coalescing invalidation ring, host-relayed routing, and
/// Interactive vs. Bulk QoS scheduling.
pub struct PeerChunkTransport {
    local_machine_id: String,
    target_machine_id: String,
    host_reachable: Arc<AtomicBool>,
    routing: Arc<RoutingTable>,
    invalidations: Arc<InvalidationRing>,
    qos_gate: Arc<QosGate>,
    local_manifests: RwLock<BTreeMap<String, FileManifest>>,
    backend: Arc<dyn ChunkTransport>,
    away_revalidation_skips: AtomicU64,
    offline_fast_fails: AtomicU64,
    remote_round_trips: AtomicU64,
}

impl PeerChunkTransport {
    pub fn new(
        local_machine_id: impl Into<String>,
        target_machine_id: impl Into<String>,
        host_reachable: Arc<AtomicBool>,
        routing: Arc<RoutingTable>,
        invalidations: Arc<InvalidationRing>,
        backend: Arc<dyn ChunkTransport>,
    ) -> Self {
        Self {
            local_machine_id: local_machine_id.into(),
            target_machine_id: target_machine_id.into(),
            host_reachable,
            routing,
            invalidations,
            qos_gate: Arc::new(QosGate::new(2)),
            local_manifests: RwLock::new(BTreeMap::new()),
            backend,
            away_revalidation_skips: AtomicU64::new(0),
            offline_fast_fails: AtomicU64::new(0),
            remote_round_trips: AtomicU64::new(0),
        }
    }

    pub fn with_qos_gate(mut self, qos_gate: Arc<QosGate>) -> Self {
        self.qos_gate = qos_gate;
        self
    }

    pub fn is_host_reachable(&self) -> bool {
        self.host_reachable.load(Ordering::Acquire)
    }

    pub fn set_host_reachable(&self, reachable: bool) {
        self.host_reachable.store(reachable, Ordering::Release);
    }

    pub fn routing(&self) -> &Arc<RoutingTable> {
        &self.routing
    }

    pub fn invalidations(&self) -> &Arc<InvalidationRing> {
        &self.invalidations
    }

    pub fn qos_gate(&self) -> &Arc<QosGate> {
        &self.qos_gate
    }

    /// Seeds a locally known manifest so `AWAY` revalidation can return it in
    /// `0 ms` without contacting the host.
    pub fn seed_local_manifest(&self, path: impl Into<String>, manifest: FileManifest) {
        self.local_manifests
            .write()
            .expect("local manifests lock poisoned")
            .insert(path.into(), manifest);
    }

    /// Resolves the active route for `target_machine_id`.
    pub fn active_route(&self) -> RouteTarget {
        self.routing
            .resolve(&self.target_machine_id)
            .unwrap_or_else(|| RouteTarget::Direct(self.target_machine_id.clone()))
    }

    /// Revalidates a local-authority path.
    ///
    /// When `host_reachable == false`, performs a **`0 ms` AWAY skip** and
    /// immediately returns the cached local manifest with zero host round-trips.
    pub fn revalidate_local_path(&self, path: &str) -> Result<Option<FileManifest>, TransportError> {
        if !self.is_host_reachable() {
            self.away_revalidation_skips.fetch_add(1, Ordering::Relaxed);
            return Ok(self
                .local_manifests
                .read()
                .expect("local manifests lock poisoned")
                .get(path)
                .cloned());
        }

        let _permit = self.qos_gate.acquire(TransportQoS::Interactive);
        self.remote_round_trips.fetch_add(1, Ordering::Relaxed);
        let manifest = self.backend.stat_manifest(path)?;
        if let Some(m) = &manifest {
            self.local_manifests
                .write()
                .expect("local manifests lock poisoned")
                .insert(path.to_string(), m.clone());
        }
        Ok(manifest)
    }

    pub fn stats(&self) -> PeerTransportStats {
        PeerTransportStats {
            host_reachable: self.is_host_reachable(),
            away_revalidation_skips: self.away_revalidation_skips.load(Ordering::Relaxed),
            offline_fast_fails: self.offline_fast_fails.load(Ordering::Relaxed),
            remote_round_trips: self.remote_round_trips.load(Ordering::Relaxed),
            invalidations: self.invalidations.stats(),
            qos: self.qos_gate.stats(),
        }
    }

    fn ensure_remote_reachable(&self, op: &str) -> Result<(), TransportError> {
        if self.target_machine_id == self.local_machine_id {
            return Ok(());
        }
        if !self.is_host_reachable() {
            self.offline_fast_fails.fetch_add(1, Ordering::Relaxed);
            let route = self.active_route();
            return Err(TransportError::NetworkUnreachable(format!(
                "offline peer fast-fail during {op} to '{}' via {route:?} (host_reachable=false)",
                self.target_machine_id
            )));
        }
        Ok(())
    }
}

impl ChunkTransport for PeerChunkTransport {
    fn fetch_chunks(
        &self,
        hashes: &[ChunkHash],
        qos: TransportQoS,
    ) -> Result<Vec<(ChunkHash, Vec<u8>)>, TransportError> {
        self.ensure_remote_reachable("fetch_chunks")?;
        let _permit = self.qos_gate.acquire(qos);
        self.remote_round_trips.fetch_add(1, Ordering::Relaxed);
        self.backend.fetch_chunks(hashes, qos)
    }

    fn push_chunks(
        &self,
        chunks: &[(ChunkHash, Vec<u8>)],
        qos: TransportQoS,
    ) -> Result<usize, TransportError> {
        self.ensure_remote_reachable("push_chunks")?;
        let _permit = self.qos_gate.acquire(qos);
        self.remote_round_trips.fetch_add(1, Ordering::Relaxed);
        self.backend.push_chunks(chunks, qos)
    }

    fn stat_manifest(&self, path: &str) -> Result<Option<FileManifest>, TransportError> {
        if !self.is_host_reachable() {
            if let Some(cached) = self
                .local_manifests
                .read()
                .expect("local manifests lock poisoned")
                .get(path)
                .cloned()
            {
                self.away_revalidation_skips.fetch_add(1, Ordering::Relaxed);
                return Ok(Some(cached));
            }
            self.ensure_remote_reachable("stat_manifest")?;
        }
        let _permit = self.qos_gate.acquire(TransportQoS::Interactive);
        self.remote_round_trips.fetch_add(1, Ordering::Relaxed);
        let manifest = self.backend.stat_manifest(path)?;
        if let Some(m) = &manifest {
            self.local_manifests
                .write()
                .expect("local manifests lock poisoned")
                .insert(path.to_string(), m.clone());
        }
        Ok(manifest)
    }

    fn commit_file(
        &self,
        path: &str,
        expected_generation: u64,
        manifest: FileManifest,
    ) -> Result<CommitResult, TransportError> {
        self.ensure_remote_reachable("commit_file")?;
        let _permit = self.qos_gate.acquire(TransportQoS::Interactive);
        self.remote_round_trips.fetch_add(1, Ordering::Relaxed);
        let result = self
            .backend
            .commit_file(path, expected_generation, manifest.clone())?;

        let mut committed = manifest;
        committed.generation = result.generation;
        self.local_manifests
            .write()
            .expect("local manifests lock poisoned")
            .insert(path.to_string(), committed);

        // Coalescing fire-and-forget invalidation: never blocks local writes.
        self.invalidations.push(InvalidationEvent {
            path: path.to_string(),
            generation: result.generation,
            machine_id: self.local_machine_id.clone(),
        });

        Ok(result)
    }
}
