//! Sparse local Copy-on-Write (CoW) range overlay, resumable commit journal,
//! and progress-bounded `fsync`.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::cdc::chunk_default;
use crate::hash::ChunkHash;
use crate::manifest::{ChunkRef, FileManifest};
use crate::store::{ChunkStore, EvictionPriority, StoreError};
use crate::transport::{ChunkTransport, CommitResult, TransportError, TransportQoS};

/// Errors produced by [`CowOverlay`] operations.
#[derive(Debug, Error)]
pub enum CowError {
    #[error("transport error: {0}")]
    Transport(#[from] TransportError),
    #[error("chunk store error: {0}")]
    Store(#[from] StoreError),
    #[error("missing chunk {0} while reading base file")]
    MissingChunk(ChunkHash),
    #[error("journal i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("journal serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Sparse local CoW overlay on top of an immutable base [`FileManifest`].
///
/// Supports arbitrary partial-chunk `pwrite`, `pread` spanning clean remote
/// chunks and dirty local overlays, `truncate`, and automatic coalescing of
/// adjacent or overlapping dirty byte ranges.
#[derive(Debug, Clone)]
pub struct CowOverlay {
    path: String,
    base_manifest: FileManifest,
    /// Upper bound of bytes from `base_manifest` that are still logically valid
    /// (clamped downward on `truncate` so extending a truncated file yields zeros).
    valid_base_len: u64,
    /// Current logical size of the file including local writes and truncates.
    logical_size: u64,
    /// Non-overlapping, coalesced dirty byte extents keyed by start offset.
    dirty: BTreeMap<u64, Vec<u8>>,
    /// Tracks whether `truncate` modified the logical size even if `dirty` is empty.
    truncated: bool,
}

impl CowOverlay {
    /// Creates a new CoW overlay backed by `base_manifest`.
    pub fn new(path: impl Into<String>, base_manifest: FileManifest) -> Self {
        let size = base_manifest.size;
        Self {
            path: path.into(),
            base_manifest,
            valid_base_len: size,
            logical_size: size,
            dirty: BTreeMap::new(),
            truncated: false,
        }
    }

    /// Returns the file path associated with this overlay.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the base [`FileManifest`].
    pub fn base_manifest(&self) -> &FileManifest {
        &self.base_manifest
    }

    /// Returns the current logical size of the file.
    pub fn size(&self) -> u64 {
        self.logical_size
    }

    /// Returns `true` if there are uncommitted local writes or truncations.
    pub fn is_dirty(&self) -> bool {
        !self.dirty.is_empty() || self.truncated
    }

    /// Returns the sorted, coalesced dirty `(start, end)` byte ranges.
    pub fn dirty_ranges(&self) -> Vec<(u64, u64)> {
        self.dirty
            .iter()
            .map(|(&start, buf)| (start, start + buf.len() as u64))
            .collect()
    }

    /// Writes `data` at arbitrary byte `offset`, coalescing any overlapping or
    /// adjacent dirty extents.
    pub fn pwrite(&mut self, offset: u64, data: &[u8]) -> usize {
        if data.is_empty() {
            return 0;
        }
        let write_end = offset.saturating_add(data.len() as u64);

        // Collect all existing dirty extents that overlap or touch [offset, write_end].
        let keys_to_merge: Vec<u64> = self
            .dirty
            .iter()
            .filter_map(|(&start, buf)| {
                let end = start + buf.len() as u64;
                if end >= offset && start <= write_end {
                    Some(start)
                } else {
                    None
                }
            })
            .collect();

        let mut merged_start = offset;
        let mut merged_end = write_end;
        let mut removed = Vec::with_capacity(keys_to_merge.len());

        for key in keys_to_merge {
            if let Some(buf) = self.dirty.remove(&key) {
                let end = key + buf.len() as u64;
                merged_start = merged_start.min(key);
                merged_end = merged_end.max(end);
                removed.push((key, buf));
            }
        }

        let mut merged = vec![0u8; (merged_end - merged_start) as usize];
        for (start, buf) in removed {
            let rel = (start - merged_start) as usize;
            merged[rel..rel + buf.len()].copy_from_slice(&buf);
        }
        let write_rel = (offset - merged_start) as usize;
        merged[write_rel..write_rel + data.len()].copy_from_slice(data);

        self.dirty.insert(merged_start, merged);
        if write_end > self.logical_size {
            self.logical_size = write_end;
        }
        data.len()
    }

    /// Truncates or extends the file to `new_size`.
    pub fn truncate(&mut self, new_size: u64) {
        if new_size == self.logical_size {
            return;
        }
        self.truncated = true;
        self.logical_size = new_size;
        self.valid_base_len = self.valid_base_len.min(new_size);

        // Drop dirty extents starting at or beyond new_size.
        let to_remove: Vec<u64> = self.dirty.range(new_size..).map(|(&k, _)| k).collect();
        for k in to_remove {
            self.dirty.remove(&k);
        }

        // Trim any single dirty extent that straddles new_size.
        if let Some((&start, buf)) = self.dirty.range_mut(..new_size).next_back() {
            let end = start + buf.len() as u64;
            if end > new_size {
                buf.truncate((new_size - start) as usize);
            }
        }
    }

    /// Reads up to `len` bytes starting at `offset`, merging clean base chunks
    /// (fetched via `read_chunk`), zero-filled holes, and dirty local overlays.
    pub fn pread<F>(&self, offset: u64, len: usize, mut read_chunk: F) -> Result<Vec<u8>, CowError>
    where
        F: FnMut(&ChunkRef) -> Result<Vec<u8>, CowError>,
    {
        if offset >= self.logical_size || len == 0 {
            return Ok(Vec::new());
        }
        let actual_len = (len as u64).min(self.logical_size - offset) as usize;
        let read_end = offset + actual_len as u64;
        let mut out = vec![0u8; actual_len];

        // 1. Fill from valid portion of base manifest chunks.
        let base_end = read_end.min(self.valid_base_len);
        if offset < base_end {
            let base_chunks = self.base_manifest.chunks_in_range(offset, base_end - offset);
            for chunk in base_chunks {
                let c_start = chunk.offset.max(offset);
                let c_end = chunk.end().min(base_end);
                if c_start >= c_end {
                    continue;
                }
                // Skip fetching the base chunk if completely covered by a single dirty extent.
                if self.is_fully_covered_by_dirty(c_start, c_end) {
                    continue;
                }
                let raw = read_chunk(chunk)?;
                let src_start = (c_start - chunk.offset) as usize;
                let copy_len = (c_end - c_start) as usize;
                let dst_start = (c_start - offset) as usize;
                out[dst_start..dst_start + copy_len]
                    .copy_from_slice(&raw[src_start..src_start + copy_len]);
            }
        }

        // 2. Overlay all intersecting dirty extents on top.
        for (&d_start, d_buf) in &self.dirty {
            let d_end = d_start + d_buf.len() as u64;
            if d_end <= offset {
                continue;
            }
            if d_start >= read_end {
                break;
            }
            let ov_start = d_start.max(offset);
            let ov_end = d_end.min(read_end);
            let src_start = (ov_start - d_start) as usize;
            let dst_start = (ov_start - offset) as usize;
            let copy_len = (ov_end - ov_start) as usize;
            out[dst_start..dst_start + copy_len]
                .copy_from_slice(&d_buf[src_start..src_start + copy_len]);
        }

        Ok(out)
    }

    /// Reads up to `len` bytes at `offset` using an optional local [`ChunkStore`]
    /// cache backed by a remote [`ChunkTransport`].
    pub fn pread_with_transport(
        &self,
        offset: u64,
        len: usize,
        transport: &dyn ChunkTransport,
        qos: TransportQoS,
        mut cache: Option<&mut ChunkStore>,
    ) -> Result<Vec<u8>, CowError> {
        self.pread(offset, len, |chunk| {
            if let Some(store) = cache.as_deref_mut() {
                if let Some(hit) = store.get(&chunk.hash)? {
                    return Ok(hit);
                }
            }
            let fetched = transport.fetch_chunks(&[chunk.hash], qos)?;
            let (_, bytes) = fetched
                .into_iter()
                .next()
                .ok_or(CowError::MissingChunk(chunk.hash))?;
            if let Some(store) = cache.as_deref_mut() {
                let prio = match qos {
                    TransportQoS::Interactive => EvictionPriority::Interactive,
                    TransportQoS::Bulk => EvictionPriority::Bulk,
                };
                store.put_verified(chunk.hash, &bytes, prio)?;
            }
            Ok(bytes)
        })
    }

    /// Builds a resumable [`CommitJournal`] representing the current CoW state.
    ///
    /// Chunks the materialized file with FastCDC and schedules only newly
    /// introduced chunks (not already present in `base_manifest`) for upload.
    pub fn prepare_commit(
        &self,
        transport: &dyn ChunkTransport,
        qos: TransportQoS,
    ) -> Result<CommitJournal, CowError> {
        let full_bytes =
            self.pread_with_transport(0, self.logical_size as usize, transport, qos, None)?;
        let slices = chunk_default(&full_bytes);
        let base_hashes: HashSet<ChunkHash> =
            self.base_manifest.chunks.iter().map(|c| c.hash).collect();

        let mut pending_chunks = BTreeMap::new();
        for s in &slices {
            if !base_hashes.contains(&s.hash) {
                let start = s.offset as usize;
                let end = s.end() as usize;
                pending_chunks.insert(s.hash, full_bytes[start..end].to_vec());
            }
        }

        let target_manifest = FileManifest::from_slices(
            self.logical_size,
            self.base_manifest.generation + 1,
            slices,
        );

        Ok(CommitJournal {
            path: self.path.clone(),
            base_generation: self.base_manifest.generation,
            target_manifest,
            pending_chunks,
            uploaded_chunks: Vec::new(),
            committed: false,
        })
    }

    fn is_fully_covered_by_dirty(&self, start: u64, end: u64) -> bool {
        if let Some((&d_start, d_buf)) = self.dirty.range(..=start).next_back() {
            d_start <= start && (d_start + d_buf.len() as u64) >= end
        } else {
            false
        }
    }
}

/// Resumable journal of pending chunk uploads and target file manifest commit.
///
/// Preserved on stall or crash so a subsequent `fsync` resumes from the exact
/// remaining chunks without losing CoW state or re-uploading completed chunks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitJournal {
    pub path: String,
    pub base_generation: u64,
    pub target_manifest: FileManifest,
    /// Chunks still awaiting durable upload to the remote transport.
    pub pending_chunks: BTreeMap<ChunkHash, Vec<u8>>,
    /// Chunks already confirmed uploaded to the remote transport.
    pub uploaded_chunks: Vec<ChunkHash>,
    /// Set to `true` once `commit_file` succeeds on the remote transport.
    pub committed: bool,
}

impl CommitJournal {
    /// Returns the number of chunks still pending upload.
    pub fn pending_count(&self) -> usize {
        self.pending_chunks.len()
    }

    /// Returns the number of chunks already uploaded.
    pub fn uploaded_count(&self) -> usize {
        self.uploaded_chunks.len()
    }

    /// Persists the journal atomically (temp+rename) to `path`.
    pub fn save_to_file(&self, path: &Path) -> Result<(), CowError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("journal.tmp");
        let bytes = serde_json::to_vec(self)?;
        let mut f = File::create(&tmp)?;
        f.write_all(&bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)?;
        Ok(())
    }

    /// Loads a previously persisted [`CommitJournal`] from `path`.
    pub fn load_from_file(path: &Path) -> Result<Self, CowError> {
        let bytes = fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

/// Terminal failure classification for progress-bounded `fsync`.
///
/// Intentionally omits `EAGAIN`: `fsync` waits while chunks are actively
/// draining and only fails on a genuine stall or I/O error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsyncErrorKind {
    /// Transport stalled / unreachable (`ENETUNREACH`).
    Enetunreach,
    /// Hard I/O or protocol failure (`EIO`).
    Eio,
}

/// Observable progress states emitted during [`fsync_journal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsyncState {
    Idle,
    Draining {
        remaining_chunks: usize,
        uploaded_chunks: usize,
    },
    Synced {
        generation: u64,
    },
    Failed(FsyncErrorKind),
}

/// Error returned by [`fsync_journal`].
///
/// The [`CommitJournal`] is always preserved intact on error so callers can
/// resume once connectivity recovers.
#[derive(Debug, Error)]
pub enum FsyncError {
    #[error("fsync stalled (ENETUNREACH) with {remaining_chunks} chunks still journaled: {detail}")]
    Enetunreach {
        remaining_chunks: usize,
        detail: String,
    },
    #[error("fsync i/o failure (EIO) with {remaining_chunks} chunks still journaled: {detail}")]
    Eio {
        remaining_chunks: usize,
        detail: String,
    },
}

impl FsyncError {
    pub fn kind(&self) -> FsyncErrorKind {
        match self {
            Self::Enetunreach { .. } => FsyncErrorKind::Enetunreach,
            Self::Eio { .. } => FsyncErrorKind::Eio,
        }
    }
}

/// Configuration for progress-bounded [`fsync_journal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FsyncConfig {
    /// Maximum duration tolerated without forward chunk-drain progress before
    /// failing with `Enetunreach` / `Eio`.
    pub stall_timeout: Duration,
    /// Sleep duration between retries while within `stall_timeout`.
    pub retry_backoff: Duration,
    /// Maximum number of chunks sent per `push_chunks` call.
    pub batch_size: usize,
    /// QoS class used for `fsync` uploads.
    pub qos: TransportQoS,
}

impl Default for FsyncConfig {
    fn default() -> Self {
        Self {
            stall_timeout: Duration::from_secs(5),
            retry_backoff: Duration::from_millis(10),
            batch_size: 8,
            qos: TransportQoS::Interactive,
        }
    }
}

/// Executes a progress-bounded `fsync` for `journal` over `transport`.
///
/// - Waits indefinitely while chunks are actively draining (every accepted
///   chunk resets the stall timer).
/// - Never returns `EAGAIN`.
/// - On stall (`elapsed >= stall_timeout`), returns `FsyncError::Enetunreach`
///   (or `FsyncError::Eio` on hard I/O error) while preserving all un-uploaded
///   chunks in `journal`.
///
/// **Blocking:** this runs on the calling thread and sleeps `retry_backoff`
/// between retries. With the default config it can block for up to 5 s with no
/// progress before it fails, and longer while chunks keep draining. Do not call
/// it from a session's I/O path (a FUSE read, write or fsync served to a
/// session, or a daemon RPC handler). Run it on a background worker instead.
pub fn fsync_journal<F>(
    journal: &mut CommitJournal,
    transport: &dyn ChunkTransport,
    cfg: &FsyncConfig,
    mut on_state: F,
) -> Result<CommitResult, FsyncError>
where
    F: FnMut(FsyncState),
{
    let batch_size = cfg.batch_size.max(1);
    let mut last_progress = Instant::now();

    if !journal.pending_chunks.is_empty() {
        on_state(FsyncState::Draining {
            remaining_chunks: journal.pending_chunks.len(),
            uploaded_chunks: journal.uploaded_chunks.len(),
        });
    }

    // Phase 1: drain all pending chunks with progress-bounded stall detection.
    while !journal.pending_chunks.is_empty() {
        let batch: Vec<(ChunkHash, Vec<u8>)> = journal
            .pending_chunks
            .iter()
            .take(batch_size)
            .map(|(&h, data)| (h, data.clone()))
            .collect();

        match transport.push_chunks(&batch, cfg.qos) {
            Ok(0) => {
                if last_progress.elapsed() >= cfg.stall_timeout {
                    on_state(FsyncState::Failed(FsyncErrorKind::Enetunreach));
                    return Err(FsyncError::Enetunreach {
                        remaining_chunks: journal.pending_chunks.len(),
                        detail: "transport accepted 0 chunks until stall timeout".into(),
                    });
                }
                std::thread::sleep(cfg.retry_backoff);
            }
            Ok(pushed) => {
                let accepted = pushed.min(batch.len());
                for (h, _) in &batch[..accepted] {
                    journal.pending_chunks.remove(h);
                    journal.uploaded_chunks.push(*h);
                }
                last_progress = Instant::now();
                on_state(FsyncState::Draining {
                    remaining_chunks: journal.pending_chunks.len(),
                    uploaded_chunks: journal.uploaded_chunks.len(),
                });
            }
            Err(TransportError::NetworkUnreachable(msg)) => {
                if last_progress.elapsed() >= cfg.stall_timeout {
                    on_state(FsyncState::Failed(FsyncErrorKind::Enetunreach));
                    return Err(FsyncError::Enetunreach {
                        remaining_chunks: journal.pending_chunks.len(),
                        detail: msg,
                    });
                }
                std::thread::sleep(cfg.retry_backoff);
            }
            Err(other) => {
                on_state(FsyncState::Failed(FsyncErrorKind::Eio));
                return Err(FsyncError::Eio {
                    remaining_chunks: journal.pending_chunks.len(),
                    detail: other.to_string(),
                });
            }
        }
    }

    // Phase 2: commit the file manifest once all chunks are durable.
    loop {
        match transport.commit_file(
            &journal.path,
            journal.base_generation,
            journal.target_manifest.clone(),
        ) {
            Ok(result) => {
                journal.committed = true;
                on_state(FsyncState::Synced {
                    generation: result.generation,
                });
                return Ok(result);
            }
            Err(TransportError::NetworkUnreachable(msg)) => {
                if last_progress.elapsed() >= cfg.stall_timeout {
                    on_state(FsyncState::Failed(FsyncErrorKind::Enetunreach));
                    return Err(FsyncError::Enetunreach {
                        remaining_chunks: 0,
                        detail: msg,
                    });
                }
                std::thread::sleep(cfg.retry_backoff);
            }
            Err(other) => {
                on_state(FsyncState::Failed(FsyncErrorKind::Eio));
                return Err(FsyncError::Eio {
                    remaining_chunks: 0,
                    detail: other.to_string(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::LoopbackTransport;

    #[test]
    fn cow_overlay_partial_writes_coalescing_reads_and_truncate() {
        let transport = LoopbackTransport::new();
        // 512 KiB base file with deterministic byte pattern.
        let base: Vec<u8> = (0..(512 * 1024usize)).map(|i| (i % 251) as u8).collect();
        let manifest = transport.seed_file("/repo/data.bin", &base);
        assert!(manifest.chunks.len() >= 2);

        let mut cow = CowOverlay::new("/repo/data.bin", manifest.clone());

        // Partial-chunk writes that coalesce when adjacent/overlapping.
        cow.pwrite(100, b"HELLO");
        cow.pwrite(105, b"_WORLD");
        assert_eq!(cow.dirty_ranges(), vec![(100, 111)]);

        // Overlapping write across a chunk boundary.
        let boundary = manifest.chunks[0].end();
        let cross_payload = vec![0xAA; 128];
        cow.pwrite(boundary - 64, &cross_payload);
        assert_eq!(cow.dirty_ranges().len(), 2);

        // Read spanning clean remote bytes + dirty local overlay + clean remote bytes.
        let read_back = cow
            .pread_with_transport(90, 32, &transport, TransportQoS::Interactive, None)
            .unwrap();
        let mut expected = base[90..122].to_vec();
        expected[10..21].copy_from_slice(b"HELLO_WORLD");
        assert_eq!(read_back, expected);

        // Read spanning the chunk boundary overlay.
        let cross_read = cow
            .pread_with_transport(
                boundary - 80,
                160,
                &transport,
                TransportQoS::Interactive,
                None,
            )
            .unwrap();
        assert_eq!(&cross_read[..16], &base[(boundary - 80) as usize..(boundary - 64) as usize]);
        assert_eq!(&cross_read[16..144], &cross_payload[..]);
        assert_eq!(&cross_read[144..160], &base[(boundary + 64) as usize..(boundary + 80) as usize]);

        // Truncate down then extend back: extended tail must read as zeros, not old base bytes.
        cow.truncate(108);
        assert_eq!(cow.size(), 108);
        assert_eq!(cow.dirty_ranges(), vec![(100, 108)]);

        cow.truncate(140);
        assert_eq!(cow.size(), 140);
        let after_extend = cow
            .pread_with_transport(100, 40, &transport, TransportQoS::Interactive, None)
            .unwrap();
        let mut expected_ext = vec![0u8; 40];
        expected_ext[..8].copy_from_slice(b"HELLO_WO");
        assert_eq!(after_extend, expected_ext);
    }
}
