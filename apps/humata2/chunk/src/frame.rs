//! The self-describing frame for compressed data at rest (SPEC TR-13, D60):
//! store rows, blob files, snapshots, scrollback, restart and bug reports.
//! Every frame names its codec, the message type it holds and that type's
//! format version, all as numbers, so any reader decodes it without context
//! and the codec can change without a migration.
//!
//! Layout, little-endian, header format 1; no names anywhere:
//!
//! | bytes  | field                                                  |
//! |--------|--------------------------------------------------------|
//! | 0..4   | magic `\x89H2Z`                                        |
//! | 4      | header format (1)                                      |
//! | 5      | codec id ([`Codec::id`]: 0 none, 1 lz4, 2 zstd)        |
//! | 6..8   | message type id, chosen by the writer's crate          |
//! | 8..10  | the message type's format version                      |
//! | 10..18 | decoded length                                         |
//! | 18..22 | CRC-32 (IEEE) of the decoded bytes                     |
//! | 22..   | the encoded bytes                                      |

use crate::chunk::{COMPRESS_MIN, SAVING_SHIFT, ZSTD_LEVEL};
use crate::codec::Codec;
use crate::error::{DecodeFailure, FrameError};

/// The first bytes of every frame. The high first byte keeps text and
/// most other formats from passing for one.
pub const FRAME_MAGIC: [u8; 4] = *b"\x89H2Z";

/// The header format this build writes and reads.
const HEADER_FORMAT: u8 = 1;

/// The header's length; the encoded bytes follow it.
const HEADER_LEN: usize = 22;

/// What a frame's header says about the bytes it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub codec: Codec,
    /// The message type held, as the writer's numeric id for it.
    pub kind: u16,
    /// The message type's format version, which its reader checks.
    pub version: u16,
    /// The decoded length.
    pub raw_len: u64,
    crc32: u32,
}

/// A decoded frame: its header and the bytes it held.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Framed {
    pub header: FrameHeader,
    pub data: Vec<u8>,
}

/// Frames `raw`, a `kind` message at format `version`, compressed with
/// `codec` when that saves at least 1/16 of it, else as it is (the header
/// then says uncompressed).
pub fn encode_framed(codec: Codec, kind: u16, version: u16, raw: &[u8]) -> Result<Vec<u8>, FrameError> {
    let packed = if raw.len() < COMPRESS_MIN {
        None
    } else {
        match codec {
            Codec::None => None,
            Codec::Lz4 => Some(lz4_flex::block::compress(raw)),
            Codec::Zstd => Some(
                zstd::bulk::compress(raw, ZSTD_LEVEL).map_err(|source| FrameError::Compress { kind, source })?,
            ),
        }
    };
    let (codec, body) = match packed {
        Some(packed) if packed.len() + (raw.len() >> SAVING_SHIFT) <= raw.len() => (codec, packed),
        _ => (Codec::None, raw.to_vec()),
    };
    let mut framed = Vec::with_capacity(HEADER_LEN + body.len());
    framed.extend_from_slice(&FRAME_MAGIC);
    framed.push(HEADER_FORMAT);
    framed.push(codec.id());
    framed.extend_from_slice(&kind.to_le_bytes());
    framed.extend_from_slice(&version.to_le_bytes());
    framed.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    framed.extend_from_slice(&crc32fast::hash(raw).to_le_bytes());
    framed.extend_from_slice(&body);
    Ok(framed)
}

/// Reads a frame's header without decoding it. `origin` names the file or
/// stream it came from, for errors.
pub fn peek_framed(framed: &[u8], origin: &str) -> Result<FrameHeader, FrameError> {
    let origin = || origin.to_owned();
    if framed.len() < 4 || framed[..4] != FRAME_MAGIC {
        return Err(FrameError::NotAFrame { origin: origin() });
    }
    let header = framed.get(..HEADER_LEN).ok_or_else(|| FrameError::Truncated { origin: origin() })?;
    if header[4] != HEADER_FORMAT {
        return Err(FrameError::UnknownFormat {
            origin: origin(),
            format: header[4],
        });
    }
    let codec = Codec::from_id(header[5]).ok_or_else(|| FrameError::UnknownCodec {
        origin: origin(),
        id: header[5],
    })?;
    let mut raw_len = [0; 8];
    raw_len.copy_from_slice(&header[10..18]);
    let mut crc32 = [0; 4];
    crc32.copy_from_slice(&header[18..22]);
    Ok(FrameHeader {
        codec,
        kind: u16::from_le_bytes([header[6], header[7]]),
        version: u16::from_le_bytes([header[8], header[9]]),
        raw_len: u64::from_le_bytes(raw_len),
        crc32: u32::from_le_bytes(crc32),
    })
}

/// Decodes a frame. `origin` names the file or stream it came from, for
/// errors; a frame that would decode to more than `max_len` bytes is
/// refused before any memory is set aside for it.
pub fn decode_framed(framed: &[u8], origin: &str, max_len: u64) -> Result<Framed, FrameError> {
    let header = peek_framed(framed, origin)?;
    if header.raw_len > max_len {
        return Err(FrameError::TooLarge {
            origin: origin.to_owned(),
            claimed: header.raw_len,
            max_len,
        });
    }
    let expected = usize::try_from(header.raw_len).map_err(|_| FrameError::TooLarge {
        origin: origin.to_owned(),
        claimed: header.raw_len,
        max_len,
    })?;
    let body = &framed[HEADER_LEN..];
    let corrupt = |source| FrameError::Corrupt {
        origin: origin.to_owned(),
        codec: header.codec,
        source,
    };
    let data = match header.codec {
        Codec::None => body.to_vec(),
        Codec::Lz4 => {
            let mut raw = vec![0; expected];
            let written = lz4_flex::block::decompress_into(body, &mut raw).map_err(|source| corrupt(DecodeFailure::Lz4(source)))?;
            raw.truncate(written);
            raw
        }
        Codec::Zstd => zstd::bulk::decompress(body, expected).map_err(|source| corrupt(DecodeFailure::Zstd(source)))?,
    };
    if data.len() != expected || crc32fast::hash(&data) != header.crc32 {
        return Err(FrameError::Damaged { origin: origin.to_owned() });
    }
    Ok(Framed { header, data })
}
