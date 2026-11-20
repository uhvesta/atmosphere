//! `fusefs_cas`: generic content-addressed storage (CAS) primitives for FUSEFS.
//!
//! Provides:
//! - [`hash`]: 256-bit BLAKE3 [`ChunkHash`] with hex parsing/formatting.
//! - [`cdc`]: FastCDC content-defined chunker (`64 KiB / 256 KiB / 1 MiB`).
//! - [`codec`]: Adaptive compression planner (`None`, `Lz4`, `Zstd`) with
//!   incompressible-data detection and source-CPU / queue-pressure shedding.
//! - [`manifest`]: Paged [`DirManifest`] / [`DirPage`] and [`FileManifest`]
//!   with binary and JSON serialization.
//! - [`store`]: Sharded on-disk BLAKE3 [`ChunkStore`] with atomic temp+rename
//!   writes and priority-aware LRU byte-budget eviction.
//! - [`cow`]: Sparse local [`CowOverlay`] with resumable [`CommitJournal`] and
//!   progress-bounded [`fsync_journal`].
//! - [`transport`]: Generic [`ChunkTransport`] trait and in-process
//!   [`LoopbackTransport`].

pub mod cdc;
pub mod codec;
pub mod cow;
pub mod hash;
pub mod manifest;
pub mod store;
pub mod transport;

pub use cdc::{ChunkSlice, FastCdcConfig, chunk_bytes, chunk_default, chunk_ranges};
pub use codec::{
    AdaptiveCodecConfig, AdaptiveCodecPlanner, Codec, CodecError, CompressionContext, EncodedChunk,
    decode as decode_chunk, encode_with as encode_chunk_with,
};
pub use cow::{
    CommitJournal, CowError, CowOverlay, FsyncConfig, FsyncError, FsyncErrorKind, FsyncState,
    fsync_journal,
};
pub use hash::{ChunkHash, HashError, Hasher, hash_bytes};
pub use manifest::{ChunkRef, DirEntry, DirManifest, DirPage, EntryKind, FileManifest, ManifestError};
pub use store::{ChunkStore, ChunkStoreConfig, EvictionPriority, StoreError};
pub use transport::{
    ChunkTransport, CommitResult, LoopbackStats, LoopbackTransport, TransportError, TransportQoS,
};

#[cfg(test)]
mod integration_tests {
    use std::time::Duration;

    use super::*;

    fn synthetic_file(len: usize, seed: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(len);
        let mut x = seed;
        while out.len() < len {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            out.extend_from_slice(&x.to_le_bytes());
        }
        out.truncate(len);
        out
    }

    #[test]
    fn loopback_random_and_sequential_reads_with_cache() {
        let transport = LoopbackTransport::new();
        let data = synthetic_file(1024 * 1024, 0xABCD_EF01_2345_6789);
        let manifest = transport.seed_file("/workspace/blob.bin", &data);

        let dir = tempfile::tempdir().unwrap();
        let mut cache = ChunkStore::open(
            dir.path(),
            ChunkStoreConfig {
                max_bytes: 2 * 1024 * 1024,
            },
        )
        .unwrap();

        let cow = CowOverlay::new("/workspace/blob.bin", manifest);

        // Sequential reads in 32 KiB windows.
        let mut seq_out = Vec::with_capacity(data.len());
        let mut offset = 0u64;
        while (offset as usize) < data.len() {
            let chunk = cow
                .pread_with_transport(
                    offset,
                    32 * 1024,
                    &transport,
                    TransportQoS::Bulk,
                    Some(&mut cache),
                )
                .unwrap();
            offset += chunk.len() as u64;
            seq_out.extend_from_slice(&chunk);
        }
        assert_eq!(seq_out, data);

        // Subsequent random reads should hit the local ChunkStore cache with zero new fetches.
        let fetches_after_seq = transport.stats().fetched_chunks;
        for &(off, len) in &[
            (0u64, 4096usize),
            (123_456, 8192),
            (512_000, 65_536),
            (1024 * 1024 - 100, 100),
        ] {
            let slice = cow
                .pread_with_transport(
                    off,
                    len,
                    &transport,
                    TransportQoS::Interactive,
                    Some(&mut cache),
                )
                .unwrap();
            assert_eq!(slice, &data[off as usize..off as usize + len]);
        }
        assert_eq!(transport.stats().fetched_chunks, fetches_after_seq);
    }

    #[test]
    fn progress_bounded_fsync_succeeds_while_actively_draining_past_stall_timeout() {
        let transport = LoopbackTransport::new();
        let base = synthetic_file(128 * 1024, 0x1111_2222_3333_4444);
        let manifest = transport.seed_file("/workspace/large.bin", &base);

        let mut cow = CowOverlay::new("/workspace/large.bin", manifest);
        // Write 1.5 MiB of new data so FastCDC produces multiple new chunks.
        let new_data = synthetic_file(1536 * 1024, 0x9999_8888_7777_6666);
        cow.pwrite(0, &new_data);

        let mut journal = cow
            .prepare_commit(&transport, TransportQoS::Interactive)
            .unwrap();
        let total_pending = journal.pending_count();
        assert!(
            total_pending >= 4,
            "expected >= 4 pending chunks, got {total_pending}"
        );

        // Drain 1 chunk per call at 15ms each (> 60ms total), while stall_timeout is 45ms.
        // Because each chunk makes forward progress, fsync MUST wait and succeed!
        transport.set_max_chunks_per_push(Some(1));
        transport.set_push_delay(Duration::from_millis(15));

        let cfg = FsyncConfig {
            stall_timeout: Duration::from_millis(45),
            retry_backoff: Duration::from_millis(5),
            batch_size: 1,
            qos: TransportQoS::Interactive,
        };

        let mut states = Vec::new();
        let result = fsync_journal(&mut journal, &transport, &cfg, |s| states.push(s)).unwrap();

        assert_eq!(result.generation, 2);
        assert!(journal.committed);
        assert_eq!(journal.pending_count(), 0);
        assert_eq!(journal.uploaded_count(), total_pending);
        assert!(states.iter().any(|s| matches!(s, FsyncState::Draining { .. })));
        assert_eq!(
            states.last(),
            Some(&FsyncState::Synced { generation: 2 })
        );
        assert_eq!(
            transport.read_file_bytes("/workspace/large.bin").unwrap(),
            new_data
        );
    }

    #[test]
    fn stalled_fsync_fails_enetunreach_preserves_journal_and_resumes_cleanly() {
        let transport = LoopbackTransport::new();
        let base = synthetic_file(128 * 1024, 0xAAAA_BBBB_CCCC_DDDD);
        let manifest = transport.seed_file("/workspace/resumable.bin", &base);

        let mut cow = CowOverlay::new("/workspace/resumable.bin", manifest);
        let new_data = synthetic_file(1536 * 1024, 0x5555_6666_7777_8888);
        cow.pwrite(0, &new_data);

        let mut journal = cow
            .prepare_commit(&transport, TransportQoS::Interactive)
            .unwrap();
        let initial_pending = journal.pending_count();
        assert!(initial_pending >= 4);

        // Allow 2 chunks to drain, then stall the transport.
        transport.set_max_chunks_per_push(Some(1));
        transport.set_stall_after_chunks(Some(2));

        let cfg = FsyncConfig {
            stall_timeout: Duration::from_millis(25),
            retry_backoff: Duration::from_millis(5),
            batch_size: 1,
            qos: TransportQoS::Interactive,
        };

        let mut states = Vec::new();
        let err = fsync_journal(&mut journal, &transport, &cfg, |s| states.push(s)).unwrap_err();
        assert_eq!(err.kind(), FsyncErrorKind::Enetunreach);
        assert_eq!(
            states.last(),
            Some(&FsyncState::Failed(FsyncErrorKind::Enetunreach))
        );

        // Journal must preserve the exact remaining CoW chunks and already-uploaded progress!
        assert!(!journal.committed);
        assert_eq!(journal.uploaded_count(), 2);
        assert_eq!(journal.pending_count(), initial_pending - 2);

        // Persist and reload the journal to prove crash/restart resumability.
        let dir = tempfile::tempdir().unwrap();
        let journal_path = dir.path().join("commit.journal");
        journal.save_to_file(&journal_path).unwrap();
        let mut reloaded = CommitJournal::load_from_file(&journal_path).unwrap();
        assert_eq!(reloaded, journal);

        // Transport recovers; resume uploads only the remaining chunks and commits.
        let pushed_before_resume = transport.stats().pushed_chunks;
        transport.set_stall_after_chunks(None);
        transport.set_stalled(false);

        let result = fsync_journal(&mut reloaded, &transport, &cfg, |_| {}).unwrap();
        assert_eq!(result.generation, 2);
        assert!(reloaded.committed);
        assert_eq!(reloaded.pending_count(), 0);
        assert_eq!(reloaded.uploaded_count(), initial_pending);
        assert_eq!(
            transport.stats().pushed_chunks - pushed_before_resume,
            (initial_pending - 2) as u64
        );

        // Remote file now matches the exact CoW-modified bytes.
        assert_eq!(
            transport.read_file_bytes("/workspace/resumable.bin").unwrap(),
            new_data
        );
    }
}
