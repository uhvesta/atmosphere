//! Sharded on-disk BLAKE3 chunk store (`<root>/<h0..2>/<h2..4>/<hex>`) with
//! atomic temp+rename writes, byte-budget LRU eviction, and pinned-chunk
//! protection.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::hash::{ChunkHash, hash_bytes};

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Eviction priority classes ordered from lowest (`Bulk`) to highest (`Pinned`).
///
/// Eviction order: `Bulk` is evicted first, then `Interactive`, then
/// `ActiveWrite`. `Pinned` chunks are never evicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EvictionPriority {
    Bulk = 0,
    Interactive = 1,
    ActiveWrite = 2,
    Pinned = 3,
}

/// Configuration for [`ChunkStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkStoreConfig {
    /// Soft byte budget enforced by priority-aware LRU eviction.
    pub max_bytes: u64,
}

impl Default for ChunkStoreConfig {
    fn default() -> Self {
        Self {
            max_bytes: 256 * 1024 * 1024,
        }
    }
}

/// Errors produced by [`ChunkStore`].
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("chunk hash mismatch: expected {expected}, computed {actual}")]
    HashMismatch {
        expected: ChunkHash,
        actual: ChunkHash,
    },
}

#[derive(Debug, Clone, Copy)]
struct ChunkMeta {
    size: u64,
    priority: EvictionPriority,
    last_access: u64,
    pin_count: u32,
}

impl ChunkMeta {
    fn is_protected(&self) -> bool {
        self.priority == EvictionPriority::Pinned || self.pin_count > 0
    }
}

/// Sharded on-disk BLAKE3 content-addressed chunk store.
pub struct ChunkStore {
    root: PathBuf,
    config: ChunkStoreConfig,
    index: HashMap<ChunkHash, ChunkMeta>,
    used_bytes: u64,
    access_clock: u64,
}

impl ChunkStore {
    /// Opens or initializes a sharded chunk store at `root`.
    ///
    /// Existing sharded chunk files under `root` are indexed automatically.
    pub fn open(root: impl Into<PathBuf>, config: ChunkStoreConfig) -> Result<Self, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root)?;

        let mut store = Self {
            root,
            config,
            index: HashMap::new(),
            used_bytes: 0,
            access_clock: 0,
        };
        store.scan_existing()?;
        Ok(store)
    }

    /// Returns the root directory of this store.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the configured byte budget.
    pub fn max_bytes(&self) -> u64 {
        self.config.max_bytes
    }

    /// Returns the total bytes currently tracked in the store.
    pub fn used_bytes(&self) -> u64 {
        self.used_bytes
    }

    /// Returns the number of chunks currently stored.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns `true` if the store contains no chunks.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Computes the sharded on-disk path `<root>/<h0..2>/<h2..4>/<hex>` for `hash`.
    pub fn chunk_path(&self, hash: &ChunkHash) -> PathBuf {
        let hex = hash.to_hex();
        self.root.join(&hex[0..2]).join(&hex[2..4]).join(&hex)
    }

    /// Returns `true` if `hash` is present in the store index.
    pub fn contains(&self, hash: &ChunkHash) -> bool {
        self.index.contains_key(hash)
    }

    /// Hashes `bytes` with BLAKE3, writes the chunk atomically, and enforces
    /// the byte budget via priority-aware LRU eviction.
    pub fn put(&mut self, bytes: &[u8], priority: EvictionPriority) -> Result<ChunkHash, StoreError> {
        let hash = hash_bytes(bytes);
        self.put_verified(hash, bytes, priority)?;
        Ok(hash)
    }

    /// Verifies that `hash_bytes(bytes) == hash`, writes the chunk atomically,
    /// and enforces the byte budget.
    pub fn put_verified(
        &mut self,
        hash: ChunkHash,
        bytes: &[u8],
        priority: EvictionPriority,
    ) -> Result<(), StoreError> {
        let actual = hash_bytes(bytes);
        if actual != hash {
            return Err(StoreError::HashMismatch {
                expected: hash,
                actual,
            });
        }

        let tick = self.next_tick();
        if let Some(meta) = self.index.get_mut(&hash) {
            meta.last_access = tick;
            if priority > meta.priority {
                meta.priority = priority;
            }
            return Ok(());
        }

        let path = self.chunk_path(&hash);
        atomic_write(&path, bytes)?;

        let size = bytes.len() as u64;
        self.index.insert(
            hash,
            ChunkMeta {
                size,
                priority,
                last_access: tick,
                pin_count: 0,
            },
        );
        self.used_bytes = self.used_bytes.saturating_add(size);
        self.evict_to_budget(Some(hash))?;
        Ok(())
    }

    /// Reads a chunk by hash, refreshing its LRU access timestamp on hit.
    pub fn get(&mut self, hash: &ChunkHash) -> Result<Option<Vec<u8>>, StoreError> {
        if !self.index.contains_key(hash) {
            return Ok(None);
        }
        let path = self.chunk_path(hash);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(meta) = self.index.remove(hash) {
                    self.used_bytes = self.used_bytes.saturating_sub(meta.size);
                }
                return Ok(None);
            }
            Err(e) => return Err(StoreError::Io(e)),
        };

        let tick = self.next_tick();
        if let Some(meta) = self.index.get_mut(hash) {
            meta.last_access = tick;
        }
        Ok(Some(bytes))
    }

    /// Increments the pin count on `hash`, protecting it from eviction.
    pub fn pin(&mut self, hash: &ChunkHash) -> bool {
        if let Some(meta) = self.index.get_mut(hash) {
            meta.pin_count = meta.pin_count.saturating_add(1);
            true
        } else {
            false
        }
    }

    /// Decrements the pin count on `hash`.
    pub fn unpin(&mut self, hash: &ChunkHash) -> bool {
        if let Some(meta) = self.index.get_mut(hash) {
            meta.pin_count = meta.pin_count.saturating_sub(1);
            true
        } else {
            false
        }
    }

    /// Updates the eviction priority of an existing chunk.
    pub fn set_priority(&mut self, hash: &ChunkHash, priority: EvictionPriority) -> bool {
        if let Some(meta) = self.index.get_mut(hash) {
            meta.priority = priority;
            true
        } else {
            false
        }
    }

    /// Explicitly evicts eligible chunks until `used_bytes <= max_bytes` or no
    /// non-pinned chunks remain. Returns the number of evicted chunks.
    pub fn evict_to_budget(&mut self, protected: Option<ChunkHash>) -> Result<usize, StoreError> {
        let mut evicted = 0usize;
        while self.used_bytes > self.config.max_bytes {
            let victim = self
                .index
                .iter()
                .filter(|&(&h, meta)| !meta.is_protected() && Some(h) != protected)
                .min_by_key(|&(_, meta)| (meta.priority, meta.last_access))
                .map(|(&h, _)| h);

            let Some(victim_hash) = victim else {
                break;
            };

            let path = self.chunk_path(&victim_hash);
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(StoreError::Io(e)),
            }

            if let Some(meta) = self.index.remove(&victim_hash) {
                self.used_bytes = self.used_bytes.saturating_sub(meta.size);
                evicted += 1;
            }
        }
        Ok(evicted)
    }

    fn next_tick(&mut self) -> u64 {
        self.access_clock = self.access_clock.wrapping_add(1);
        self.access_clock
    }

    fn scan_existing(&mut self) -> Result<(), StoreError> {
        let Ok(level1) = fs::read_dir(&self.root) else {
            return Ok(());
        };
        for d1 in level1.flatten() {
            if !d1.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let Ok(level2) = fs::read_dir(d1.path()) else {
                continue;
            };
            for d2 in level2.flatten() {
                if !d2.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let Ok(files) = fs::read_dir(d2.path()) else {
                    continue;
                };
                for entry in files.flatten() {
                    let name = entry.file_name();
                    let Some(name_str) = name.to_str() else {
                        continue;
                    };
                    if name_str.starts_with('.') {
                        continue;
                    }
                    let Ok(hash) = ChunkHash::from_hex(name_str) else {
                        continue;
                    };
                    let Ok(md) = entry.metadata() else {
                        continue;
                    };
                    if !md.is_file() {
                        continue;
                    }
                    let size = md.len();
                    let tick = self.next_tick();
                    self.index.insert(
                        hash,
                        ChunkMeta {
                            size,
                            priority: EvictionPriority::Bulk,
                            last_access: tick,
                            pin_count: 0,
                        },
                    );
                    self.used_bytes = self.used_bytes.saturating_add(size);
                }
            }
        }
        Ok(())
    }
}

/// Atomically writes `bytes` to `dest` via a temp file in the same directory
/// followed by `sync_all` and `rename`.
fn atomic_write(dest: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let parent = dest
        .parent()
        .expect("sharded chunk path always has a parent directory");
    fs::create_dir_all(parent)?;

    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("chunk");
    let tmp_path = parent.join(format!(".{file_name}.tmp.{}.{seq}", std::process::id()));

    let mut file = File::create(&tmp_path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);

    fs::rename(&tmp_path, dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sharded_path_and_atomic_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = ChunkStore::open(dir.path(), ChunkStoreConfig::default()).unwrap();

        let data = b"hello sharded cas store";
        let hash = store.put(data, EvictionPriority::Interactive).unwrap();
        let hex = hash.to_hex();
        let expected_path = dir.path().join(&hex[0..2]).join(&hex[2..4]).join(&hex);
        assert_eq!(store.chunk_path(&hash), expected_path);
        assert!(expected_path.is_file());

        // No leftover temp files in shard dir.
        let shard_entries: Vec<_> = fs::read_dir(expected_path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(shard_entries, vec![hex]);

        assert_eq!(store.get(&hash).unwrap().as_deref(), Some(data.as_slice()));

        // Re-opening discovers existing chunk.
        let mut reopened = ChunkStore::open(dir.path(), ChunkStoreConfig::default()).unwrap();
        assert!(reopened.contains(&hash));
        assert_eq!(reopened.used_bytes(), data.len() as u64);
        assert_eq!(
            reopened.get(&hash).unwrap().as_deref(),
            Some(data.as_slice())
        );
    }

    #[test]
    fn lru_eviction_respects_priority_and_pinned_protection() {
        let dir = tempfile::tempdir().unwrap();
        // Budget fits 3 chunks of 100 bytes each.
        let cfg = ChunkStoreConfig { max_bytes: 300 };
        let mut store = ChunkStore::open(dir.path(), cfg).unwrap();

        let pinned_data = vec![1u8; 100];
        let interactive_data = vec![2u8; 100];
        let bulk_old_data = vec![3u8; 100];
        let bulk_new_data = vec![4u8; 100];

        let h_pinned = store.put(&pinned_data, EvictionPriority::Pinned).unwrap();
        let h_interactive = store
            .put(&interactive_data, EvictionPriority::Interactive)
            .unwrap();
        let h_bulk_old = store.put(&bulk_old_data, EvictionPriority::Bulk).unwrap();
        assert_eq!(store.used_bytes(), 300);

        // Inserting a 4th chunk must evict `h_bulk_old` (lowest priority).
        let h_bulk_new = store.put(&bulk_new_data, EvictionPriority::Bulk).unwrap();
        assert_eq!(store.used_bytes(), 300);
        assert!(!store.contains(&h_bulk_old));
        assert!(store.contains(&h_pinned));
        assert!(store.contains(&h_interactive));
        assert!(store.contains(&h_bulk_new));

        // Insert an ActiveWrite chunk: must evict `h_bulk_new` (Bulk < Interactive < ActiveWrite).
        let active_data = vec![5u8; 100];
        let h_active = store
            .put(&active_data, EvictionPriority::ActiveWrite)
            .unwrap();
        assert!(!store.contains(&h_bulk_new));
        assert!(store.contains(&h_pinned));
        assert!(store.contains(&h_interactive));
        assert!(store.contains(&h_active));

        // Insert another ActiveWrite chunk: must evict `h_interactive` (Interactive < ActiveWrite),
        // keeping `h_pinned` intact.
        let active_data_2 = vec![6u8; 100];
        let h_active_2 = store
            .put(&active_data_2, EvictionPriority::ActiveWrite)
            .unwrap();
        assert!(!store.contains(&h_interactive));
        assert!(store.contains(&h_pinned));
        assert!(store.contains(&h_active));
        assert!(store.contains(&h_active_2));
    }

    #[test]
    fn explicit_pin_protects_bulk_chunk_from_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = ChunkStoreConfig { max_bytes: 200 };
        let mut store = ChunkStore::open(dir.path(), cfg).unwrap();

        let h1 = store.put(&vec![10u8; 100], EvictionPriority::Bulk).unwrap();
        let h2 = store.put(&vec![20u8; 100], EvictionPriority::Bulk).unwrap();
        assert!(store.pin(&h1));

        // Even though h1 is older, it is pinned so h2 must be evicted instead.
        let h3 = store.put(&vec![30u8; 100], EvictionPriority::Bulk).unwrap();
        assert!(store.contains(&h1));
        assert!(!store.contains(&h2));
        assert!(store.contains(&h3));
    }
}
