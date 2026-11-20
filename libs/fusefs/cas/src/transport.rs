//! Generic content-addressed chunk transport trait and in-process
//! [`LoopbackTransport`] for testing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cdc::chunk_default;
use crate::codec::CodecError;
use crate::hash::{ChunkHash, hash_bytes};
use crate::manifest::FileManifest;

/// Quality-of-service class for chunk transport operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportQoS {
    /// Latency-sensitive foreground reads/writes (`pread`, `fsync`).
    Interactive,
    /// Background prefetch, rehydration, or bulk sync.
    Bulk,
}

/// Result returned when a file manifest commit succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitResult {
    pub generation: u64,
    pub manifest_hash: ChunkHash,
}

/// Errors returned by a [`ChunkTransport`].
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("object not found: {0}")]
    NotFound(String),
    #[error("manifest generation conflict: expected {expected}, actual {actual}")]
    GenerationConflict { expected: u64, actual: u64 },
    #[error("network unreachable: {0}")]
    NetworkUnreachable(String),
    #[error("transport i/o error: {0}")]
    Io(String),
    #[error("chunk hash mismatch: expected {expected}, actual {actual}")]
    HashMismatch {
        expected: ChunkHash,
        actual: ChunkHash,
    },
    #[error("codec error: {0}")]
    Codec(#[from] CodecError),
}

/// Generic remote CAS chunk & manifest transport.
pub trait ChunkTransport: Send + Sync {
    /// Fetches the requested chunk payloads by hash.
    fn fetch_chunks(
        &self,
        hashes: &[ChunkHash],
        qos: TransportQoS,
    ) -> Result<Vec<(ChunkHash, Vec<u8>)>, TransportError>;

    /// Pushes a batch of `(hash, raw_bytes)` chunks and returns how many chunks
    /// from the prefix of `chunks` were durably accepted.
    fn push_chunks(
        &self,
        chunks: &[(ChunkHash, Vec<u8>)],
        qos: TransportQoS,
    ) -> Result<usize, TransportError>;

    /// Returns the current [`FileManifest`] for `path`, or `None` if absent.
    fn stat_manifest(&self, path: &str) -> Result<Option<FileManifest>, TransportError>;

    /// Atomically commits `manifest` for `path` if the current generation
    /// matches `expected_generation`.
    fn commit_file(
        &self,
        path: &str,
        expected_generation: u64,
        manifest: FileManifest,
    ) -> Result<CommitResult, TransportError>;
}

/// Telemetry counters exposed by [`LoopbackTransport`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LoopbackStats {
    pub interactive_calls: u64,
    pub bulk_calls: u64,
    pub fetched_chunks: u64,
    pub pushed_chunks: u64,
    pub commits: u64,
}

#[derive(Default)]
struct LoopbackState {
    chunks: HashMap<ChunkHash, Vec<u8>>,
    manifests: HashMap<String, FileManifest>,
    stalled: bool,
    io_error: bool,
    max_chunks_per_push: Option<usize>,
    stall_after_chunks: Option<usize>,
    push_delay: Duration,
    stats: LoopbackStats,
}

/// In-process thread-safe [`ChunkTransport`] with fault injection for tests.
#[derive(Clone, Default)]
pub struct LoopbackTransport {
    inner: Arc<Mutex<LoopbackState>>,
}

impl LoopbackTransport {
    /// Creates an empty in-process loopback transport.
    pub fn new() -> Self {
        Self::default()
    }

    /// Chunks `data` with FastCDC, stores all chunks, and seeds `path` at
    /// generation 1.
    pub fn seed_file(&self, path: impl Into<String>, data: &[u8]) -> FileManifest {
        let slices = chunk_default(data);
        let mut guard = self.inner.lock().unwrap();
        for slice in &slices {
            let start = slice.offset as usize;
            let end = slice.end() as usize;
            guard.chunks.insert(slice.hash, data[start..end].to_vec());
        }
        let manifest = FileManifest::from_slices(data.len() as u64, 1, slices);
        guard.manifests.insert(path.into(), manifest.clone());
        manifest
    }

    /// Reads and reassembles all bytes of `path` directly from the loopback store.
    pub fn read_file_bytes(&self, path: &str) -> Result<Vec<u8>, TransportError> {
        let guard = self.inner.lock().unwrap();
        let manifest = guard
            .manifests
            .get(path)
            .ok_or_else(|| TransportError::NotFound(path.to_string()))?;
        let mut out = vec![0u8; manifest.size as usize];
        for c in &manifest.chunks {
            let data = guard
                .chunks
                .get(&c.hash)
                .ok_or_else(|| TransportError::NotFound(c.hash.to_hex()))?;
            let start = c.offset as usize;
            let end = (c.offset + c.len as u64) as usize;
            out[start..end].copy_from_slice(&data[..c.len as usize]);
        }
        Ok(out)
    }

    /// Toggles simulated network stall (`TransportError::NetworkUnreachable`).
    pub fn set_stalled(&self, stalled: bool) {
        self.inner.lock().unwrap().stalled = stalled;
    }

    /// Toggles simulated hard transport I/O errors (`TransportError::Io`).
    pub fn set_io_error(&self, io_error: bool) {
        self.inner.lock().unwrap().io_error = io_error;
    }

    /// Limits how many chunks each `push_chunks` call accepts before returning.
    pub fn set_max_chunks_per_push(&self, limit: Option<usize>) {
        self.inner.lock().unwrap().max_chunks_per_push = limit;
    }

    /// Automatically flips `stalled = true` after `n` more chunks are pushed.
    pub fn set_stall_after_chunks(&self, remaining: Option<usize>) {
        self.inner.lock().unwrap().stall_after_chunks = remaining;
    }

    /// Adds an artificial delay to each `push_chunks` call.
    pub fn set_push_delay(&self, delay: Duration) {
        self.inner.lock().unwrap().push_delay = delay;
    }

    /// Returns a snapshot of transport counters.
    pub fn stats(&self) -> LoopbackStats {
        self.inner.lock().unwrap().stats
    }

    fn record_qos(state: &mut LoopbackState, qos: TransportQoS) {
        match qos {
            TransportQoS::Interactive => state.stats.interactive_calls += 1,
            TransportQoS::Bulk => state.stats.bulk_calls += 1,
        }
    }
}

impl ChunkTransport for LoopbackTransport {
    fn fetch_chunks(
        &self,
        hashes: &[ChunkHash],
        qos: TransportQoS,
    ) -> Result<Vec<(ChunkHash, Vec<u8>)>, TransportError> {
        let mut guard = self.inner.lock().unwrap();
        Self::record_qos(&mut guard, qos);
        if guard.io_error {
            return Err(TransportError::Io("simulated loopback i/o error".into()));
        }
        if guard.stalled {
            return Err(TransportError::NetworkUnreachable(
                "simulated loopback stall".into(),
            ));
        }

        let mut out = Vec::with_capacity(hashes.len());
        for &h in hashes {
            let bytes = guard
                .chunks
                .get(&h)
                .ok_or_else(|| TransportError::NotFound(h.to_hex()))?
                .clone();
            out.push((h, bytes));
        }
        guard.stats.fetched_chunks += out.len() as u64;
        Ok(out)
    }

    fn push_chunks(
        &self,
        chunks: &[(ChunkHash, Vec<u8>)],
        qos: TransportQoS,
    ) -> Result<usize, TransportError> {
        let delay = self.inner.lock().unwrap().push_delay;
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }

        let mut guard = self.inner.lock().unwrap();
        Self::record_qos(&mut guard, qos);
        if guard.io_error {
            return Err(TransportError::Io("simulated loopback i/o error".into()));
        }
        if guard.stalled {
            return Err(TransportError::NetworkUnreachable(
                "simulated loopback stall".into(),
            ));
        }

        let mut limit = chunks.len();
        if let Some(max_per_push) = guard.max_chunks_per_push {
            limit = limit.min(max_per_push);
        }
        if let Some(rem) = guard.stall_after_chunks {
            if rem == 0 {
                guard.stalled = true;
                return Err(TransportError::NetworkUnreachable(
                    "simulated loopback stall after chunk budget".into(),
                ));
            }
            limit = limit.min(rem);
        }

        let mut accepted = 0usize;
        for (hash, bytes) in &chunks[..limit] {
            let actual = hash_bytes(bytes);
            if actual != *hash {
                return Err(TransportError::HashMismatch {
                    expected: *hash,
                    actual,
                });
            }
            guard.chunks.insert(*hash, bytes.clone());
            accepted += 1;
        }
        guard.stats.pushed_chunks += accepted as u64;

        if let Some(rem) = guard.stall_after_chunks.as_mut() {
            *rem = rem.saturating_sub(accepted);
            if *rem == 0 {
                guard.stalled = true;
            }
        }

        Ok(accepted)
    }

    fn stat_manifest(&self, path: &str) -> Result<Option<FileManifest>, TransportError> {
        let guard = self.inner.lock().unwrap();
        if guard.io_error {
            return Err(TransportError::Io("simulated loopback i/o error".into()));
        }
        if guard.stalled {
            return Err(TransportError::NetworkUnreachable(
                "simulated loopback stall".into(),
            ));
        }
        Ok(guard.manifests.get(path).cloned())
    }

    fn commit_file(
        &self,
        path: &str,
        expected_generation: u64,
        manifest: FileManifest,
    ) -> Result<CommitResult, TransportError> {
        let mut guard = self.inner.lock().unwrap();
        if guard.io_error {
            return Err(TransportError::Io("simulated loopback i/o error".into()));
        }
        if guard.stalled {
            return Err(TransportError::NetworkUnreachable(
                "simulated loopback stall".into(),
            ));
        }

        let current_gen = guard
            .manifests
            .get(path)
            .map(|m| m.generation)
            .unwrap_or(0);
        if current_gen != expected_generation {
            return Err(TransportError::GenerationConflict {
                expected: expected_generation,
                actual: current_gen,
            });
        }

        // Verify all referenced chunks exist before committing.
        for c in &manifest.chunks {
            if !guard.chunks.contains_key(&c.hash) {
                return Err(TransportError::NotFound(c.hash.to_hex()));
            }
        }

        let new_gen = expected_generation + 1;
        let mut committed = manifest;
        committed.generation = new_gen;
        let manifest_hash = hash_bytes(&committed.to_binary_bytes());
        guard.manifests.insert(path.to_string(), committed);
        guard.stats.commits += 1;

        Ok(CommitResult {
            generation: new_gen,
            manifest_hash,
        })
    }
}
