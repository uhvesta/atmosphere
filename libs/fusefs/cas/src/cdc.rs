//! FastCDC content-defined chunker (`min = 64 KiB`, `avg = 256 KiB`, `max = 1 MiB`).

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::hash::{ChunkHash, hash_bytes};

/// Default minimum chunk size (64 KiB).
pub const DEFAULT_MIN_SIZE: usize = 64 * 1024;
/// Default average chunk size (256 KiB).
pub const DEFAULT_AVG_SIZE: usize = 256 * 1024;
/// Default maximum chunk size (1 MiB).
pub const DEFAULT_MAX_SIZE: usize = 1024 * 1024;

/// Deterministic 256-entry Gear hash table derived via SplitMix64.
const fn build_gear_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut i = 0;
    while i < 256 {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        table[i] = z ^ (z >> 31);
        i += 1;
    }
    table
}

const GEAR: [u64; 256] = build_gear_table();

/// FastCDC normalized chunking configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FastCdcConfig {
    pub min_size: usize,
    pub avg_size: usize,
    pub max_size: usize,
    /// Stricter mask used between `min_size` and `avg_size`.
    pub mask_s: u64,
    /// Looser mask used between `avg_size` and `max_size`.
    pub mask_l: u64,
}

impl Default for FastCdcConfig {
    fn default() -> Self {
        Self::fusefs_default()
    }
}

impl FastCdcConfig {
    /// Default FUSEFS parameters: 64 KiB min, 256 KiB avg, 1 MiB max.
    pub const fn fusefs_default() -> Self {
        // avg = 256 KiB = 2^18. Normalized level 2: mask_s has 20 bits, mask_l has 16 bits.
        Self {
            min_size: DEFAULT_MIN_SIZE,
            avg_size: DEFAULT_AVG_SIZE,
            max_size: DEFAULT_MAX_SIZE,
            mask_s: 0x0000_D903_0353_0000,
            mask_l: 0x0000_D900_0350_0000,
        }
    }
}

/// A single content-defined chunk slice within a byte buffer or file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkSlice {
    /// Byte offset from the start of the input stream.
    pub offset: u64,
    /// Length of the chunk in bytes.
    pub len: u32,
    /// BLAKE3 digest of the chunk bytes.
    pub hash: ChunkHash,
}

impl ChunkSlice {
    /// Returns the exclusive end offset of this chunk.
    #[inline]
    pub const fn end(&self) -> u64 {
        self.offset + self.len as u64
    }
}

/// Computes content-defined chunk byte ranges for `data`.
pub fn chunk_ranges(data: &[u8], cfg: FastCdcConfig) -> Vec<Range<usize>> {
    assert!(cfg.min_size > 0 && cfg.min_size <= cfg.avg_size && cfg.avg_size <= cfg.max_size);

    let mut chunks = Vec::new();
    let mut offset = 0;
    let total = data.len();

    while offset < total {
        let remaining = total - offset;
        if remaining <= cfg.min_size {
            chunks.push(offset..total);
            break;
        }
        let max_end = offset + remaining.min(cfg.max_size);
        let normal_end = offset + remaining.min(cfg.avg_size);
        let mut cursor = offset + cfg.min_size;
        let mut fp: u64 = 0;
        let mut cut = max_end;

        while cursor < normal_end {
            fp = (fp << 1).wrapping_add(GEAR[data[cursor] as usize]);
            if (fp & cfg.mask_s) == 0 {
                cut = cursor + 1;
                break;
            }
            cursor += 1;
        }
        if cut == max_end {
            while cursor < max_end {
                fp = (fp << 1).wrapping_add(GEAR[data[cursor] as usize]);
                if (fp & cfg.mask_l) == 0 {
                    cut = cursor + 1;
                    break;
                }
                cursor += 1;
            }
        }
        chunks.push(offset..cut);
        offset = cut;
    }
    chunks
}

/// Splits `data` into content-defined [`ChunkSlice`]s with BLAKE3 hashes.
pub fn chunk_bytes(data: &[u8], cfg: FastCdcConfig) -> Vec<ChunkSlice> {
    chunk_ranges(data, cfg)
        .into_iter()
        .map(|range| {
            let slice = &data[range.start..range.end];
            ChunkSlice {
                offset: range.start as u64,
                len: slice.len() as u32,
                hash: hash_bytes(slice),
            }
        })
        .collect()
}

/// Convenience wrapper using [`FastCdcConfig::fusefs_default`].
pub fn chunk_default(data: &[u8]) -> Vec<ChunkSlice> {
    chunk_bytes(data, FastCdcConfig::fusefs_default())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn empty_and_small_inputs() {
        assert!(chunk_default(b"").is_empty());

        let small = vec![42u8; 1024];
        let slices = chunk_default(&small);
        assert_eq!(slices.len(), 1);
        assert_eq!(slices[0].offset, 0);
        assert_eq!(slices[0].len, 1024);
        assert_eq!(slices[0].hash, hash_bytes(&small));
    }

    #[test]
    fn covers_entire_buffer_within_bounds() {
        let cfg = FastCdcConfig::fusefs_default();
        let data: Vec<u8> = (0..(2 * 1024 * 1024))
            .map(|i: usize| ((i.wrapping_mul(31) ^ (i >> 5)) & 0xFF) as u8)
            .collect();
        let slices = chunk_bytes(&data, cfg);
        assert!(!slices.is_empty());
        assert_eq!(slices.first().unwrap().offset, 0);
        assert_eq!(slices.last().unwrap().end(), data.len() as u64);

        for (idx, s) in slices.iter().enumerate() {
            let len = s.len as usize;
            assert!(len <= cfg.max_size);
            if idx + 1 < slices.len() {
                assert!(len >= cfg.min_size);
                assert_eq!(s.end(), slices[idx + 1].offset);
            }
            let expected = hash_bytes(&data[s.offset as usize..s.end() as usize]);
            assert_eq!(s.hash, expected);
        }
    }

    #[test]
    fn boundary_stability_on_64b_insertion() {
        let mut base = Vec::with_capacity(2 * 1024 * 1024);
        let mut x: u64 = 0xCAFE_BABE_DEAD_BEEF;
        while base.len() < 2 * 1024 * 1024 {
            x = x
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            base.extend_from_slice(&x.to_le_bytes());
        }

        let insert_at = 128 * 1024;
        let mut edited = Vec::with_capacity(base.len() + 64);
        edited.extend_from_slice(&base[..insert_at]);
        edited.extend_from_slice(&[b'X'; 64]);
        edited.extend_from_slice(&base[insert_at..]);

        let base_slices = chunk_default(&base);
        let edited_slices = chunk_default(&edited);

        let base_hashes: HashSet<ChunkHash> = base_slices.iter().map(|s| s.hash).collect();
        let reused_bytes: usize = edited_slices
            .iter()
            .filter(|s| base_hashes.contains(&s.hash))
            .map(|s| s.len as usize)
            .sum();

        let dedup_ratio = reused_bytes as f64 / edited.len() as f64;
        assert!(
            dedup_ratio > 0.60,
            "FastCDC boundary stability too low after 64B insert: dedup_ratio={dedup_ratio:.3}"
        );
    }
}
