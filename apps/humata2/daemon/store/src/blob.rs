//! Data the store keeps whole rather than in columns: a frame
//! ([`humata2_chunk::encode_framed`], D65) naming the codec, the body's
//! [`Type`] and its format version by number, then the body, zstd when
//! that pays (TR-13, D60). It is the same frame every file Humata keeps at
//! rest uses, so one reader decodes them all. The frame's checksum of the
//! decoded body catches damage a codec wouldn't.
//!
//! A blob up to [`INLINE_MAX`] bytes goes in its row. A bigger one goes in
//! `blobs/<sha256>` under the state folder, named by the hash of its bytes,
//! written and fsynced before the row that names it commits: a crash can
//! leave a file no row names, which [`sweep`] removes, but never a row
//! naming a file that isn't there.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use humata2_chunk::{Codec, FrameError, decode_framed, encode_framed};
use sha2::{Digest, Sha256};

use crate::StoreError;
use crate::atomic::{is_partial, write_atomic};
use crate::fault;

/// Blobs bigger than this live in a file of their own.
pub(crate) const INLINE_MAX: usize = 64 << 10;
/// No body the store wrote is bigger; a header claiming more is damage.
const BODY_MAX: u64 = 1 << 30;

/// What a blob's body is. Each id is used for one type forever: a type
/// that goes away keeps its id, and a new one takes the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Type {
    /// A journal entry's payload, as the writer gave it.
    JournalPayload = 1,
    /// `humata.store.v1.AgentLaunch`.
    AgentLaunch = 2,
    /// `humata.store.v1.ReviewState`.
    ReviewState = 3,
}

/// Why a blob's bytes can't be read back.
#[derive(Debug)]
pub(crate) enum Unreadable {
    /// The frame itself is damaged, cut short or from a newer Humata.
    Frame(FrameError),
    /// A sound frame holding another type than the row expects.
    OtherType,
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

impl Type {
    fn name(self) -> &'static str {
        match self {
            Self::JournalPayload => "a journal entry",
            Self::AgentLaunch => "an agent's launch record",
            Self::ReviewState => "a session's review",
        }
    }
}

/// `body` in its frame, compressed with zstd when that makes it smaller.
pub(crate) fn encode(kind: Type, version: u16, body: &[u8]) -> Result<Vec<u8>, StoreError> {
    encode_framed(Codec::Zstd, kind as u16, version, body).map_err(|source| StoreError::Encode {
        what: kind.name(),
        source,
    })
}

/// The body of a blob of type `kind`, and its format version. `origin`
/// names where it was read from.
pub(crate) fn decode(bytes: &[u8], kind: Type, origin: &str) -> Result<(u16, Vec<u8>), Unreadable> {
    let framed = decode_framed(bytes, origin, BODY_MAX).map_err(Unreadable::Frame)?;
    if framed.header.kind != kind as u16 {
        return Err(Unreadable::OtherType);
    }
    Ok((framed.header.version, framed.data))
}

/// The error for a blob of `what` at `path` that [`decode`] couldn't read.
fn unreadable(path: PathBuf, what: String, why: Unreadable) -> StoreError {
    match why {
        Unreadable::OtherType => StoreError::Damaged {
            path,
            what,
            reason: "it holds another kind of record",
        },
        Unreadable::Frame(source @ (FrameError::UnknownFormat { .. } | FrameError::UnknownCodec { .. })) => {
            StoreError::NewerFormat { path, what, source }
        }
        Unreadable::Frame(source) => StoreError::Undecodable { path, what, source },
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
    let origin = format!("{} in {}", what(), path.display());
    decode(&bytes, kind, &origin).map_err(|why| unreadable(path, what(), why))
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
    use super::{Type, Unreadable, decode, encode, read};
    use crate::StoreError;

    fn sample() -> Vec<u8> {
        b"the same words again and again, ".repeat(64)
    }

    #[test]
    fn what_is_encoded_decodes_to_the_same_body_and_version() {
        for body in [Vec::new(), b"short".to_vec(), sample()] {
            let encoded = encode(Type::AgentLaunch, 7, &body).unwrap();
            assert_eq!(decode(&encoded, Type::AgentLaunch, "test").unwrap(), (7, body));
        }
        assert!(encode(Type::AgentLaunch, 1, &sample()).unwrap().len() < sample().len());
    }

    #[test]
    fn another_type_a_cut_header_or_a_changed_body_is_damage() {
        let encoded = encode(Type::AgentLaunch, 1, &sample()).unwrap();
        assert!(matches!(decode(&encoded, Type::JournalPayload, "test"), Err(Unreadable::OtherType)));
        assert!(matches!(decode(&encoded[..6], Type::AgentLaunch, "test"), Err(Unreadable::Frame(_))));
        assert!(matches!(decode(b"\x02nope", Type::AgentLaunch, "test"), Err(Unreadable::Frame(_))));
        let mut plain = encode(Type::AgentLaunch, 1, b"body").unwrap();
        let last = plain.len() - 1;
        plain[last] ^= 1;
        assert!(matches!(decode(&plain, Type::AgentLaunch, "test"), Err(Unreadable::Frame(_))));
    }

    #[test]
    fn a_blob_from_a_newer_humata_is_refused_as_newer_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut stored = encode(Type::JournalPayload, 1, b"whatever this is").unwrap();
        // The codec id: one this build doesn't know.
        stored[5] = 9;
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
            Err(StoreError::NewerFormat { path, .. }) => assert_eq!(path, dir.path().join(hash)),
            other => panic!("not refused as newer: {other:?}"),
        }
    }
}
