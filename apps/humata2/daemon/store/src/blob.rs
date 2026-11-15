//! Data the store keeps whole rather than in columns: a small header, then
//! the body, compressed when that pays (TR-13, D44, D60).
//!
//! ```text
//! magic      [u8; 4] "H2BL"
//! codec      u8      humata.v1.ChunkCodec: 0 = as is, 1 = lz4, 2 = zstd
//! type       u16 BE  what the body is: a [`Type`] id
//! version    u16 BE  the body's format version
//! body       [u8]    lz4: the decoded length (u32 LE), then the block
//! ```
//!
//! Every field is a number (D60). The codec numbers are the link's, so a
//! body a link delivered can be stored as it came. The store writes zstd
//! and reads all three. Only unreleased builds wrote blobs without the
//! magic; those are refused as damaged.
//!
//! A blob up to [`INLINE_MAX`] bytes goes in its row. A bigger one goes in
//! `blobs/<sha256>` under the state folder, named by the hash of its bytes,
//! written and fsynced before the row that names it commits: a crash can
//! leave a file no row names, which [`sweep`] removes, but never a row
//! naming a file that isn't there.

use std::collections::HashSet;
use std::fs;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::StoreError;
use crate::atomic::{is_partial, write_atomic};
use crate::fault;

/// Bodies shorter than this are kept as they are: zstd's frame costs more
/// than it saves.
const COMPRESS_MIN: usize = 256;
/// Blobs bigger than this live in a file of their own.
pub(crate) const INLINE_MAX: usize = 64 << 10;
/// No body the store wrote is bigger; a header claiming more is damage.
const BODY_MAX: u64 = 1 << 30;

const MAGIC: [u8; 4] = *b"H2BL";
const PLAIN: u8 = 0;
const LZ4: u8 = 1;
const ZSTD: u8 = 2;

/// What a blob's body is. Each id is used for one type forever: a type
/// that goes away keeps its id, and a new one takes the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Type {
    /// A journal entry's payload, as the writer gave it.
    JournalPayload = 1,
    /// `humata.store.v1.AgentLaunch`.
    AgentLaunch = 2,
}

/// Why a blob's bytes can't be read back.
#[derive(Debug)]
pub(crate) enum Unreadable {
    Damaged(&'static str),
    /// Its header names a codec this build doesn't know.
    UnknownCodec(u8),
}

/// A blob's bytes and where they go.
pub(crate) enum Placed {
    Inline(Vec<u8>),
    /// The hex sha256 of the file's bytes, which is also its name.
    File(String),
}

impl Placed {
    pub(crate) fn inline(&self) -> Option<&[u8]> {
        match self {
            Self::Inline(bytes) => Some(bytes),
            Self::File(_) => None,
        }
    }

    pub(crate) fn file(&self) -> Option<&str> {
        match self {
            Self::Inline(_) => None,
            Self::File(hash) => Some(hash),
        }
    }
}

/// `body` with its header, compressed with zstd when that makes it
/// smaller.
pub(crate) fn encode(kind: Type, version: u16, body: &[u8]) -> Vec<u8> {
    let compressed = (body.len() >= COMPRESS_MIN)
        .then(|| ruzstd::encoding::compress_to_vec(body, ruzstd::encoding::CompressionLevel::Fastest))
        .filter(|smaller| smaller.len() < body.len());
    match &compressed {
        Some(smaller) => framed(ZSTD, kind, version, smaller),
        None => framed(PLAIN, kind, version, body),
    }
}

/// `stored`, already encoded with `codec`, behind the header.
fn framed(codec: u8, kind: Type, version: u16, stored: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(MAGIC.len() + 5 + stored.len());
    bytes.extend_from_slice(&MAGIC);
    bytes.push(codec);
    bytes.extend_from_slice(&(kind as u16).to_be_bytes());
    bytes.extend_from_slice(&version.to_be_bytes());
    bytes.extend_from_slice(stored);
    bytes
}

/// The body of a blob of type `kind`, and its format version.
pub(crate) fn decode(bytes: &[u8], kind: Type) -> Result<(u16, Vec<u8>), Unreadable> {
    let cut_short = Unreadable::Damaged("its header is cut short");
    let Some(rest) = bytes.strip_prefix(&MAGIC) else {
        return Err(Unreadable::Damaged("its header is unrecognised"));
    };
    let [codec, type_high, type_low, version_high, version_low, body @ ..] = rest else {
        return Err(cut_short);
    };
    if u16::from_be_bytes([*type_high, *type_low]) != kind as u16 {
        return Err(Unreadable::Damaged("it holds another kind of record"));
    }
    let (codec, version) = (*codec, u16::from_be_bytes([*version_high, *version_low]));
    let unreadable = Unreadable::Damaged("its compressed body is unreadable");
    let too_big = Unreadable::Damaged("its body is larger than any the store writes");
    let body = match codec {
        PLAIN => body.to_vec(),
        LZ4 => {
            let Some((len, block)) = body.split_first_chunk::<4>() else {
                return Err(cut_short);
            };
            if u64::from(u32::from_le_bytes(*len)) > BODY_MAX {
                return Err(too_big);
            }
            lz4_flex::block::decompress_size_prepended(&[len.as_slice(), block].concat())
                .map_err(|_| unreadable)?
        }
        ZSTD => {
            let decoder =
                ruzstd::decoding::StreamingDecoder::new(body).map_err(|_| unreadable)?;
            let mut plain = Vec::new();
            decoder
                .take(BODY_MAX + 1)
                .read_to_end(&mut plain)
                .map_err(|_| Unreadable::Damaged("its compressed body is unreadable"))?;
            if plain.len() as u64 > BODY_MAX {
                return Err(too_big);
            }
            plain
        }
        other => return Err(Unreadable::UnknownCodec(other)),
    };
    Ok((version, body))
}

/// The error for a blob of `what` at `path` that [`decode`] couldn't read.
fn unreadable(path: PathBuf, what: String, why: Unreadable) -> StoreError {
    match why {
        Unreadable::Damaged(reason) => StoreError::Damaged { path, what, reason },
        Unreadable::UnknownCodec(codec) => StoreError::UnknownCodec { path, what, codec },
    }
}

/// Reads a blob of type `kind` from its row (`inline` or `file`) and
/// decodes it. `database` is where an inline one lives.
pub(crate) fn read(
    blobs: &Path,
    database: &Path,
    inline: Option<Vec<u8>>,
    file: Option<String>,
    kind: Type,
    what: impl Fn() -> String,
) -> Result<(u16, Vec<u8>), StoreError> {
    let path = file.as_ref().map_or_else(|| database.to_owned(), |hash| blobs.join(hash));
    let bytes = load(blobs, inline, file, &what)?;
    decode(&bytes, kind).map_err(|why| unreadable(path, what(), why))
}

/// Puts `encoded` where it belongs: in the row, or, when big, in a file
/// that is on disk before this returns.
pub(crate) fn place(blobs: &Path, encoded: Vec<u8>) -> Result<Placed, StoreError> {
    if encoded.len() <= INLINE_MAX {
        return Ok(Placed::Inline(encoded));
    }
    let hash = hex(&Sha256::digest(&encoded));
    let path = blobs.join(&hash);
    // Same name, same bytes: one already there is this blob.
    if !path.exists() {
        write_atomic(&path, &encoded)?;
    }
    fault::point("store.blob.written");
    Ok(Placed::File(hash))
}

/// A blob's bytes from its row: those inline, or the file it names,
/// checked against its name.
fn load(
    blobs: &Path,
    inline: Option<Vec<u8>>,
    file: Option<String>,
    what: impl Fn() -> String,
) -> Result<Vec<u8>, StoreError> {
    match (inline, file) {
        (Some(bytes), None) => Ok(bytes),
        (None, Some(hash)) => {
            let path = blobs.join(&hash);
            let bytes = fs::read(&path).map_err(|source| {
                if source.kind() == ErrorKind::NotFound {
                    StoreError::Damaged {
                        path: path.clone(),
                        what: what(),
                        reason: "the file holding it is missing",
                    }
                } else {
                    StoreError::Read {
                        path: path.clone(),
                        source,
                    }
                }
            })?;
            if hex(&Sha256::digest(&bytes)) != hash {
                return Err(StoreError::Damaged {
                    path,
                    what: what(),
                    reason: "the file holding it no longer matches its hash",
                });
            }
            Ok(bytes)
        }
        _ => Err(StoreError::Damaged {
            path: blobs.to_owned(),
            what: what(),
            reason: "its row names neither bytes nor a file",
        }),
    }
}

/// Removes the files in `blobs` that no row names, and writes a crash cut
/// short. Returns how many went.
pub(crate) fn sweep(blobs: &Path, referenced: &HashSet<String>) -> Result<usize, StoreError> {
    let read_failed = |source| StoreError::Read {
        path: blobs.to_owned(),
        source,
    };
    let mut removed = 0;
    for entry in fs::read_dir(blobs).map_err(read_failed)? {
        let entry = entry.map_err(read_failed)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if referenced.contains(&name) {
            continue;
        }
        if !is_partial(&name) && !is_hash(&name) {
            // Not the store's: left alone.
            continue;
        }
        let path = entry.path();
        match humata2_durable::remove_file(&path) {
            Ok(()) => removed += 1,
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(StoreError::Remove { path, source }),
        }
    }
    Ok(removed)
}

fn is_hash(name: &str) -> bool {
    name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{LZ4, MAGIC, Type, Unreadable, decode, encode, framed, read};
    use crate::StoreError;

    fn sample() -> Vec<u8> {
        b"the same words again and again, ".repeat(64)
    }

    #[test]
    fn what_is_encoded_decodes_to_the_same_body_and_version() {
        for body in [Vec::new(), b"short".to_vec(), sample()] {
            let encoded = encode(Type::AgentLaunch, 7, &body);
            assert_eq!(decode(&encoded, Type::AgentLaunch).unwrap(), (7, body));
        }
        assert!(encode(Type::AgentLaunch, 1, &sample()).len() < sample().len());
    }

    #[test]
    fn an_lz4_body_from_a_link_decodes() {
        let packed = lz4_flex::block::compress_prepend_size(&sample());
        let stored = framed(LZ4, Type::JournalPayload, 1, &packed);
        assert_eq!(decode(&stored, Type::JournalPayload).unwrap(), (1, sample()));
    }

    #[test]
    fn another_type_or_a_cut_header_is_damage() {
        let encoded = encode(Type::AgentLaunch, 1, b"body");
        assert!(matches!(decode(&encoded, Type::JournalPayload), Err(Unreadable::Damaged(_))));
        assert!(matches!(decode(&encoded[..6], Type::AgentLaunch), Err(Unreadable::Damaged(_))));
        assert!(matches!(decode(b"\x02nope", Type::AgentLaunch), Err(Unreadable::Damaged(_))));
    }

    #[test]
    fn an_unknown_codec_is_refused_naming_the_codec_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut stored = MAGIC.to_vec();
        stored.push(9);
        stored.extend_from_slice(&(Type::JournalPayload as u16).to_be_bytes());
        stored.extend_from_slice(&1u16.to_be_bytes());
        stored.extend_from_slice(b"whatever this is");
        let hash = super::hex(&<sha2::Sha256 as sha2::Digest>::digest(&stored));
        std::fs::write(dir.path().join(&hash), &stored).unwrap();

        let found = read(
            dir.path(),
            &dir.path().join("state.sqlite"),
            None,
            Some(hash.clone()),
            Type::JournalPayload,
            || "journal entry 1".to_owned(),
        );
        match found {
            Err(StoreError::UnknownCodec { path, codec, .. }) => {
                assert_eq!((path, codec), (dir.path().join(hash), 9));
            }
            other => panic!("not refused for its codec: {other:?}"),
        }
    }
}
