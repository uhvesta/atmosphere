use crate::{Error, error::disk};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Output and size changes in the order seen by this attachment. No input is recorded.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Record {
    Output {
        offset: u64,
        bytes: Vec<u8>,
    },
    Resize {
        cols: u16,
        rows: u16,
    },
    /// Bytes lost outside the holder's replay window. Reset the renderer before continuing.
    Gap {
        from: u64,
        to: u64,
    },
}

pub(crate) struct Journal {
    path: PathBuf,
    file: Option<File>,
    _lock: Option<File>,
}
impl Drop for Journal {
    fn drop(&mut self) {
        // The final Arc owner drops this Journal after all readers/writers
        // release it. A spawned child can still temporarily retain the same
        // kernel file description; release logical ownership before close.
        for file in [self.file.as_ref(), self._lock.as_ref()]
            .into_iter()
            .flatten()
        {
            // SAFETY: each descriptor remains owned until after this Drop body.
            let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
        }
    }
}
fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |sum, b| {
        (sum ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
impl Journal {
    pub(crate) fn open(path: &Path) -> Result<(Self, Vec<Record>, u64), Error> {
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| disk("open", path, e))?;
        // SAFETY: the journal owns this descriptor until its last reader/writer is dropped.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(Error::Attached {
                path: path.into(),
                source: std::io::Error::last_os_error(),
            });
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| disk("read", path, e))?;
        let mut at = 0;
        let mut records = Vec::new();
        let mut offset = 0;
        while at < bytes.len() {
            if bytes.len() - at < 12 {
                break;
            }
            let len =
                u32::from_le_bytes(bytes[at..at + 4].try_into().map_err(|_| Error::Journal {
                    path: path.into(),
                    offset: at as u64,
                })?) as usize;
            if len > 4 << 20 {
                return Err(Error::Journal {
                    path: path.into(),
                    offset: at as u64,
                });
            }
            let end = at + 12 + len;
            if end > bytes.len() {
                break;
            }
            let sum = u64::from_le_bytes(bytes[at + 4..at + 12].try_into().map_err(|_| {
                Error::Journal {
                    path: path.into(),
                    offset: at as u64,
                }
            })?);
            if checksum(&bytes[at + 12..end]) != sum {
                return Err(Error::Journal {
                    path: path.into(),
                    offset: at as u64,
                });
            }
            let record: Record =
                serde_json::from_slice(&bytes[at + 12..end]).map_err(|source| Error::State {
                    path: path.into(),
                    source,
                })?;
            match &record {
                Record::Output {
                    offset: start,
                    bytes,
                } => offset = start + bytes.len() as u64,
                Record::Gap { to, .. } => offset = *to,
                _ => {}
            }
            records.push(record);
            at = end;
        }
        if at < bytes.len() {
            file.set_len(at as u64)
                .map_err(|e| disk("repair the incomplete final record in", path, e))?;
            file.sync_all().map_err(|e| disk("sync", path, e))?;
        }
        Ok((
            Self {
                path: path.into(),
                file: Some(file),
                _lock: None,
            },
            records,
            offset,
        ))
    }
    /// Lock ownership without writing terminal output to disk.
    pub(crate) fn memory_only(path: &Path) -> Result<Self, Error> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)
            .map_err(|e| disk("lock", path, e))?;
        // SAFETY: the lock file is owned until Journal drops.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(Error::Attached {
                path: path.into(),
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(Self {
            path: path.into(),
            file: None,
            _lock: Some(file),
        })
    }
    pub(crate) fn append(&mut self, record: &Record) -> Result<(), Error> {
        let Some(file) = self.file.as_mut() else {
            return Ok(());
        };
        let bytes = serde_json::to_vec(record).map_err(|source| Error::State {
            path: self.path.clone(),
            source,
        })?;
        let mut frame = Vec::with_capacity(bytes.len() + 12);
        frame.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        frame.extend_from_slice(&checksum(&bytes).to_le_bytes());
        frame.extend_from_slice(&bytes);
        file.write_all(&frame)
            .and_then(|_| file.sync_data())
            .map_err(|e| disk("save output in", &self.path, e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropping_logical_attachment_releases_ownership_before_inherited_fd_closes() {
        for memory_only in [true, false] {
            let dir = tempfile::tempdir_in("/tmp").unwrap();
            let path = dir.path().join("owned-lock");
            let acquire = || {
                if memory_only {
                    Journal::memory_only(&path)
                } else {
                    Journal::open(&path).map(|(journal, _, _)| journal)
                }
            };
            let journal = acquire().unwrap();
            // A spawned child temporarily retains this same open file
            // description until exec closes CLOEXEC descriptors.
            let inherited = journal
                ._lock
                .as_ref()
                .or(journal.file.as_ref())
                .unwrap()
                .try_clone()
                .unwrap();
            assert!(matches!(acquire(), Err(Error::Attached { .. })));
            drop(journal);
            let replacement =
                acquire().expect("logical owner dropped even though inherited fd remains alive");
            drop(inherited);
            assert!(
                matches!(acquire(), Err(Error::Attached { .. })),
                "closing an old inherited fd must not unlock the replacement owner"
            );
            drop(replacement);
            assert!(acquire().is_ok());
        }
    }
}
