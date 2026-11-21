//! On-disk binary framing for hot `.pblog` segments, cold `.pblog.zst` archives,
//! and content-addressed blobs (SPEC.md §3.4, §4.2).

use std::io::Write;

use prost::Message;
use sha2::{Digest, Sha256};

use crate::error::FrameError;
use crate::types::LogEvent;

/// Hot `.pblog` file header magic (`b"\x89LORE\r\n\x1a"`).
pub const HOT_MAGIC: [u8; 8] = *b"\x89LORE\r\n\x1a";

/// Cold `.pblog.zst` archive file header magic (`b"\x89LORZ\r\n\x1a"`).
pub const ARCHIVE_MAGIC: [u8; 8] = *b"\x89LORZ\r\n\x1a";

/// Current binary format version for both hot segments and cold archives.
pub const FORMAT_VERSION: u16 = 1;

/// Default checksum algorithm for hot segments (`crc32fast` SIMD IEEE).
pub const CHECKSUM_ALG_CRC32FAST: u8 = 1;

/// Default codec for cold archives (`zstd` with native XXH64 frame checksum).
pub const ARCHIVE_CODEC_ZSTD: u8 = 1;

/// Default zstd compression level for cold archives and blobs.
pub const ARCHIVE_ZSTD_LEVEL: i32 = 6;

/// Length in bytes of both hot segment headers and cold archive headers.
pub const SEGMENT_HEADER_LEN: usize = 16;

/// Length in bytes of each record frame header (`payload_len + crc32 + seq`).
pub const RECORD_HEADER_LEN: usize = 16;

/// Maximum permitted serialized `LogEvent` payload length (4 MiB).
pub const MAX_RECORD_PAYLOAD_LEN: usize = 4 * 1024 * 1024;

/// Content-addressed blob prefix for uncompressed payloads.
pub const BLOB_MAGIC_RAW: [u8; 4] = *b"LRB0";

/// Content-addressed blob prefix for zstd-compressed payloads.
pub const BLOB_MAGIC_ZSTD: [u8; 4] = *b"LRBZ";

/// Minimum raw blob size eligible for zstd compression (1 KiB).
pub const BLOB_COMPRESS_MIN_BYTES: usize = 1024;

/// Minimum savings shift (`1 / 16` = 6.25%) required to keep zstd blob compression.
const BLOB_SAVING_SHIFT: u32 = 4;

/// Parsed hot `.pblog` segment header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentHeader {
    pub format_ver: u16,
    pub checksum_alg: u8,
    pub flags: u8,
    pub header_crc32: u32,
}

/// Parsed cold `.pblog.zst` archive header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArchiveHeader {
    pub format_ver: u16,
    pub codec: u8,
    pub flags: u8,
    pub header_crc32: u32,
}

/// Tail integrity status returned by [`decode_segment_frames`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailStatus {
    /// All bytes in the segment decoded cleanly.
    Clean,
    /// The final frame was truncated or had a corrupted tail; safe to truncate to `valid_end_offset`.
    TruncatedTail { valid_end_offset: u64 },
    /// A non-tail frame failed integrity verification; must NOT be silently truncated.
    MidSegmentCorrupt { offset: u64, seq: u64 },
}

fn build_16b_header(magic: [u8; 8], alg_or_codec: u8, flags: u8) -> [u8; SEGMENT_HEADER_LEN] {
    let mut buf = [0u8; SEGMENT_HEADER_LEN];
    buf[0..8].copy_from_slice(&magic);
    buf[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    buf[10] = alg_or_codec;
    buf[11] = flags;
    let crc = crc32fast::hash(&buf[0..12]);
    buf[12..16].copy_from_slice(&crc.to_le_bytes());
    buf
}

fn verify_16b_header(
    header: &[u8],
    expected_magic: [u8; 8],
    expected_alg: u8,
) -> std::result::Result<(u16, u8, u8, u32), FrameError> {
    if header.len() < SEGMENT_HEADER_LEN {
        return Err(FrameError::HeaderTooShort {
            expected: SEGMENT_HEADER_LEN,
            actual: header.len(),
        });
    }
    if header[0..8] != expected_magic {
        return Err(FrameError::InvalidMagic);
    }
    let format_ver = u16::from_le_bytes([header[8], header[9]]);
    if format_ver != FORMAT_VERSION {
        return Err(FrameError::UnsupportedVersion {
            version: format_ver,
            expected: FORMAT_VERSION,
        });
    }
    let alg = header[10];
    if alg != expected_alg {
        return Err(FrameError::UnsupportedAlgorithm { algorithm: alg });
    }
    let flags = header[11];
    let expected_crc = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);
    let actual_crc = crc32fast::hash(&header[0..12]);
    if actual_crc != expected_crc {
        return Err(FrameError::HeaderChecksumMismatch {
            expected: expected_crc,
            actual: actual_crc,
        });
    }
    Ok((format_ver, alg, flags, expected_crc))
}

fn record_checksum(seq: u64, payload: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&seq.to_le_bytes());
    hasher.update(payload);
    hasher.finalize()
}

/// Write a canonical 16-byte hot `.pblog` segment header.
pub fn write_segment_header<W: Write>(writer: &mut W) -> std::result::Result<(), FrameError> {
    let header = build_16b_header(HOT_MAGIC, CHECKSUM_ALG_CRC32FAST, 0);
    writer.write_all(&header)?;
    Ok(())
}

/// Verify a 16-byte hot `.pblog` segment header.
pub fn verify_segment_header(header: &[u8]) -> std::result::Result<SegmentHeader, FrameError> {
    let (format_ver, checksum_alg, flags, header_crc32) =
        verify_16b_header(header, HOT_MAGIC, CHECKSUM_ALG_CRC32FAST)?;
    Ok(SegmentHeader {
        format_ver,
        checksum_alg,
        flags,
        header_crc32,
    })
}

/// Encode a single [`LogEvent`] into a hot record frame:
/// `payload_len u32 LE | crc32 u32 LE | seq u64 LE | prost payload`.
pub fn encode_record_frame(event: &LogEvent) -> std::result::Result<Vec<u8>, FrameError> {
    let payload_len = event.encoded_len();
    if payload_len > MAX_RECORD_PAYLOAD_LEN {
        return Err(FrameError::PayloadTooLarge {
            len: payload_len,
            max: MAX_RECORD_PAYLOAD_LEN,
        });
    }

    let mut payload = Vec::with_capacity(payload_len);
    event.encode(&mut payload)?;

    let crc = record_checksum(event.seq, &payload);
    let len_u32 = u32::try_from(payload.len()).map_err(|_| FrameError::PayloadTooLarge {
        len: payload.len(),
        max: MAX_RECORD_PAYLOAD_LEN,
    })?;

    let mut frame = Vec::with_capacity(RECORD_HEADER_LEN + payload.len());
    frame.extend_from_slice(&len_u32.to_le_bytes());
    frame.extend_from_slice(&crc.to_le_bytes());
    frame.extend_from_slice(&event.seq.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

/// Decode all valid record frames from a hot `.pblog` segment byte slice,
/// classifying torn-tail corruption separately from mid-segment corruption.
pub fn decode_segment_frames(
    bytes: &[u8],
) -> std::result::Result<(Vec<(u64, LogEvent)>, TailStatus), FrameError> {
    let _header = verify_segment_header(bytes)?;
    let mut records = Vec::new();
    let mut offset: usize = SEGMENT_HEADER_LEN;
    let mut prev_seq: Option<u64> = None;

    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining < RECORD_HEADER_LEN {
            return Ok((
                records,
                TailStatus::TruncatedTail {
                    valid_end_offset: offset as u64,
                },
            ));
        }

        let payload_len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let expected_crc = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        let seq = u64::from_le_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
            bytes[offset + 12],
            bytes[offset + 13],
            bytes[offset + 14],
            bytes[offset + 15],
        ]);

        if payload_len > MAX_RECORD_PAYLOAD_LEN {
            return Ok((
                records,
                TailStatus::MidSegmentCorrupt {
                    offset: offset as u64,
                    seq,
                },
            ));
        }

        let frame_end = offset
            .checked_add(RECORD_HEADER_LEN)
            .and_then(|v| v.checked_add(payload_len));

        let Some(frame_end) = frame_end else {
            return Ok((
                records,
                TailStatus::MidSegmentCorrupt {
                    offset: offset as u64,
                    seq,
                },
            ));
        };

        if frame_end > bytes.len() {
            return Ok((
                records,
                TailStatus::TruncatedTail {
                    valid_end_offset: offset as u64,
                },
            ));
        }

        let is_tail_frame = frame_end == bytes.len();

        let payload = &bytes[offset + RECORD_HEADER_LEN..frame_end];
        let actual_crc = record_checksum(seq, payload);
        if actual_crc != expected_crc {
            if is_tail_frame {
                return Ok((
                    records,
                    TailStatus::TruncatedTail {
                        valid_end_offset: offset as u64,
                    },
                ));
            }
            return Ok((
                records,
                TailStatus::MidSegmentCorrupt {
                    offset: offset as u64,
                    seq,
                },
            ));
        }

        let event = match LogEvent::decode(payload) {
            Ok(ev) => ev,
            Err(_) if is_tail_frame => {
                return Ok((
                    records,
                    TailStatus::TruncatedTail {
                        valid_end_offset: offset as u64,
                    },
                ));
            }
            Err(_) => {
                return Ok((
                    records,
                    TailStatus::MidSegmentCorrupt {
                        offset: offset as u64,
                        seq,
                    },
                ));
            }
        };

        if event.seq != seq {
            if is_tail_frame {
                return Ok((
                    records,
                    TailStatus::TruncatedTail {
                        valid_end_offset: offset as u64,
                    },
                ));
            }
            return Ok((
                records,
                TailStatus::MidSegmentCorrupt {
                    offset: offset as u64,
                    seq,
                },
            ));
        }

        if let Some(prev) = prev_seq {
            if seq <= prev {
                if is_tail_frame {
                    return Ok((
                        records,
                        TailStatus::TruncatedTail {
                            valid_end_offset: offset as u64,
                        },
                    ));
                }
                return Ok((
                    records,
                    TailStatus::MidSegmentCorrupt {
                        offset: offset as u64,
                        seq,
                    },
                ));
            }
        }

        prev_seq = Some(seq);
        records.push((offset as u64, event));
        offset = frame_end;
    }

    Ok((records, TailStatus::Clean))
}

/// Decode a contiguous slice of raw record frames (without a 16-byte segment header),
/// such as the decompressed body of a cold `.pblog.zst` frame.
pub fn decode_raw_record_frames(
    bytes: &[u8],
) -> std::result::Result<Vec<(u64, LogEvent)>, FrameError> {
    let mut records = Vec::new();
    let mut offset: usize = 0;

    while offset < bytes.len() {
        let remaining = bytes.len() - offset;
        if remaining < RECORD_HEADER_LEN {
            return Err(FrameError::TruncatedFrame {
                expected: RECORD_HEADER_LEN,
                actual: remaining,
            });
        }

        let payload_len = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        if payload_len > MAX_RECORD_PAYLOAD_LEN {
            return Err(FrameError::PayloadTooLarge {
                len: payload_len,
                max: MAX_RECORD_PAYLOAD_LEN,
            });
        }

        let expected_crc = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]);
        let seq = u64::from_le_bytes([
            bytes[offset + 8],
            bytes[offset + 9],
            bytes[offset + 10],
            bytes[offset + 11],
            bytes[offset + 12],
            bytes[offset + 13],
            bytes[offset + 14],
            bytes[offset + 15],
        ]);

        let needed = RECORD_HEADER_LEN + payload_len;
        if remaining < needed {
            return Err(FrameError::TruncatedFrame {
                expected: needed,
                actual: remaining,
            });
        }

        let payload = &bytes[offset + RECORD_HEADER_LEN..offset + needed];
        let actual_crc = record_checksum(seq, payload);
        if actual_crc != expected_crc {
            return Err(FrameError::RecordChecksumMismatch {
                seq,
                expected: expected_crc,
                actual: actual_crc,
            });
        }

        let event = LogEvent::decode(payload)?;
        records.push((offset as u64, event));
        offset += needed;
    }

    Ok(records)
}

/// Write a canonical 16-byte cold `.pblog.zst` archive file header.
pub fn write_archive_header<W: Write>(writer: &mut W) -> std::result::Result<(), FrameError> {
    let header = build_16b_header(ARCHIVE_MAGIC, ARCHIVE_CODEC_ZSTD, 0);
    writer.write_all(&header)?;
    Ok(())
}

/// Verify a 16-byte cold `.pblog.zst` archive file header.
pub fn verify_archive_header(header: &[u8]) -> std::result::Result<ArchiveHeader, FrameError> {
    let (format_ver, codec, flags, header_crc32) =
        verify_16b_header(header, ARCHIVE_MAGIC, ARCHIVE_CODEC_ZSTD)?;
    Ok(ArchiveHeader {
        format_ver,
        codec,
        flags,
        header_crc32,
    })
}

/// Compress a block of raw record frames into one self-contained zstd frame
/// at level 6 with native XXH64 checksum enabled.
pub fn encode_zstd_frame(raw_bytes: &[u8]) -> std::result::Result<Vec<u8>, FrameError> {
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), ARCHIVE_ZSTD_LEVEL)?;
    encoder.include_checksum(true)?;
    encoder.write_all(raw_bytes)?;
    let compressed = encoder.finish()?;
    Ok(compressed)
}

/// Decompress a single zstd frame, verifying its embedded XXH64 checksum.
pub fn decode_zstd_frame(compressed: &[u8]) -> std::result::Result<Vec<u8>, FrameError> {
    let decoded = zstd::stream::decode_all(compressed)?;
    Ok(decoded)
}

/// Encode content-addressed blob bytes:
/// - `< 1 KiB`: raw with `b"LRB0"` prefix.
/// - `>= 1 KiB`: zstd (level 6, XXH64 checksum) with `b"LRBZ"` prefix iff it saves `>= 1/16` (6.25%), else raw with `b"LRB0"`.
pub fn encode_blob_bytes(raw: &[u8]) -> std::result::Result<Vec<u8>, FrameError> {
    if raw.len() >= BLOB_COMPRESS_MIN_BYTES {
        let compressed = encode_zstd_frame(raw)?;
        if compressed.len() + (raw.len() >> BLOB_SAVING_SHIFT) <= raw.len() {
            let mut out = Vec::with_capacity(BLOB_MAGIC_ZSTD.len() + compressed.len());
            out.extend_from_slice(&BLOB_MAGIC_ZSTD);
            out.extend_from_slice(&compressed);
            return Ok(out);
        }
    }

    let mut out = Vec::with_capacity(BLOB_MAGIC_RAW.len() + raw.len());
    out.extend_from_slice(&BLOB_MAGIC_RAW);
    out.extend_from_slice(raw);
    Ok(out)
}

/// Decode content-addressed blob bytes produced by [`encode_blob_bytes`].
pub fn decode_blob_bytes(encoded: &[u8]) -> std::result::Result<Vec<u8>, FrameError> {
    if encoded.len() < 4 {
        return Err(FrameError::InvalidBlobMagic);
    }
    let magic = &encoded[0..4];
    if magic == BLOB_MAGIC_RAW {
        Ok(encoded[4..].to_vec())
    } else if magic == BLOB_MAGIC_ZSTD {
        decode_zstd_frame(&encoded[4..])
    } else {
        Err(FrameError::InvalidBlobMagic)
    }
}

/// Compute lowercase hex SHA-256 digest for content-addressed blobs and verifiers.
pub fn blob_sha256_hex(raw: &[u8]) -> String {
    let digest = Sha256::digest(raw);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}
