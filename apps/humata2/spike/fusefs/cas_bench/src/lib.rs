//! Gate T1 CAS wire benchmark: fixed 256 KiB vs FastCDC (64K/256K/1M) with
//! BLAKE3 + `{none, lz4, zstd}` on a synthetic repo-like tree, including
//! boundary-shift deduplication after a small mid-file insertion.

pub mod blake3;
pub mod fastcdc;

use std::collections::HashSet;
use std::ops::Range;
use std::time::{Duration, Instant};

use fastcdc::{FastCdcConfig, chunk_fastcdc, chunk_fixed};

/// Chunking strategy compared in T1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkerKind {
    Fixed256K,
    FastCdc,
}

impl ChunkerKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed256K => "fixed-256KiB",
            Self::FastCdc => "fastcdc-64K/256K/1M",
        }
    }

    pub fn chunk(self, data: &[u8]) -> Vec<Range<usize>> {
        match self {
            Self::Fixed256K => chunk_fixed(data, 256 * 1024),
            Self::FastCdc => chunk_fastcdc(data, FastCdcConfig::fusefs_default()),
        }
    }
}

/// Wire compression codec compared in T1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodecKind {
    None,
    Lz4,
    Zstd,
}

impl CodecKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lz4 => "lz4",
            Self::Zstd => "zstd-3",
        }
    }

    pub fn compress(self, raw: &[u8]) -> Vec<u8> {
        match self {
            Self::None => raw.to_vec(),
            Self::Lz4 => lz4_flex::compress_prepend_size(raw),
            Self::Zstd => zstd::bulk::compress(raw, 3).expect("zstd compress"),
        }
    }
}

/// Summary metrics for one `(ChunkerKind, CodecKind)` run.
#[derive(Debug, Clone)]
pub struct BenchRow {
    pub chunker: ChunkerKind,
    pub codec: CodecKind,
    pub raw_bytes: usize,
    pub wire_bytes: usize,
    pub total_chunks: usize,
    pub unique_chunks: usize,
    pub elapsed: Duration,
    /// Bytes that must be uploaded after inserting 64 bytes at offset 128 KiB
    /// into a 2 MiB repo-like file.
    pub edit_upload_raw_bytes: usize,
    pub edit_file_bytes: usize,
}

impl BenchRow {
    pub fn throughput_mib_s(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64().max(1e-9);
        (self.raw_bytes as f64 / (1024.0 * 1024.0)) / secs
    }

    pub fn compression_ratio(&self) -> f64 {
        if self.raw_bytes == 0 {
            1.0
        } else {
            self.wire_bytes as f64 / self.raw_bytes as f64
        }
    }

    pub fn edit_dedup_ratio(&self) -> f64 {
        if self.edit_file_bytes == 0 {
            0.0
        } else {
            1.0 - (self.edit_upload_raw_bytes as f64 / self.edit_file_bytes as f64)
        }
    }
}

/// Generates a deterministic synthetic repo-like tree (Rust source, Markdown,
/// JSON transcript, and incompressible binary blob).
pub fn generate_synthetic_repo(scale: usize) -> Vec<Vec<u8>> {
    let mut files = Vec::new();

    // 1. Rust-like source file (repetitive identifiers + varied line numbers).
    let mut rust_src = Vec::with_capacity(512 * 1024 * scale);
    for i in 0..(4_000 * scale) {
        rust_src.extend_from_slice(
            format!(
                "pub fn handler_{i:05}(ctx: &mut Context, id: u64) -> Result<usize, VfsError> {{ let v = ctx.lookup(id ^ {i})?; Ok(v.len()) }}\n"
            )
            .as_bytes(),
        );
    }
    files.push(rust_src);

    // 2. Markdown + JSONL transcript-like file.
    let mut transcript = Vec::with_capacity(512 * 1024 * scale);
    for i in 0..(3_500 * scale) {
        transcript.extend_from_slice(
            format!(
                "{{\"turn\":{i},\"role\":\"assistant\",\"msg\":\"Checking BLAKE3 chunk manifest and FSKit mountpoint status for session s-{:04}.\"}}\n",
                i % 64
            )
            .as_bytes(),
        );
    }
    files.push(transcript);

    // 3. Deterministic pseudo-random binary asset (tests incompressible path).
    let bin_len = 512 * 1024 * scale;
    let mut binary = Vec::with_capacity(bin_len);
    let mut x: u64 = 0x1234_5678_9ABC_DEF0;
    for _ in 0..bin_len {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        binary.push((x >> 56) as u8);
    }
    files.push(binary);

    // 4. Duplicate copy of part of the Rust file (tests cross-file chunk dedup).
    files.push(files[0].clone());

    files
}

/// Generates a 2 MiB file and a copy with a 64-byte insertion at offset 128 KiB
/// to measure boundary-shift deduplication.
pub fn generate_boundary_shift_pair() -> (Vec<u8>, Vec<u8>) {
    let mut base = Vec::with_capacity(2 * 1024 * 1024);
    let mut x: u64 = 0xCAFE_BABE_DEAD_BEEF;
    while base.len() < 2 * 1024 * 1024 {
        x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        base.extend_from_slice(&x.to_le_bytes());
    }
    let mut edited = Vec::with_capacity(base.len() + 64);
    let insert_at = 128 * 1024;
    edited.extend_from_slice(&base[..insert_at]);
    edited.extend_from_slice(&[b'X'; 64]);
    edited.extend_from_slice(&base[insert_at..]);
    (base, edited)
}

/// Runs one `(chunker, codec)` benchmark over `corpus` + boundary-shift pair.
pub fn run_bench_row(
    chunker: ChunkerKind,
    codec: CodecKind,
    corpus: &[Vec<u8>],
    shift_pair: &(Vec<u8>, Vec<u8>),
) -> BenchRow {
    let start = Instant::now();
    let mut raw_bytes = 0usize;
    let mut wire_bytes = 0usize;
    let mut total_chunks = 0usize;
    let mut seen: HashSet<[u8; 32]> = HashSet::new();

    for file in corpus {
        raw_bytes += file.len();
        for range in chunker.chunk(file) {
            total_chunks += 1;
            let slice = &file[range];
            let digest = blake3::hash(slice);
            if seen.insert(digest) {
                let encoded = codec.compress(slice);
                wire_bytes += encoded.len();
            }
        }
    }
    let elapsed = start.elapsed();

    // Measure boundary-shift upload bytes on the 2 MiB pair.
    let (base, edited) = shift_pair;
    let base_digests: HashSet<[u8; 32]> = chunker
        .chunk(base)
        .into_iter()
        .map(|r| blake3::hash(&base[r]))
        .collect();
    let edit_upload_raw_bytes: usize = chunker
        .chunk(edited)
        .into_iter()
        .filter_map(|r| {
            let slice = &edited[r];
            let d = blake3::hash(slice);
            (!base_digests.contains(&d)).then_some(slice.len())
        })
        .sum();

    BenchRow {
        chunker,
        codec,
        raw_bytes,
        wire_bytes,
        total_chunks,
        unique_chunks: seen.len(),
        elapsed,
        edit_upload_raw_bytes,
        edit_file_bytes: edited.len(),
    }
}

/// Runs the full 2 x 3 `(chunker x codec)` matrix.
pub fn run_matrix(scale: usize) -> Vec<BenchRow> {
    let corpus = generate_synthetic_repo(scale);
    let shift_pair = generate_boundary_shift_pair();
    let mut rows = Vec::new();
    for chunker in [ChunkerKind::Fixed256K, ChunkerKind::FastCdc] {
        for codec in [CodecKind::None, CodecKind::Lz4, CodecKind::Zstd] {
            rows.push(run_bench_row(chunker, codec, &corpus, &shift_pair));
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fastcdc_beats_fixed_chunking_on_boundary_shift_dedup() {
        let corpus = generate_synthetic_repo(1);
        let shift_pair = generate_boundary_shift_pair();

        let fixed = run_bench_row(ChunkerKind::Fixed256K, CodecKind::None, &corpus, &shift_pair);
        let cdc = run_bench_row(ChunkerKind::FastCdc, CodecKind::None, &corpus, &shift_pair);

        // Fixed 256 KiB invalidates every chunk from 0..end after inserting at 128 KiB.
        assert!(
            fixed.edit_dedup_ratio() < 0.05,
            "fixed chunker unexpectedly deduplicated: {:.2}",
            fixed.edit_dedup_ratio()
        );
        // FastCDC re-synchronizes quickly and deduplicates most of the 2 MiB tail.
        assert!(
            cdc.edit_dedup_ratio() > 0.60,
            "FastCDC dedup ratio too low: {:.2} (upload {} / {})",
            cdc.edit_dedup_ratio(),
            cdc.edit_upload_raw_bytes,
            cdc.edit_file_bytes
        );
    }

    #[test]
    fn lz4_and_zstd_reduce_wire_bytes_on_repo_corpus() {
        let rows = run_matrix(1);
        let find = |ch, co| {
            rows.iter()
                .find(|r| r.chunker == ch && r.codec == co)
                .expect("row")
        };
        let none = find(ChunkerKind::FastCdc, CodecKind::None);
        let lz4 = find(ChunkerKind::FastCdc, CodecKind::Lz4);
        let zstd = find(ChunkerKind::FastCdc, CodecKind::Zstd);

        assert!(lz4.wire_bytes < none.wire_bytes);
        assert!(zstd.wire_bytes <= lz4.wire_bytes);
        // Duplicate Rust file in corpus means unique_chunks < total_chunks.
        assert!(none.unique_chunks < none.total_chunks);
    }
}
