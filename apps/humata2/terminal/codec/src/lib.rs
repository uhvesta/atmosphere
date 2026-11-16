//! Bounded compression independent of terminal engines, transports and disks.
mod builtin;
mod error;
#[cfg(test)]
mod tests;

pub use error::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const LZ4: &str = "lz4-block-v1";
pub const ZLIB: &str = "zlib-v1";
const VERSION: u8 = 1;
const MAGIC: &[u8; 8] = b"TCDC\0\0\0\x01";
const HEADER: usize = 34;

/// Trusted caller policy; untrusted envelope claims never set these limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub max_decoded_bytes: usize,
    pub max_encoded_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_decoded_bytes: 16 << 20,
            max_encoded_bytes: 4 << 20,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Progress {
    pub consumed: usize,
    pub written: usize,
}

/// Codecs write into caller-bounded slices; they never choose the decoded
/// allocation. A registered implementation is trusted caller code.
pub trait Codec: Send + Sync {
    fn id(&self) -> &str;
    fn max_encoded_len(&self, raw_len: usize) -> Result<usize, Error>;
    fn encode_into(&self, raw: &[u8], output: &mut [u8]) -> Result<usize, Error>;
    fn decode_into(&self, encoded: &[u8], output: &mut [u8]) -> Result<Progress, Error>;
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u8,
    pub codec: String,
    pub decoded_bytes: usize,
    pub encoded_bytes: usize,
    pub raw_crc32: u32,
    pub encoded_crc32: u32,
    pub data: Vec<u8>,
}

pub struct Registry {
    codecs: BTreeMap<String, Box<dyn Codec>>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            codecs: builtin::codecs()
                .into_iter()
                .map(|codec| (codec.id().to_owned(), codec))
                .collect(),
        }
    }
}
impl Registry {
    pub fn register(&mut self, codec: Box<dyn Codec>) -> Result<(), Error> {
        validate_id(codec.id())?;
        let id = codec.id().to_owned();
        if self.codecs.contains_key(&id) {
            return Err(Error::DuplicateCodec { codec: id });
        }
        self.codecs.insert(id, codec);
        Ok(())
    }
    fn codec(&self, id: &str) -> Result<&dyn Codec, Error> {
        validate_id(id)?;
        self.codecs
            .get(id)
            .map(AsRef::as_ref)
            .ok_or_else(|| Error::UnknownCodec {
                codec: id.to_owned(),
            })
    }
    pub fn encode(&self, id: &str, raw: &[u8], limits: Limits) -> Result<Envelope, Error> {
        let (output, written) = self.encode_block(id, raw, limits)?;
        check_limit("encoded", written, limits.max_encoded_bytes)?;
        finish_envelope(id, raw, &output[..written])
    }
    /// Try the explicitly configured codec order. Only an actual encoded
    /// budget overflow selects the next codec; implementation errors never do.
    pub fn encode_preferred(
        &self,
        ids: &[&str],
        raw: &[u8],
        limits: Limits,
    ) -> Result<Envelope, Error> {
        let mut last_budget = None;
        for id in ids {
            // This propagates all plugin/unknown-codec/working-buffer errors,
            // even if a plugin returns a similarly named Limit variant.
            let (output, written) = self.encode_block(id, raw, limits)?;
            if written > limits.max_encoded_bytes {
                last_budget = Some(Error::Limit {
                    what: "encoded",
                    actual: written as u64,
                    limit: limits.max_encoded_bytes as u64,
                });
                continue;
            }
            return finish_envelope(id, raw, &output[..written]);
        }
        Err(last_budget.unwrap_or(Error::NoPreference))
    }
    fn encode_block(
        &self,
        id: &str,
        raw: &[u8],
        limits: Limits,
    ) -> Result<(Vec<u8>, usize), Error> {
        check_limit("decoded", raw.len(), limits.max_decoded_bytes)?;
        let codec = self.codec(id)?;
        let capacity = codec.max_encoded_len(raw.len())?;
        // LZ4 requires its worst-case capacity even when the actual block is
        // much smaller. Bound this scratch independently of transport policy.
        let work_limit = raw
            .len()
            .checked_mul(2)
            .and_then(|n| n.checked_add(1024))
            .ok_or(Error::SizeOverflow)?
            .max(limits.max_encoded_bytes);
        check_limit("encoder working buffer", capacity, work_limit)?;
        let mut output = allocate(capacity)?;
        let written = codec.encode_into(raw, &mut output)?;
        if written > output.len() {
            return Err(Error::Progress {
                what: "encoded output",
                expected: output.len(),
                actual: written,
            });
        }
        Ok((output, written))
    }
    pub fn decode(&self, envelope: &Envelope, limits: Limits) -> Result<Vec<u8>, Error> {
        envelope.validate(limits)?;
        let codec = self.codec(&envelope.codec)?;
        let mut output = allocate(
            envelope
                .decoded_bytes
                .checked_add(1)
                .ok_or(Error::SizeOverflow)?,
        )?;
        let progress = codec.decode_into(&envelope.data, &mut output)?;
        exact(progress, &envelope.data, envelope.decoded_bytes)?;
        output.truncate(progress.written);
        let actual = crc32(&output);
        if actual != envelope.raw_crc32 {
            return Err(Error::Checksum {
                what: "decoded",
                expected: envelope.raw_crc32,
                actual,
            });
        }
        Ok(output)
    }
    /// For historical Zlib payloads whose format already carries a raw length.
    /// Archives without that length retain their own bounded legacy decoder.
    pub fn decode_legacy_zlib(
        &self,
        encoded: &[u8],
        decoded_bytes: usize,
        limits: Limits,
    ) -> Result<Vec<u8>, Error> {
        check_limit("encoded", encoded.len(), limits.max_encoded_bytes)?;
        check_limit("decoded", decoded_bytes, limits.max_decoded_bytes)?;
        let mut output = allocate(decoded_bytes.checked_add(1).ok_or(Error::SizeOverflow)?)?;
        let progress = self.codec(ZLIB)?.decode_into(encoded, &mut output)?;
        exact(progress, encoded, decoded_bytes)?;
        output.truncate(progress.written);
        Ok(output)
    }
}

fn finish_envelope(id: &str, raw: &[u8], encoded: &[u8]) -> Result<Envelope, Error> {
    // Compact the result so a small block does not retain its worst-case
    // scratch allocation in a slow viewer's queue.
    let data = copy_bytes(encoded)?;
    Ok(Envelope {
        version: VERSION,
        codec: id.to_owned(),
        decoded_bytes: raw.len(),
        encoded_bytes: data.len(),
        raw_crc32: crc32(raw),
        encoded_crc32: crc32(&data),
        data,
    })
}

impl Envelope {
    fn validate(&self, limits: Limits) -> Result<(), Error> {
        if self.version != VERSION {
            return Err(Error::Version {
                actual: self.version,
            });
        }
        validate_id(&self.codec)?;
        check_limit("decoded", self.decoded_bytes, limits.max_decoded_bytes)?;
        check_limit("encoded", self.encoded_bytes, limits.max_encoded_bytes)?;
        if self.encoded_bytes != self.data.len() {
            return Err(Error::Progress {
                what: "encoded length",
                expected: self.encoded_bytes,
                actual: self.data.len(),
            });
        }
        let actual = crc32(&self.data);
        if actual != self.encoded_crc32 {
            return Err(Error::Checksum {
                what: "encoded",
                expected: self.encoded_crc32,
                actual,
            });
        }
        Ok(())
    }
    /// Compact little-endian disk carrier, independent of a caller's archive
    /// magic. CRCs detect corruption; authentication belongs to the transport.
    pub fn to_bytes(&self, limits: Limits) -> Result<Vec<u8>, Error> {
        self.validate(limits)?;
        let size = HEADER
            .checked_add(self.codec.len())
            .and_then(|n| n.checked_add(self.data.len()))
            .ok_or(Error::SizeOverflow)?;
        let mut bytes = allocate(size)?;
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..10].copy_from_slice(&(self.codec.len() as u16).to_le_bytes());
        bytes[10..18].copy_from_slice(&(self.decoded_bytes as u64).to_le_bytes());
        bytes[18..26].copy_from_slice(&(self.encoded_bytes as u64).to_le_bytes());
        bytes[26..30].copy_from_slice(&self.raw_crc32.to_le_bytes());
        bytes[30..34].copy_from_slice(&self.encoded_crc32.to_le_bytes());
        bytes[HEADER..HEADER + self.codec.len()].copy_from_slice(self.codec.as_bytes());
        bytes[HEADER + self.codec.len()..].copy_from_slice(&self.data);
        Ok(bytes)
    }
    pub fn from_bytes(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
        if bytes.len() < HEADER || &bytes[..8] != MAGIC {
            return Err(Error::Header {
                reason: "unknown or truncated binary envelope",
            });
        }
        let id_len = u16::from_le_bytes([bytes[8], bytes[9]]) as usize;
        if !(1..=64).contains(&id_len) {
            return Err(Error::CodecId);
        }
        let decoded = u64::from_le_bytes(bytes[10..18].try_into().map_err(|_| Error::Header {
            reason: "truncated decoded length",
        })?);
        let encoded = u64::from_le_bytes(bytes[18..26].try_into().map_err(|_| Error::Header {
            reason: "truncated encoded length",
        })?);
        check_limit_u64("decoded", decoded, limits.max_decoded_bytes)?;
        check_limit_u64("encoded", encoded, limits.max_encoded_bytes)?;
        let prefix = HEADER + id_len;
        let expected = prefix
            .checked_add(encoded as usize)
            .ok_or(Error::SizeOverflow)?;
        if bytes.len() != expected {
            return Err(Error::Progress {
                what: "binary envelope length",
                expected,
                actual: bytes.len(),
            });
        }
        let id = std::str::from_utf8(&bytes[HEADER..prefix])
            .map_err(|source| Error::CodecUtf8 { source })?;
        validate_id(id)?;
        let raw_crc32 =
            u32::from_le_bytes(bytes[26..30].try_into().map_err(|_| Error::Header {
                reason: "truncated decoded checksum",
            })?);
        let encoded_crc32 =
            u32::from_le_bytes(bytes[30..34].try_into().map_err(|_| Error::Header {
                reason: "truncated encoded checksum",
            })?);
        let actual = crc32(&bytes[prefix..]);
        if actual != encoded_crc32 {
            return Err(Error::Checksum {
                what: "encoded",
                expected: encoded_crc32,
                actual,
            });
        }
        Ok(Self {
            version: VERSION,
            codec: id.to_owned(),
            decoded_bytes: decoded as usize,
            encoded_bytes: encoded as usize,
            raw_crc32,
            encoded_crc32,
            data: copy_bytes(&bytes[prefix..])?,
        })
    }
}

fn exact(progress: Progress, input: &[u8], expected: usize) -> Result<(), Error> {
    if progress.consumed != input.len() {
        return Err(Error::Progress {
            what: "consumed input",
            expected: input.len(),
            actual: progress.consumed,
        });
    }
    if progress.written != expected {
        return Err(Error::Progress {
            what: "decoded output",
            expected,
            actual: progress.written,
        });
    }
    Ok(())
}
fn validate_id(id: &str) -> Result<(), Error> {
    if !(1..=64).contains(&id.len())
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(Error::CodecId);
    }
    Ok(())
}
fn allocate(bytes: usize) -> Result<Vec<u8>, Error> {
    let mut output = Vec::new();
    output
        .try_reserve_exact(bytes)
        .map_err(|source| Error::Allocation { bytes, source })?;
    output.resize(bytes, 0);
    Ok(output)
}
fn copy_bytes(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    let mut output = allocate(bytes.len())?;
    output.copy_from_slice(bytes);
    Ok(output)
}
fn check_limit(what: &'static str, actual: usize, limit: usize) -> Result<(), Error> {
    check_limit_u64(what, actual as u64, limit)
}
fn check_limit_u64(what: &'static str, actual: u64, limit: usize) -> Result<(), Error> {
    if actual > limit as u64 {
        return Err(Error::Limit {
            what,
            actual,
            limit: limit as u64,
        });
    }
    Ok(())
}
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}
