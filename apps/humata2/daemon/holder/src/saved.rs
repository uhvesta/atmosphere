//! The holder's output saved on disk, so a holder started on the same
//! socket after this one was killed, or after a reboot, starts with it
//! (PER-1, PER-12).
//!
//! The file is an append log of records, each with a checksum: output at
//! an offset, or a reservation of offsets up to some point. Output is
//! written before any client is sent it and synced every half second. A
//! reservation is synced before output past it is sent, so no client ever
//! holds an offset past the last synced one, and a holder that takes over
//! starts its program's output there: a client's old cursor then reads as
//! trimmed rather than as the new program's bytes.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use humata2_durable::{File, rename, sync_dir};

use crate::HolderError;
use crate::protocol::CHUNK;
use crate::scrollback::Scrollback;

const MAGIC: &[u8; 8] = b"H2SAVED\x01";
const OUTPUT: u8 = 1;
const RESERVE: u8 = 2;
/// Kind, offset and length come before a record's data; its checksum
/// after.
const HEAD: usize = 1 + 8 + 4;
const CHECK: usize = 8;
/// How far past the output sent a reservation reaches.
const RESERVE_AHEAD: u64 = 1 << 20;
/// How often output written since the last sync is synced.
pub(crate) const SYNC_EVERY: Duration = Duration::from_millis(500);

/// Where a holder on `socket` saves its output.
pub fn saved_scrollback_path(socket: &Path) -> PathBuf {
    socket.with_extension("scrollback")
}

/// What a holder that was killed, or lost to a reboot, left on disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SavedOutput {
    /// Output before this offset was saved.
    pub saved_through: u64,
    /// The offset a holder taking over starts its program's output at.
    /// Between the two, output may have been sent that wasn't saved.
    pub resumes_at: u64,
    /// The end of the file was unreadable and was dropped: a power cut
    /// caught it mid-write.
    pub tail_damaged: bool,
}

/// Reads what a holder saved at `path`, or `None` if it saved nothing
/// there or was shut down.
pub fn read_saved(path: &Path) -> Result<Option<SavedOutput>, HolderError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(parse(&bytes, 1).summary())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HolderError::Saved {
            path: path.to_owned(),
            source,
        }),
    }
}

/// The last `max_bytes` of output a holder saved at `path`, oldest first,
/// or `None` if it saved nothing there or was shut down. Output a holder
/// that took over printed after a gap follows with nothing between.
pub fn read_saved_tail(path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>, HolderError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(tail(&bytes, max_bytes))),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HolderError::Saved {
            path: path.to_owned(),
            source,
        }),
    }
}

/// The last `max_bytes` of the output in the saved log `bytes`.
fn tail(bytes: &[u8], max_bytes: usize) -> Vec<u8> {
    parse(bytes, max_bytes)
        .kept
        .since(0)
        .into_iter()
        .flat_map(|(_, data)| data)
        .collect()
}

struct Parsed {
    kept: Scrollback,
    reserved: u64,
    tail_damaged: bool,
}

impl Parsed {
    fn summary(&self) -> SavedOutput {
        SavedOutput {
            saved_through: self.kept.end(),
            resumes_at: self.kept.end().max(self.reserved),
            tail_damaged: self.tail_damaged,
        }
    }
}

fn parse(bytes: &[u8], limit: usize) -> Parsed {
    let mut parsed = Parsed {
        kept: Scrollback::new(limit),
        reserved: 0,
        tail_damaged: false,
    };
    let Some(mut rest) = bytes.strip_prefix(MAGIC.as_slice()) else {
        parsed.tail_damaged = !bytes.is_empty();
        return parsed;
    };
    while let Some((kind, offset, data, after)) = record(rest) {
        match kind {
            // Offsets only ever grow; one that doesn't is damage.
            OUTPUT if offset >= parsed.kept.end() => {
                parsed.kept.skip_to(offset);
                parsed.kept.push(data);
            }
            RESERVE => parsed.reserved = parsed.reserved.max(offset),
            _ => break,
        }
        rest = after;
    }
    parsed.tail_damaged = !rest.is_empty();
    parsed
}

/// The record at the start of `bytes` if it's whole and its checksum
/// holds: its kind, offset, data and what follows it.
fn record(bytes: &[u8]) -> Option<(u8, u64, &[u8], &[u8])> {
    let head = bytes.get(..HEAD)?;
    let kind = head[0];
    let offset = u64::from_le_bytes(head[1..9].try_into().ok()?);
    let len = usize::try_from(u32::from_le_bytes(head[9..13].try_into().ok()?)).ok()?;
    let body = bytes.get(..HEAD + len)?;
    let check = bytes.get(HEAD + len..HEAD + len + CHECK)?;
    if u64::from_le_bytes(check.try_into().ok()?) != checksum(body) {
        return None;
    }
    Some((kind, offset, &body[HEAD..], &bytes[HEAD + len + CHECK..]))
}

fn put_record(out: &mut Vec<u8>, kind: u8, offset: u64, data: &[u8]) {
    let start = out.len();
    out.push(kind);
    out.extend_from_slice(&offset.to_le_bytes());
    // Callers write at most CHUNK bytes a record.
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    let check = checksum(&out[start..]);
    out.extend_from_slice(&check.to_le_bytes());
}

/// FNV-1a: enough to tell a record cut short or never written from a
/// whole one.
fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The open log a running holder appends its output to.
pub(crate) struct SaveLog {
    path: PathBuf,
    file: File,
    len: u64,
    reserved: u64,
    unsynced: bool,
    failed: bool,
}

impl SaveLog {
    /// Opens the log at `path`, taking what an earlier holder saved there,
    /// and returns it with that output kept, moved on to where this
    /// holder's program starts.
    pub(crate) fn open(path: &Path, limit: usize) -> Result<(Self, Scrollback), HolderError> {
        let failed = |source| HolderError::Saved {
            path: path.to_owned(),
            source,
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(failed(error)),
        };
        let parsed = parse(&bytes, limit);
        let mut kept = parsed.kept;
        kept.skip_to(parsed.reserved);
        // Written again whole, so a damaged tail doesn't stay to hide what
        // comes after it.
        let (file, len) = rewrite(path, &kept).map_err(failed)?;
        let log = Self {
            path: path.to_owned(),
            file,
            len,
            reserved: kept.end(),
            unsynced: false,
            failed: false,
        };
        Ok((log, kept))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Saves `data`, printed at `offset` and already pushed to `kept`.
    /// Called before any client is sent it. If saving fails it says so
    /// once and stops; the terminal goes on without it.
    pub(crate) fn output(&mut self, offset: u64, data: &[u8], kept: &Scrollback) {
        if self.failed {
            return;
        }
        if let Err(error) = self.save(offset, data, kept) {
            self.failed = true;
            eprintln!(
                "humata2_holder: stopped saving this terminal's output to {}, so a restart will lose what it prints from here: {error}. Free some disk space; the terminal itself goes on",
                self.path.display()
            );
        }
    }

    fn save(&mut self, offset: u64, data: &[u8], kept: &Scrollback) -> std::io::Result<()> {
        let end = offset + data.len() as u64;
        // Twice what's kept: rewriting it then costs no more than the
        // output that came since.
        let grown = self.len > 2 * kept.limit() as u64 + RESERVE_AHEAD;
        if grown {
            (self.file, self.len) = rewrite(&self.path, kept)?;
            self.reserved = kept.end();
            self.unsynced = false;
            return Ok(());
        }
        let mut out = Vec::with_capacity(data.len() + 2 * (HEAD + CHECK));
        let mut at = offset;
        for chunk in data.chunks(CHUNK) {
            put_record(&mut out, OUTPUT, at, chunk);
            at += chunk.len() as u64;
        }
        let reserving = end > self.reserved;
        if reserving {
            self.reserved = end + RESERVE_AHEAD;
            put_record(&mut out, RESERVE, self.reserved, &[]);
        }
        self.file.append(&out)?;
        self.len += out.len() as u64;
        if reserving {
            self.file.sync_data()?;
            self.unsynced = false;
        } else {
            self.unsynced = true;
        }
        Ok(())
    }

    /// The file to sync, if anything was written since the last sync.
    pub(crate) fn take_unsynced(&mut self) -> Option<File> {
        if !std::mem::take(&mut self.unsynced) || self.failed {
            return None;
        }
        self.file.try_clone().ok()
    }
}

/// Writes `kept` to `path` whole, through a file renamed into place, and
/// returns the file opened to append to and its length.
fn rewrite(path: &Path, kept: &Scrollback) -> std::io::Result<(File, u64)> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    for (start, data) in kept.since(0) {
        let mut at = start;
        for chunk in data.chunks(CHUNK) {
            put_record(&mut out, OUTPUT, at, chunk);
            at += chunk.len() as u64;
        }
    }
    put_record(&mut out, RESERVE, kept.end(), &[]);

    let partial = path.with_extension("scrollback-partial");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(0o600);
    let mut file = File::open(&options, &partial)?;
    file.append(&out)?;
    file.sync_all()?;
    drop(file);
    rename(&partial, path)?;
    if let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        sync_dir(folder)?;
    }
    let file = File::open(OpenOptions::new().read(true).write(true), path)?;
    Ok((file, out.len() as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(records: &[(u8, u64, &[u8])]) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        for (kind, offset, data) in records {
            put_record(&mut out, *kind, *offset, data);
        }
        out
    }

    #[test]
    fn the_tail_is_the_newest_output() {
        let saved = log(&[(OUTPUT, 0, b"hello "), (OUTPUT, 6, b"world"), (RESERVE, 1 << 20, b"")]);
        assert_eq!(tail(&saved, 1024), b"hello world");
        assert_eq!(tail(&saved, 5), b"world");
    }

    #[test]
    fn output_after_a_takeover_follows_the_old() {
        let saved = log(&[(OUTPUT, 0, b"old"), (RESERVE, 100, b""), (OUTPUT, 100, b"new")]);
        assert_eq!(tail(&saved, 1024), b"oldnew");
    }

    #[test]
    fn a_damaged_end_keeps_what_came_before_it() {
        let mut saved = log(&[(OUTPUT, 0, b"kept"), (OUTPUT, 4, b"cut")]);
        saved.truncate(saved.len() - 3);
        assert_eq!(tail(&saved, 1024), b"kept");
    }

    #[test]
    fn a_file_that_is_not_a_log_has_no_output() {
        assert!(tail(b"something else", 1024).is_empty());
    }
}
