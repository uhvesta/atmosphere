//! Adaptive wire compression planner with incompressible-data detection
//! and source-CPU / queue-pressure shedding.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Compression codec applied to a chunk payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Codec {
    None,
    Lz4,
    Zstd { level: i32 },
}

/// Self-describing encoded chunk payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EncodedChunk {
    pub codec: Codec,
    pub raw_len: u32,
    pub data: Vec<u8>,
}

/// Errors produced by compression or decompression.
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("zstd compression failed: {0}")]
    ZstdCompress(String),
    #[error("zstd decompression failed: {0}")]
    ZstdDecompress(String),
    #[error("lz4 decompression failed: {0}")]
    Lz4Decompress(String),
    #[error("decoded length mismatch: expected {expected}, got {actual}")]
    LengthMismatch { expected: usize, actual: usize },
}

/// Runtime link and source-host signals used by [`AdaptiveCodecPlanner`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CompressionContext {
    /// Estimated link bandwidth in Mbps.
    pub bandwidth_mbps: f64,
    /// Normalized source CPU load in `[0.0, 1.0+]` (e.g., 0.85 = 85% busy).
    pub source_cpu_load: f32,
    /// Normalized encode/upload queue pressure in `[0.0, 1.0]`.
    pub queue_pressure: f32,
}

impl Default for CompressionContext {
    fn default() -> Self {
        Self {
            bandwidth_mbps: 100.0,
            source_cpu_load: 0.25,
            queue_pressure: 0.0,
        }
    }
}

/// Policy thresholds for [`AdaptiveCodecPlanner`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdaptiveCodecConfig {
    /// Default Zstd level when Zstd is selected.
    pub zstd_level: i32,
    /// Bandwidth threshold (Mbps) at or below which Zstd is preferred over LZ4.
    pub zstd_max_bandwidth_mbps: f64,
    /// Source CPU load above which Zstd is shed to LZ4.
    pub cpu_shed_to_lz4_threshold: f32,
    /// Source CPU load above which all compression is shed to `Codec::None`.
    pub cpu_shed_to_none_threshold: f32,
    /// Queue pressure above which Zstd is shed to LZ4.
    pub queue_shed_to_lz4_threshold: f32,
    /// Queue pressure above which all compression is shed to `Codec::None`.
    pub queue_shed_to_none_threshold: f32,
    /// Minimum chunk size in bytes worth attempting to compress.
    pub min_compress_len: usize,
    /// Prefix sample length used for fast incompressible-data detection.
    pub sample_len: usize,
    /// Minimum fractional byte reduction required to keep compressed output.
    pub min_savings_ratio: f64,
}

impl Default for AdaptiveCodecConfig {
    fn default() -> Self {
        Self {
            zstd_level: 3,
            zstd_max_bandwidth_mbps: 200.0,
            cpu_shed_to_lz4_threshold: 0.70,
            cpu_shed_to_none_threshold: 0.90,
            queue_shed_to_lz4_threshold: 0.65,
            queue_shed_to_none_threshold: 0.85,
            min_compress_len: 512,
            sample_len: 4096,
            min_savings_ratio: 0.05,
        }
    }
}

/// Selects and applies per-chunk compression based on link bandwidth,
/// data compressibility, and source CPU / queue pressure.
#[derive(Debug, Clone, Copy, Default)]
pub struct AdaptiveCodecPlanner {
    pub config: AdaptiveCodecConfig,
}

impl AdaptiveCodecPlanner {
    /// Creates a planner with the provided configuration.
    pub const fn new(config: AdaptiveCodecConfig) -> Self {
        Self { config }
    }

    /// Chooses the best [`Codec`] for `data` under `ctx`.
    pub fn plan(&self, data: &[u8], ctx: &CompressionContext) -> Codec {
        let cfg = &self.config;
        if data.len() < cfg.min_compress_len {
            return Codec::None;
        }

        // Source-CPU & queue-pressure shedding: drop all compression under severe load.
        if ctx.source_cpu_load >= cfg.cpu_shed_to_none_threshold
            || ctx.queue_pressure >= cfg.queue_shed_to_none_threshold
        {
            return Codec::None;
        }

        // Pick base codec from link bandwidth.
        let mut chosen = if ctx.bandwidth_mbps <= cfg.zstd_max_bandwidth_mbps {
            Codec::Zstd {
                level: cfg.zstd_level,
            }
        } else {
            Codec::Lz4
        };

        // Shed heavy Zstd down to fast LZ4 under moderate CPU/queue pressure.
        if matches!(chosen, Codec::Zstd { .. })
            && (ctx.source_cpu_load >= cfg.cpu_shed_to_lz4_threshold
                || ctx.queue_pressure >= cfg.queue_shed_to_lz4_threshold)
        {
            chosen = Codec::Lz4;
        }

        // Fast incompressible-data detection via magic sniff + sample probe.
        if looks_already_compressed(data) || !sample_is_compressible(data, cfg) {
            return Codec::None;
        }

        chosen
    }

    /// Plans and encodes `raw` according to `ctx`.
    ///
    /// If the full compressed output fails to shrink by at least
    /// `config.min_savings_ratio`, transparently falls back to `Codec::None`.
    pub fn encode(&self, raw: &[u8], ctx: &CompressionContext) -> Result<EncodedChunk, CodecError> {
        let codec = self.plan(raw, ctx);
        if codec == Codec::None {
            return Ok(EncodedChunk {
                codec: Codec::None,
                raw_len: raw.len() as u32,
                data: raw.to_vec(),
            });
        }

        let encoded = encode_with(raw, codec)?;
        let max_acceptable = ((raw.len() as f64) * (1.0 - self.config.min_savings_ratio)) as usize;
        if encoded.data.len() > max_acceptable {
            return Ok(EncodedChunk {
                codec: Codec::None,
                raw_len: raw.len() as u32,
                data: raw.to_vec(),
            });
        }
        Ok(encoded)
    }
}

/// Encodes `raw` using an explicit [`Codec`].
pub fn encode_with(raw: &[u8], codec: Codec) -> Result<EncodedChunk, CodecError> {
    let data = match codec {
        Codec::None => raw.to_vec(),
        Codec::Lz4 => lz4_flex::block::compress(raw),
        Codec::Zstd { level } => {
            zstd::bulk::compress(raw, level).map_err(|e| CodecError::ZstdCompress(e.to_string()))?
        }
    };
    Ok(EncodedChunk {
        codec,
        raw_len: raw.len() as u32,
        data,
    })
}

/// Decodes an [`EncodedChunk`] back to its exact raw bytes.
pub fn decode(encoded: &EncodedChunk) -> Result<Vec<u8>, CodecError> {
    let expected = encoded.raw_len as usize;
    let out = match encoded.codec {
        Codec::None => encoded.data.clone(),
        Codec::Lz4 => lz4_flex::block::decompress(&encoded.data, expected)
            .map_err(|e| CodecError::Lz4Decompress(e.to_string()))?,
        Codec::Zstd { .. } => zstd::bulk::decompress(&encoded.data, expected)
            .map_err(|e| CodecError::ZstdDecompress(e.to_string()))?,
    };
    if out.len() != expected {
        return Err(CodecError::LengthMismatch {
            expected,
            actual: out.len(),
        });
    }
    Ok(out)
}

/// Checks common magic prefixes of already-compressed formats.
fn looks_already_compressed(data: &[u8]) -> bool {
    if data.len() < 4 {
        return false;
    }
    // gzip, zstd, lz4 frame, zip, png, jpeg, xz, bzip2
    data.starts_with(&[0x1F, 0x8B])
        || data.starts_with(&[0x28, 0xB5, 0x2F, 0xFD])
        || data.starts_with(&[0x04, 0x22, 0x4D, 0x18])
        || data.starts_with(b"PK\x03\x04")
        || data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        || data.starts_with(&[0xFF, 0xD8, 0xFF])
        || data.starts_with(&[0xFD, b'7', b'z', b'X', b'Z', 0x00])
        || data.starts_with(b"BZh")
}

/// Runs a cheap LZ4 probe on a prefix sample to detect incompressible payloads.
fn sample_is_compressible(data: &[u8], cfg: &AdaptiveCodecConfig) -> bool {
    let sample_len = data.len().min(cfg.sample_len);
    let sample = &data[..sample_len];
    let probe = lz4_flex::block::compress(sample);
    let max_probe_len = ((sample_len as f64) * (1.0 - cfg.min_savings_ratio)) as usize;
    probe.len() <= max_probe_len
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compressible_sample() -> Vec<u8> {
        let mut buf = Vec::with_capacity(64 * 1024);
        for i in 0..1024 {
            buf.extend_from_slice(
                format!("fn handle_chunk_{i:04}(offset: u64, len: usize) -> Result<usize, Error>\n")
                    .as_bytes(),
            );
        }
        buf
    }

    fn incompressible_sample() -> Vec<u8> {
        let mut buf = Vec::with_capacity(64 * 1024);
        let mut x: u64 = 0x1234_5678_9ABC_DEF0;
        for _ in 0..(64 * 1024) {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            buf.push((x >> 56) as u8);
        }
        buf
    }

    #[test]
    fn selects_codec_by_bandwidth_and_compressibility() {
        let planner = AdaptiveCodecPlanner::default();
        let text = compressible_sample();
        let random = incompressible_sample();

        let slow_link = CompressionContext {
            bandwidth_mbps: 50.0,
            source_cpu_load: 0.20,
            queue_pressure: 0.10,
        };
        assert_eq!(planner.plan(&text, &slow_link), Codec::Zstd { level: 3 });

        let fast_link = CompressionContext {
            bandwidth_mbps: 1_000.0,
            source_cpu_load: 0.20,
            queue_pressure: 0.10,
        };
        assert_eq!(planner.plan(&text, &fast_link), Codec::Lz4);

        // Incompressible data skips compression regardless of bandwidth.
        assert_eq!(planner.plan(&random, &slow_link), Codec::None);
        assert_eq!(planner.plan(&random, &fast_link), Codec::None);

        // Tiny buffers skip compression.
        assert_eq!(planner.plan(b"tiny payload", &slow_link), Codec::None);
    }

    #[test]
    fn sheds_compression_under_source_cpu_and_queue_pressure() {
        let planner = AdaptiveCodecPlanner::default();
        let text = compressible_sample();

        // Moderate CPU load sheds Zstd -> Lz4 on slow link.
        let moderate_cpu = CompressionContext {
            bandwidth_mbps: 50.0,
            source_cpu_load: 0.78,
            queue_pressure: 0.10,
        };
        assert_eq!(planner.plan(&text, &moderate_cpu), Codec::Lz4);

        // Severe CPU load sheds all compression -> None.
        let severe_cpu = CompressionContext {
            bandwidth_mbps: 50.0,
            source_cpu_load: 0.95,
            queue_pressure: 0.10,
        };
        assert_eq!(planner.plan(&text, &severe_cpu), Codec::None);

        // Moderate queue pressure sheds Zstd -> Lz4.
        let moderate_queue = CompressionContext {
            bandwidth_mbps: 50.0,
            source_cpu_load: 0.20,
            queue_pressure: 0.70,
        };
        assert_eq!(planner.plan(&text, &moderate_queue), Codec::Lz4);

        // High queue pressure sheds to None.
        let high_queue = CompressionContext {
            bandwidth_mbps: 50.0,
            source_cpu_load: 0.20,
            queue_pressure: 0.90,
        };
        assert_eq!(planner.plan(&text, &high_queue), Codec::None);
    }

    #[test]
    fn encode_decode_roundtrip_all_codecs() {
        let text = compressible_sample();
        for codec in [Codec::None, Codec::Lz4, Codec::Zstd { level: 3 }] {
            let encoded = encode_with(&text, codec).unwrap();
            assert_eq!(encoded.codec, codec);
            if codec != Codec::None {
                assert!(encoded.data.len() < text.len());
            }
            let decoded = decode(&encoded).unwrap();
            assert_eq!(decoded, text);
        }

        let planner = AdaptiveCodecPlanner::default();
        let random = incompressible_sample();
        let encoded = planner
            .encode(&random, &CompressionContext::default())
            .unwrap();
        assert_eq!(encoded.codec, Codec::None);
        assert_eq!(decode(&encoded).unwrap(), random);
    }
}
