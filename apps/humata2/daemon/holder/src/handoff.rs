//! What a holder hands to the image it moves onto (LIVEHOLDER, D801): the
//! open descriptors by number, and everything the new image needs to serve
//! as the old one did. Written beside the socket as `<socket>.handoff`.
//!
//! Layout: magic, `u32` version, `u64` body length, body, `u64` checksum of
//! the body. All integers little-endian. Reading fails closed: a version,
//! length or checksum that doesn't hold is an error, never a guess.

use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use crate::saved::{SaveParts, checksum};

/// The handoff versions this build reads and writes, newest last.
pub(crate) const VERSIONS: &[u32] = &[1];

const MAGIC: &[u8; 9] = b"H2HANDOFF";

/// Why a handoff couldn't be read.
#[derive(Debug, thiserror::Error)]
pub enum HandoffError {
    #[error("it doesn't start like a handoff; it isn't one this program wrote. Don't use it")]
    NotAHandoff,

    #[error(
        "it is version {version}, which this build can't read (it reads {}); the build that wrote it and this one are not meant to move onto each other. Move onto a build that reads version {version}",
        VERSIONS.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
    )]
    UnknownVersion { version: u32 },

    #[error("it ends before it should; the write of it was cut short. Don't use it")]
    Truncated,

    #[error("its checksum doesn't match, so it is damaged. Don't use it")]
    Damaged,

    #[error("it can't be used: {reason}. Don't use it")]
    Invalid { reason: &'static str },
}

/// The state of a holder at the moment it moves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Handoff {
    pub(crate) listener: i32,
    pub(crate) master: i32,
    /// The program's pid, 0 if unknown.
    pub(crate) child: u32,
    /// The exit code the clients were told of, if any.
    pub(crate) exited: Option<i32>,
    /// The exit code the program was reaped with, if it was.
    pub(crate) reaped: Option<i32>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) limit: u64,
    pub(crate) end: u64,
    pub(crate) runs: Vec<(u64, Vec<u8>)>,
    pub(crate) save: Option<SaveParts>,
    pub(crate) instance: u64,
    pub(crate) keys: Vec<String>,
    /// How long the program had run, in milliseconds.
    pub(crate) started_ms: u64,
}

struct Writer(Vec<u8>);

impl Writer {
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    fn bytes(&mut self, bytes: &[u8]) {
        self.u64(bytes.len() as u64);
        self.0.extend_from_slice(bytes);
    }

    fn option(&mut self, value: Option<i32>) {
        self.u8(u8::from(value.is_some()));
        self.i32(value.unwrap_or(0));
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], HandoffError> {
        let Some((head, tail)) = self.0.split_at_checked(len) else {
            return Err(HandoffError::Truncated);
        };
        self.0 = tail;
        Ok(head)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], HandoffError> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8, HandoffError> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, HandoffError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, HandoffError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, HandoffError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, HandoffError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn bytes(&mut self) -> Result<&'a [u8], HandoffError> {
        let len = usize::try_from(self.u64()?).map_err(|_| HandoffError::Truncated)?;
        self.take(len)
    }

    fn option(&mut self) -> Result<Option<i32>, HandoffError> {
        let some = self.u8()? != 0;
        let value = self.i32()?;
        Ok(some.then_some(value))
    }
}

impl Handoff {
    /// The handoff as bytes, written as `version`.
    pub(crate) fn encode(&self, version: u32) -> Vec<u8> {
        let mut body = Writer(Vec::new());
        body.i32(self.listener);
        body.i32(self.master);
        body.u32(self.child);
        body.option(self.exited);
        body.option(self.reaped);
        body.u16(self.cols);
        body.u16(self.rows);
        body.u64(self.limit);
        body.u64(self.end);
        body.u64(self.runs.len() as u64);
        for (start, bytes) in &self.runs {
            body.u64(*start);
            body.bytes(bytes);
        }
        match &self.save {
            Some(save) => {
                body.u8(1);
                body.bytes(&save.path.clone().into_os_string().into_vec());
                body.u64(save.len);
                body.u64(save.reserved);
                body.u64(save.ahead);
                body.u8(u8::from(save.failed));
            }
            None => body.u8(0),
        }
        body.u64(self.instance);
        body.u64(self.keys.len() as u64);
        for key in &self.keys {
            body.bytes(key.as_bytes());
        }
        body.u64(self.started_ms);

        let body = body.0;
        let mut out = Vec::with_capacity(MAGIC.len() + 4 + 8 + body.len() + 8);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&version.to_le_bytes());
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&checksum(&body).to_le_bytes());
        out
    }

    /// Reads a handoff, or says why it can't.
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, HandoffError> {
        let mut outer = Reader(bytes);
        if outer.take(MAGIC.len()).map_err(|_| HandoffError::NotAHandoff)? != MAGIC {
            return Err(HandoffError::NotAHandoff);
        }
        let version = outer.u32()?;
        if !VERSIONS.contains(&version) {
            return Err(HandoffError::UnknownVersion { version });
        }
        let len = usize::try_from(outer.u64()?).map_err(|_| HandoffError::Truncated)?;
        let body = outer.take(len)?;
        let check = outer.u64()?;
        if !outer.0.is_empty() {
            return Err(HandoffError::Invalid {
                reason: "it has bytes after its end",
            });
        }
        if check != checksum(body) {
            return Err(HandoffError::Damaged);
        }

        let mut body = Reader(body);
        let listener = body.i32()?;
        let master = body.i32()?;
        let child = body.u32()?;
        let exited = body.option()?;
        let reaped = body.option()?;
        let cols = body.u16()?;
        let rows = body.u16()?;
        let limit = body.u64()?;
        let end = body.u64()?;
        let run_count = body.u64()?;
        let mut runs = Vec::new();
        for _ in 0..run_count {
            let start = body.u64()?;
            runs.push((start, body.bytes()?.to_vec()));
        }
        let save = match body.u8()? {
            0 => None,
            1 => {
                let path = PathBuf::from(OsString::from_vec(body.bytes()?.to_vec()));
                Some(SaveParts {
                    path,
                    len: body.u64()?,
                    reserved: body.u64()?,
                    ahead: body.u64()?,
                    failed: body.u8()? != 0,
                })
            }
            _ => {
                return Err(HandoffError::Invalid {
                    reason: "it says something other than yes or no about a saved log",
                });
            }
        };
        let instance = body.u64()?;
        let key_count = body.u64()?;
        let mut keys = Vec::new();
        for _ in 0..key_count {
            let key = String::from_utf8(body.bytes()?.to_vec()).map_err(|_| HandoffError::Invalid {
                reason: "a key it holds isn't text",
            })?;
            keys.push(key);
        }
        let started_ms = body.u64()?;
        if !body.0.is_empty() {
            return Err(HandoffError::Invalid {
                reason: "its body is longer than its version says",
            });
        }
        if listener < 0 || master < 0 {
            return Err(HandoffError::Invalid {
                reason: "it names a descriptor that doesn't exist",
            });
        }
        Ok(Self {
            listener,
            master,
            child,
            exited,
            reaped,
            cols,
            rows,
            limit,
            end,
            runs,
            save,
            instance,
            keys,
            started_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Handoff {
        Handoff {
            listener: 5,
            master: 7,
            child: 4242,
            exited: Some(3),
            reaped: Some(3),
            cols: 100,
            rows: 30,
            limit: 1 << 20,
            end: 500,
            runs: vec![(0, b"hello".to_vec()), (100, b"world\x1b[0m".to_vec())],
            save: Some(SaveParts {
                path: PathBuf::from("/tmp/x/t.scrollback"),
                len: 321,
                reserved: 1 << 20,
                ahead: 2 << 20,
                failed: false,
            }),
            instance: 0xdead_beef_1234,
            keys: vec!["s/d-1".to_owned(), "s/d-2".to_owned()],
            started_ms: 987,
        }
    }

    #[test]
    fn a_handoff_reads_back_as_written() {
        let handoff = sample();
        assert_eq!(Handoff::decode(&handoff.encode(1)).unwrap(), handoff);
        let bare = Handoff {
            exited: None,
            reaped: None,
            save: None,
            keys: Vec::new(),
            runs: Vec::new(),
            ..sample()
        };
        assert_eq!(Handoff::decode(&bare.encode(1)).unwrap(), bare);
    }

    #[test]
    fn a_changed_byte_is_refused() {
        let bytes = sample().encode(1);
        for at in [MAGIC.len() + 12 + 3, bytes.len() / 2, bytes.len() - 1] {
            let mut damaged = bytes.clone();
            damaged[at] ^= 0x55;
            assert!(Handoff::decode(&damaged).is_err(), "byte {at} changed unnoticed");
        }
    }

    #[test]
    fn a_cut_short_handoff_is_refused() {
        let bytes = sample().encode(1);
        for len in [0, 5, MAGIC.len() + 2, bytes.len() - 9, bytes.len() - 1] {
            assert!(Handoff::decode(&bytes[..len]).is_err(), "cut at {len} was read");
        }
    }

    #[test]
    fn a_version_this_build_does_not_read_is_refused() {
        let bytes = sample().encode(99);
        assert!(matches!(
            Handoff::decode(&bytes),
            Err(HandoffError::UnknownVersion { version: 99 })
        ));
    }

    #[test]
    fn something_else_is_not_a_handoff() {
        assert!(matches!(
            Handoff::decode(b"not a handoff at all"),
            Err(HandoffError::NotAHandoff)
        ));
    }
}
