//! Pure-Rust FastCDC (64 KiB min / 256 KiB avg / 1 MiB max) and fixed 256 KiB
//! chunkers for Gate T1 benchmarking.

use std::ops::Range;

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

/// FastCDC configuration (normalized chunking).
#[derive(Debug, Clone, Copy)]
pub struct FastCdcConfig {
    pub min_size: usize,
    pub avg_size: usize,
    pub max_size: usize,
    /// Stricter mask used between `min_size` and `avg_size`.
    pub mask_s: u64,
    /// Looser mask used between `avg_size` and `max_size`.
    pub mask_l: u64,
}

impl FastCdcConfig {
    /// Spec default for FUSEFS T1: 64 KiB min, 256 KiB avg, 1 MiB max.
    pub const fn fusefs_default() -> Self {
        // avg = 256 KiB = 2^18. Normalized level 2: mask_s has 20 bits, mask_l has 16 bits.
        Self {
            min_size: 64 * 1024,
            avg_size: 256 * 1024,
            max_size: 1024 * 1024,
            mask_s: 0x0000_D903_0353_0000, // ~20 set bits
            mask_l: 0x0000_D900_0350_0000, // ~16 set bits
        }
    }
}

/// Splits `data` into content-defined chunk ranges using FastCDC.
pub fn chunk_fastcdc(data: &[u8], cfg: FastCdcConfig) -> Vec<Range<usize>> {
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

/// Splits `data` into fixed-size chunks (256 KiB default).
pub fn chunk_fixed(data: &[u8], chunk_size: usize) -> Vec<Range<usize>> {
    assert!(chunk_size > 0);
    let mut chunks = Vec::new();
    let mut offset = 0;
    while offset < data.len() {
        let end = (offset + chunk_size).min(data.len());
        chunks.push(offset..end);
        offset = end;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fastcdc_covers_entire_buffer_within_bounds() {
        let cfg = FastCdcConfig::fusefs_default();
        let data: Vec<u8> = (0..(2 * 1024 * 1024))
            .map(|i: usize| ((i.wrapping_mul(31) ^ (i >> 5)) & 0xFF) as u8)
            .collect();
        let ranges = chunk_fastcdc(&data, cfg);
        assert!(!ranges.is_empty());
        assert_eq!(ranges.first().unwrap().start, 0);
        assert_eq!(ranges.last().unwrap().end, data.len());
        for (idx, r) in ranges.iter().enumerate() {
            let len = r.len();
            assert!(len <= cfg.max_size);
            if idx + 1 < ranges.len() {
                assert!(len >= cfg.min_size);
                assert_eq!(r.end, ranges[idx + 1].start);
            }
        }
    }
}
