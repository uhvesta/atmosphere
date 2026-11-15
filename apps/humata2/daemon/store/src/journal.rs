//! Writes kept on a machine until the host has them (SL-1).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::atomic::{create_dir, is_partial, remove_partials, write_atomic};
use crate::store::{read_json, remove_file, unix_ms, write_json};
use crate::StoreError;

const ENTRY: &str = ".entry";
const ACKED: &str = "acked";

/// One write waiting for the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    /// Counts up from 1 and is never reused, even after entries are acked.
    pub seq: u64,
    pub kind: String,
    pub written_unix_ms: i64,
    pub payload: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct Header {
    seq: u64,
    kind: String,
    written_unix_ms: i64,
    len: u64,
}

#[derive(Serialize, Deserialize)]
struct Acked {
    through: u64,
}

/// An append-only list of entries, one file each (`<seq>.entry`: a JSON
/// header line, then the payload), plus `acked`, the highest sequence
/// the host has taken. Entries up to it are deleted; the number stays so
/// sequences never repeat.
pub struct Journal {
    folder: PathBuf,
    next: Mutex<u64>,
}

impl Journal {
    pub(crate) fn open(folder: PathBuf) -> Result<Self, StoreError> {
        create_dir(&folder)?;
        remove_partials(&folder)?;
        let acked = read_acked(&folder)?;
        let newest = entry_seqs(&folder)?.into_iter().max().unwrap_or(0);
        Ok(Self {
            folder,
            next: Mutex::new(acked.max(newest) + 1),
        })
    }

    /// Adds an entry and returns its sequence once it's on disk.
    pub fn append(&self, kind: &str, payload: &[u8]) -> Result<u64, StoreError> {
        let mut next = self.lock();
        let seq = *next;
        let header = Header {
            seq,
            kind: kind.to_owned(),
            written_unix_ms: unix_ms(),
            len: payload.len() as u64,
        };
        let path = self.entry_path(seq);
        let mut bytes = serde_json::to_vec(&header).map_err(|source| StoreError::Write {
            path: path.clone(),
            source: std::io::Error::other(source),
        })?;
        bytes.push(b'\n');
        bytes.extend_from_slice(payload);
        write_atomic(&path, &bytes)?;
        *next = seq + 1;
        Ok(seq)
    }

    /// The entries the host hasn't acked, in the order they were written.
    pub fn entries(&self) -> Result<Vec<JournalEntry>, StoreError> {
        let _writes = self.lock();
        let acked = read_acked(&self.folder)?;
        let mut seqs: Vec<u64> = entry_seqs(&self.folder)?
            .into_iter()
            .filter(|seq| *seq > acked)
            .collect();
        seqs.sort_unstable();
        seqs.into_iter()
            .map(|seq| read_entry(&self.entry_path(seq), seq))
            .collect()
    }

    /// Records that the host has every entry through `seq`, and deletes
    /// them.
    pub fn ack_through(&self, seq: u64) -> Result<(), StoreError> {
        let mut next = self.lock();
        let through = read_acked(&self.folder)?.max(seq);
        write_json(&self.folder.join(ACKED), &Acked { through })?;
        *next = (*next).max(through + 1);
        for old in entry_seqs(&self.folder)? {
            if old <= through {
                remove_file(&self.entry_path(old))?;
            }
        }
        Ok(())
    }

    /// The highest sequence the host has acked, 0 before any.
    pub fn acked(&self) -> Result<u64, StoreError> {
        read_acked(&self.folder)
    }

    fn entry_path(&self, seq: u64) -> PathBuf {
        self.folder.join(format!("{seq:020}{ENTRY}"))
    }

    fn lock(&self) -> MutexGuard<'_, u64> {
        // The counter is a plain number, still right after a panic.
        self.next.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn read_acked(folder: &Path) -> Result<u64, StoreError> {
    Ok(read_json::<Acked>(&folder.join(ACKED))?.map_or(0, |acked| acked.through))
}

fn entry_seqs(folder: &Path) -> Result<Vec<u64>, StoreError> {
    let read_failed = |source| StoreError::Read {
        path: folder.to_owned(),
        source,
    };
    let mut seqs = Vec::new();
    for entry in fs::read_dir(folder).map_err(read_failed)? {
        let name = entry.map_err(read_failed)?.file_name();
        let name = name.to_string_lossy();
        if is_partial(&name) {
            continue;
        }
        if let Some(seq) = name.strip_suffix(ENTRY).and_then(|seq| seq.parse().ok()) {
            seqs.push(seq);
        }
    }
    Ok(seqs)
}

fn read_entry(path: &Path, seq: u64) -> Result<JournalEntry, StoreError> {
    let damaged = |reason| StoreError::JournalEntryDamaged {
        path: path.to_owned(),
        reason,
    };
    let bytes = fs::read(path).map_err(|source| StoreError::Read {
        path: path.to_owned(),
        source,
    })?;
    let Some(split) = bytes.iter().position(|byte| *byte == b'\n') else {
        return Err(damaged("it has no header line"));
    };
    let header: Header =
        serde_json::from_slice(&bytes[..split]).map_err(|_| damaged("its header is unreadable"))?;
    if header.seq != seq {
        return Err(damaged("its header names another sequence"));
    }
    let payload = &bytes[split + 1..];
    if payload.len() as u64 != header.len {
        return Err(damaged("its payload is a different length from its header"));
    }
    Ok(JournalEntry {
        seq,
        kind: header.kind,
        written_unix_ms: header.written_unix_ms,
        payload: payload.to_vec(),
    })
}
