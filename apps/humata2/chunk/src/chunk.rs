//! Encoding and decoding one chunk.

use crate::codec::Codec;
use crate::error::{ChunkError, DecodeFailure};
use crate::sniff;

/// The most bytes one chunk holds, before encoding (TR-11).
pub const CHUNK_MAX: usize = 1 << 20;

/// Chunks smaller than this go as they are: compressing them saves too
/// little to pay for itself.
pub(crate) const COMPRESS_MIN: usize = 512;

/// A compressed chunk is sent only when it saves at least 1/16 of the
/// bytes (6.25%); otherwise the receiver would pay to decompress for
/// nothing.
pub(crate) const SAVING_SHIFT: u32 = 4;

/// After this many chunks in a row that didn't shrink, the encoder tries
/// only every `RETRY_EVERY`th chunk, so a stream of random data costs one
/// attempt in eight.
const MISSES_BEFORE_BACKOFF: u32 = 4;
const RETRY_EVERY: u32 = 8;

/// The zstd level (D65: chosen by the release bench).
pub(crate) const ZSTD_LEVEL: i32 = 3;

/// What a chunk's header says about its bytes. On the wire these are the
/// `codec`, `raw_size` and `crc32` fields of the message carrying the
/// chunk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub codec: Codec,
    /// The chunk's length once decoded.
    pub raw_len: u32,
    /// CRC-32 (IEEE) of the decoded bytes. Every chunk this crate encodes
    /// has one; a chunk from a peer that predates compression has none,
    /// which only an uncompressed chunk may lack.
    pub crc32: Option<u32>,
}

/// One encoded chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub header: Header,
    pub data: Vec<u8>,
}

/// Encodes a stream's chunks with one codec. It remembers what it has
/// learned about the stream: data whose first bytes say it is compressed
/// already (an image, an archive) is sent as is from then on, and data that
/// keeps not shrinking is tried only now and then.
pub struct Encoder {
    codec: Codec,
    zstd: Option<zstd::bulk::Compressor<'static>>,
    first: bool,
    incompressible: bool,
    misses: u32,
    since_try: u32,
}

impl Encoder {
    /// An encoder for a stream sent with `codec`.
    pub fn new(codec: Codec) -> Self {
        Self {
            codec,
            zstd: None,
            first: true,
            incompressible: false,
            misses: 0,
            since_try: 0,
        }
    }

    /// The codec this encoder compresses with, when a chunk shrinks.
    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Changes the codec from the next chunk on, keeping what was learned
    /// about the stream (a link that slowed down switches to zstd).
    pub fn set_codec(&mut self, codec: Codec) {
        self.codec = codec;
    }

    /// Encodes `raw`, the stream's next bytes, as one chunk: compressed
    /// when that shrinks it enough, else as it is.
    pub fn encode(&mut self, raw: Vec<u8>) -> Result<Chunk, ChunkError> {
        if raw.len() > CHUNK_MAX {
            return Err(ChunkError::TooBig { len: raw.len() });
        }
        if std::mem::take(&mut self.first) && sniff::already_compressed(&raw) {
            self.incompressible = true;
        }
        let raw_len = raw.len() as u32;
        let crc32 = Some(crc32fast::hash(&raw));
        if !self.worth_trying(raw.len()) {
            return Ok(plain(raw, raw_len, crc32));
        }
        let packed = match self.codec {
            Codec::None => return Ok(plain(raw, raw_len, crc32)),
            Codec::Lz4 => lz4_flex::block::compress(&raw),
            Codec::Zstd => self.zstd()?.compress(&raw).map_err(|source| ChunkError::Compress { source })?,
        };
        if packed.len() + (raw.len() >> SAVING_SHIFT) > raw.len() {
            self.misses += 1;
            return Ok(plain(raw, raw_len, crc32));
        }
        self.misses = 0;
        Ok(Chunk {
            header: Header {
                codec: self.codec,
                raw_len,
                crc32,
            },
            data: packed,
        })
    }

    fn worth_trying(&mut self, len: usize) -> bool {
        if self.incompressible || len < COMPRESS_MIN || self.codec == Codec::None {
            return false;
        }
        if self.misses < MISSES_BEFORE_BACKOFF {
            return true;
        }
        self.since_try += 1;
        if self.since_try >= RETRY_EVERY {
            self.since_try = 0;
            return true;
        }
        false
    }

    fn zstd(&mut self) -> Result<&mut zstd::bulk::Compressor<'static>, ChunkError> {
        let compressor = match self.zstd.take() {
            Some(compressor) => compressor,
            None => zstd::bulk::Compressor::new(ZSTD_LEVEL).map_err(|source| ChunkError::ZstdContext {
                doing: "compressor",
                source,
            })?,
        };
        Ok(self.zstd.insert(compressor))
    }
}

fn plain(raw: Vec<u8>, raw_len: u32, crc32: Option<u32>) -> Chunk {
    Chunk {
        header: Header {
            codec: Codec::None,
            raw_len,
            crc32,
        },
        data: raw,
    }
}

/// Decodes chunks, whatever codec each names. Damaged input is an error,
/// never a panic or wrong bytes: lengths are checked against the header and
/// every compressed chunk's checksum against its decoded bytes.
#[derive(Default)]
pub struct Decoder {
    zstd: Option<zstd::bulk::Decompressor<'static>>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The decoded bytes of a chunk with `header` and encoded `data`.
    pub fn decode(&mut self, header: Header, data: Vec<u8>) -> Result<Vec<u8>, ChunkError> {
        let codec = header.codec;
        let expected = header.raw_len;
        if expected as usize > CHUNK_MAX {
            return Err(ChunkError::SizeOutOfRange {
                codec,
                claimed: u64::from(expected),
            });
        }
        let raw = match codec {
            Codec::None => data,
            Codec::Lz4 => {
                let mut raw = vec![0; expected as usize];
                let written = lz4_flex::block::decompress_into(&data, &mut raw).map_err(|source| ChunkError::Corrupt {
                    codec,
                    source: DecodeFailure::Lz4(source),
                })?;
                raw.truncate(written);
                raw
            }
            Codec::Zstd => self
                .zstd()?
                .decompress(&data, expected as usize)
                .map_err(|source| ChunkError::Corrupt {
                    codec,
                    source: DecodeFailure::Zstd(source),
                })?,
        };
        if raw.len() != expected as usize {
            return Err(ChunkError::WrongSize {
                codec,
                expected,
                actual: raw.len(),
            });
        }
        match header.crc32 {
            Some(expected) => {
                let actual = crc32fast::hash(&raw);
                if actual != expected {
                    return Err(ChunkError::ChecksumMismatch {
                        codec,
                        expected,
                        actual,
                    });
                }
            }
            None if codec != Codec::None => return Err(ChunkError::NoChecksum { codec }),
            None => {}
        }
        Ok(raw)
    }

    fn zstd(&mut self) -> Result<&mut zstd::bulk::Decompressor<'static>, ChunkError> {
        let decompressor = match self.zstd.take() {
            Some(decompressor) => decompressor,
            None => zstd::bulk::Decompressor::new().map_err(|source| ChunkError::ZstdContext {
                doing: "decompressor",
                source,
            })?,
        };
        Ok(self.zstd.insert(decompressor))
    }
}
