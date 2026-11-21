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
use std::io::{BufReader, ErrorKind, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
/// How much a rewrite gathers before it writes.
const REWRITE_BATCH: usize = 1 << 20;
/// How far past the output sent a reservation reaches, at least.
const RESERVE_AHEAD: u64 = 1 << 20;
/// How far it reaches at most, when output keeps outrunning it.
const MOST_AHEAD: u64 = 16 << 20;
/// How often output written since the last sync is synced.
pub(crate) const SYNC_EVERY: Duration = Duration::from_millis(500);
/// Initial wait before retrying after a save error; doubles up to
/// `RETRY_MAX` while errors persist.
const RETRY_BASE: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// Where a holder on `socket` saves its output.
pub fn saved_scrollback_path(socket: &Path) -> PathBuf {
    socket.with_extension("scrollback")
}

/// Where the daemon marks that the user stopped the terminal of `socket`
/// (D784 "Stop"): a start's revive leaves it stopped, and only Resume
/// starts it again, which takes the mark away.
pub fn stopped_mark_path(socket: &Path) -> PathBuf {
    socket.with_extension("stopped")
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
    let failed = |source| HolderError::Saved {
        path: path.to_owned(),
        source,
    };
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    let parsed = parse_reader(BufReader::new(file), 1).map_err(failed)?;
    Ok(Some(parsed.summary()))
}

/// The last `max_bytes` of output a holder saved at `path`, oldest first,
/// or `None` if it saved nothing there or was shut down. Output a holder
/// that took over printed after a gap follows with nothing between.
pub fn read_saved_tail(path: &Path, max_bytes: usize) -> Result<Option<Vec<u8>>, HolderError> {
    let failed = |source| HolderError::Saved {
        path: path.to_owned(),
        source,
    };
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(failed(error)),
    };
    let parsed = parse_reader(BufReader::new(file), max_bytes).map_err(failed)?;
    Ok(Some(
        parsed
            .kept
            .since(0)
            .into_iter()
            .flat_map(|(_, data)| data)
            .collect(),
    ))
}

/// The last `max_bytes` of the output in the saved log `bytes`.
#[cfg(test)]
fn tail(bytes: &[u8], max_bytes: usize) -> Vec<u8> {
    parse_reader(bytes, max_bytes)
        .expect("reading from a slice cannot fail")
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

enum ReadStatus {
    Full,
    Eof,
    Partial,
}

fn read_exact_or_eof(reader: &mut impl Read, mut buf: &mut [u8]) -> std::io::Result<ReadStatus> {
    let total = buf.len();
    while !buf.is_empty() {
        match reader.read(buf) {
            Ok(0) => {
                return Ok(if buf.len() == total {
                    ReadStatus::Eof
                } else {
                    ReadStatus::Partial
                });
            }
            Ok(n) => buf = &mut buf[n..],
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(ReadStatus::Full)
}

fn parse_reader(mut reader: impl Read, limit: usize) -> std::io::Result<Parsed> {
    let mut parsed = Parsed {
        kept: Scrollback::new(limit),
        reserved: 0,
        tail_damaged: false,
    };
    let mut magic = [0u8; 8];
    match read_exact_or_eof(&mut reader, &mut magic)? {
        ReadStatus::Eof => return Ok(parsed),
        ReadStatus::Partial => {
            parsed.tail_damaged = true;
            return Ok(parsed);
        }
        ReadStatus::Full => {
            if &magic != MAGIC {
                parsed.tail_damaged = true;
                return Ok(parsed);
            }
        }
    }
    let mut head = [0u8; HEAD];
    let mut check = [0u8; CHECK];
    let mut data = Vec::new();
    loop {
        match read_exact_or_eof(&mut reader, &mut head)? {
            ReadStatus::Eof => break,
            ReadStatus::Partial => {
                parsed.tail_damaged = true;
                break;
            }
            ReadStatus::Full => {}
        }
        let kind = head[0];
        let offset = u64::from_le_bytes(head[1..9].try_into().expect("8-byte offset"));
        let Ok(len) = usize::try_from(u32::from_le_bytes(
            head[9..13].try_into().expect("4-byte len"),
        )) else {
            parsed.tail_damaged = true;
            break;
        };
        data.clear();
        let read = reader.by_ref().take(len as u64).read_to_end(&mut data)?;
        if read != len {
            parsed.tail_damaged = true;
            break;
        }
        match read_exact_or_eof(&mut reader, &mut check)? {
            ReadStatus::Full => {}
            ReadStatus::Eof | ReadStatus::Partial => {
                parsed.tail_damaged = true;
                break;
            }
        }
        let expected = checksum_step(checksum(&head), &data);
        if u64::from_le_bytes(check) != expected {
            parsed.tail_damaged = true;
            break;
        }
        match kind {
            // Offsets only ever grow; one that doesn't is damage.
            OUTPUT if offset >= parsed.kept.end() => {
                parsed.kept.skip_to(offset);
                parsed.kept.push(&data);
            }
            RESERVE => parsed.reserved = parsed.reserved.max(offset),
            _ => {
                parsed.tail_damaged = true;
                break;
            }
        }
    }
    Ok(parsed)
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
pub(crate) fn checksum(bytes: &[u8]) -> u64 {
    checksum_step(0xcbf2_9ce4_8422_2325, bytes)
}

fn checksum_step(seed: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(seed, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// What a holder that moves onto another build hands over about its open
/// log, so the next image appends to the same file where this one stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SaveParts {
    pub(crate) path: PathBuf,
    pub(crate) len: u64,
    pub(crate) reserved: u64,
    pub(crate) ahead: u64,
    pub(crate) failed: bool,
}

/// Backoff state while recovering from a transient save or sync error.
struct FailedSave {
    next_retry: Instant,
    backoff: Duration,
}

/// The open log a running holder appends its output to.
pub(crate) struct SaveLog {
    path: PathBuf,
    file: File,
    len: u64,
    reserved: u64,
    unsynced: bool,
    /// A new log isn't synced when it is made: nothing on it is anyone's
    /// yet, and an empty log lost in a power cut reads as no log. It is
    /// synced whole, folder included, before the first reservation counts.
    made_unsynced: bool,
    /// How far the last reservation reached past the output, and when it
    /// was made: each sync on macOS is a flush of the whole disk, so a
    /// flood mustn't wait on one per MiB.
    ahead: u64,
    reserved_at: Option<Instant>,
    failed: Option<FailedSave>,
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
        let (parsed, new) = match fs::File::open(path) {
            Ok(file) => (parse_reader(BufReader::new(file), limit).map_err(failed)?, false),
            Err(error) if error.kind() == ErrorKind::NotFound => (
                Parsed {
                    kept: Scrollback::new(limit),
                    reserved: 0,
                    tail_damaged: false,
                },
                true,
            ),
            Err(error) => return Err(failed(error)),
        };
        let mut kept = parsed.kept;
        kept.skip_to(parsed.reserved);
        // Written again whole, so a damaged tail doesn't stay to hide what
        // comes after it. A new one is synced at its first reservation, so
        // a holder's start doesn't wait on a disk flush (F_FULLFSYNC on
        // macOS waits behind every other flush on the disk).
        let (file, len) = rewrite(path, &kept, !new).map_err(failed)?;
        let log = Self {
            path: path.to_owned(),
            file,
            len,
            reserved: kept.end(),
            unsynced: false,
            made_unsynced: new,
            ahead: RESERVE_AHEAD,
            reserved_at: None,
            failed: None,
        };
        Ok((log, kept))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// What the next image needs to go on with this log. Call it after
    /// everything written is synced.
    pub(crate) fn parts(&self) -> SaveParts {
        SaveParts {
            path: self.path.clone(),
            len: self.len,
            reserved: self.reserved,
            ahead: self.ahead,
            failed: self.failed.is_some(),
        }
    }

    /// Opens the log `parts` describes to append to, as it was left: it
    /// isn't read or rewritten.
    pub(crate) fn reopen(parts: SaveParts) -> Result<Self, HolderError> {
        let file = File::open(OpenOptions::new().read(true).write(true), &parts.path).map_err(
            |source| HolderError::Saved {
                path: parts.path.clone(),
                source,
            },
        )?;
        let failed = parts.failed.then(|| FailedSave {
            next_retry: Instant::now(),
            backoff: RETRY_BASE,
        });
        Ok(Self {
            path: parts.path,
            file,
            len: parts.len,
            reserved: parts.reserved,
            unsynced: false,
            made_unsynced: false,
            ahead: parts.ahead,
            reserved_at: None,
            failed,
        })
    }

    /// Saves `data`, printed at `offset` and already pushed to `kept`.
    /// Called before any client is sent it. If saving fails it logs once
    /// and backs off, then rewrites the log from `kept` on later output
    /// once the disk is writable again.
    pub(crate) fn output(&mut self, offset: u64, data: &[u8], kept: &Scrollback) {
        self.output_at(offset, data, kept, Instant::now());
    }

    fn output_at(&mut self, offset: u64, data: &[u8], kept: &Scrollback, now: Instant) {
        if let Some(failed) = self.failed.as_ref() {
            if now < failed.next_retry {
                return;
            }
            let prev_backoff = failed.backoff;
            match self.recover(kept) {
                Ok(()) => {
                    self.failed = None;
                    eprintln!(
                        "humata2_holder: resumed saving this terminal's output to {}",
                        self.path.display()
                    );
                }
                Err(_) => {
                    let backoff = (prev_backoff * 2).min(RETRY_MAX);
                    self.failed = Some(FailedSave {
                        next_retry: now + backoff,
                        backoff,
                    });
                }
            }
            return;
        }
        if let Err(error) = self.save(offset, data, kept, now) {
            self.unsynced = false;
            self.failed = Some(FailedSave {
                next_retry: now + RETRY_BASE,
                backoff: RETRY_BASE,
            });
            eprintln!(
                "humata2_holder: couldn't save this terminal's output to {}: {error}. Will retry as more output arrives; free some disk space, the terminal itself goes on",
                self.path.display()
            );
        }
    }

    /// Records that a background `sync_data` failed so the next output
    /// after backoff rewrites and syncs the log cleanly.
    pub(crate) fn note_sync_failed(&mut self) {
        self.unsynced = false;
        if self.failed.is_none() {
            let now = Instant::now();
            self.failed = Some(FailedSave {
                next_retry: now + RETRY_BASE,
                backoff: RETRY_BASE,
            });
        }
    }

    fn recover(&mut self, kept: &Scrollback) -> std::io::Result<()> {
        (self.file, self.len) = rewrite(&self.path, kept, true)?;
        self.reserved = kept.end();
        self.unsynced = false;
        self.made_unsynced = false;
        self.reserved_at = None;
        Ok(())
    }

    fn save(
        &mut self,
        offset: u64,
        data: &[u8],
        kept: &Scrollback,
        now: Instant,
    ) -> std::io::Result<()> {
        let end = offset + data.len() as u64;
        // Compact once the log grows past what's kept plus headroom, clamped so
        // small scrollbacks don't thrash rewrites.
        let headroom = ((kept.limit() / 2) as u64).max(2 << 20);
        let grown = self.len > kept.limit() as u64 + headroom;
        if grown {
            return self.recover(kept);
        }
        let mut out = Vec::with_capacity(data.len() + 2 * (HEAD + CHECK));
        let mut at = offset;
        for chunk in data.chunks(CHUNK) {
            put_record(&mut out, OUTPUT, at, chunk);
            at += chunk.len() as u64;
        }
        let reserving = end > self.reserved;
        if reserving {
            self.ahead = next_ahead(self.ahead, self.reserved_at.map(|at| now.saturating_duration_since(at)));
            self.reserved_at = Some(now);
            self.reserved = end + self.ahead;
            put_record(&mut out, RESERVE, self.reserved, &[]);
        }
        self.file.append(&out)?;
        self.len += out.len() as u64;
        if reserving && self.made_unsynced {
            self.file.sync_all()?;
            if let Some(folder) = self.path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
                sync_dir(folder)?;
            }
            self.made_unsynced = false;
            self.unsynced = false;
        } else if reserving {
            self.file.sync_data()?;
            self.unsynced = false;
        } else {
            self.unsynced = true;
        }
        Ok(())
    }

    /// The file to sync, if anything was written since the last sync.
    pub(crate) fn take_unsynced(&mut self) -> Option<File> {
        if !std::mem::take(&mut self.unsynced) || self.failed.is_some() {
            return None;
        }
        self.file.try_clone().ok()
    }
}

/// How far the next reservation reaches, given the last one's reach and
/// how long ago it was made: twice as far while output uses each up within
/// a sync interval, back to the least once it doesn't. Reaching further
/// costs nothing but offsets a takeover skips.
fn next_ahead(last: u64, since: Option<Duration>) -> u64 {
    match since {
        Some(since) if since < SYNC_EVERY => (last * 2).clamp(RESERVE_AHEAD, MOST_AHEAD),
        _ => RESERVE_AHEAD,
    }
}

/// Writes `kept` to `path` whole, through a file renamed into place, and
/// returns the file opened to append to and its length. With `sync`, the
/// file and its folder are synced before it is returned.
fn rewrite(path: &Path, kept: &Scrollback, sync: bool) -> std::io::Result<(File, u64)> {
    // Written a few records at a time, straight from what's kept: a
    // rewrite of a full scrollback used to copy it twice first, into
    // memory the holder then kept (FAST, D789).
    let mut out = Vec::with_capacity(REWRITE_BATCH + HEAD + CHECK);
    out.extend_from_slice(MAGIC);

    let partial = path.with_extension("scrollback-partial");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true).mode(0o600);
    let mut file = File::open(&options, &partial)?;
    let mut len = 0u64;
    let mut write = |out: &mut Vec<u8>| -> std::io::Result<()> {
        file.append(out)?;
        len += out.len() as u64;
        out.clear();
        Ok(())
    };
    let mut failed = None;
    kept.each_piece(|at, piece| {
        if failed.is_some() {
            return;
        }
        let mut at = at;
        for chunk in piece.chunks(CHUNK) {
            put_record(&mut out, OUTPUT, at, chunk);
            at += chunk.len() as u64;
            if out.len() >= REWRITE_BATCH
                && let Err(error) = write(&mut out)
            {
                failed = Some(error);
                return;
            }
        }
    });
    if let Some(error) = failed {
        return Err(error);
    }
    put_record(&mut out, RESERVE, kept.end(), &[]);
    write(&mut out)?;
    if sync {
        file.sync_all()?;
    }
    drop(file);
    rename(&partial, path)?;
    if sync && let Some(folder) = path.parent().filter(|folder| !folder.as_os_str().is_empty()) {
        sync_dir(folder)?;
    }
    let file = File::open(OpenOptions::new().read(true).write(true), path)?;
    Ok((file, len))
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
    fn a_flood_reserves_further_each_time_up_to_a_limit() {
        let quick = Some(Duration::from_millis(10));
        let mut ahead = RESERVE_AHEAD;
        let mut syncs = 0;
        let mut reached = 0;
        while reached < 64 << 20 {
            ahead = next_ahead(ahead, quick);
            reached += ahead;
            syncs += 1;
        }
        assert!(syncs < 10, "a 64 MiB flood took {syncs} syncs");
        assert_eq!(ahead, MOST_AHEAD);
    }

    #[test]
    fn output_after_a_pause_reserves_the_least_again() {
        assert_eq!(next_ahead(MOST_AHEAD, Some(SYNC_EVERY * 2)), RESERVE_AHEAD);
        assert_eq!(next_ahead(MOST_AHEAD, None), RESERVE_AHEAD);
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

    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "h2-saved-{tag}-{}-{n}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_transient_save_error_recovers_on_later_output_and_reloads() {
        let dir = ScratchDir::new("recover");
        let path = dir.0.join("t.scrollback");
        let (mut log, mut kept) = SaveLog::open(&path, 4096).unwrap();
        let t0 = Instant::now();

        let write_chunk = |log: &mut SaveLog, kept: &mut Scrollback, data: &[u8], at: Instant| {
            let offset = kept.end();
            kept.push(data);
            log.output_at(offset, data, kept, at);
        };

        write_chunk(&mut log, &mut kept, b"before ", t0);
        assert!(!log.parts().failed);

        // Simulate a transient write failure by swapping the open handle for a
        // read-only descriptor (append fails with EBADF on every Unix host).
        log.file = File::open(OpenOptions::new().read(true), &path).unwrap();
        write_chunk(&mut log, &mut kept, b"during ", t0);
        assert!(log.parts().failed);
        assert!(log.take_unsynced().is_none());

        // Before backoff elapses, output does not thrash retries.
        write_chunk(&mut log, &mut kept, b"waiting ", t0 + Duration::from_millis(100));
        assert!(log.parts().failed);

        // Once backoff elapses, recovery rewrites the full scrollback and clears failure.
        let t1 = t0 + RETRY_BASE;
        write_chunk(&mut log, &mut kept, b"after ", t1);
        assert!(!log.parts().failed);

        // Subsequent output appends and syncs normally. The first write after
        // recovery reserves ahead and syncs immediately; the next write within
        // that reservation leaves unsynced data for the background syncer.
        write_chunk(&mut log, &mut kept, b"recovered", t1);
        assert!(log.take_unsynced().is_none());
        write_chunk(&mut log, &mut kept, b"!", t1);
        let to_sync = log.take_unsynced().expect("recovered log must track unsynced writes");
        to_sync.sync_data().unwrap();

        let expected = b"before during waiting after recovered!";
        let summary = read_saved(&path).unwrap().unwrap();
        assert_eq!(summary.saved_through, expected.len() as u64);
        assert!(!summary.tail_damaged);
        assert_eq!(read_saved_tail(&path, 4096).unwrap().unwrap(), expected);

        let (_, reloaded) = SaveLog::open(&path, 4096).unwrap();
        let reloaded_bytes: Vec<u8> = reloaded
            .since(0)
            .into_iter()
            .flat_map(|(_, bytes)| bytes)
            .collect();
        assert_eq!(reloaded_bytes, expected);
    }

    #[test]
    fn repeated_save_errors_back_off_and_still_recover_once_cleared() {
        let dir = ScratchDir::new("backoff");
        let path = dir.0.join("t.scrollback");
        let partial = path.with_extension("scrollback-partial");
        let (mut log, mut kept) = SaveLog::open(&path, 4096).unwrap();
        let t0 = Instant::now();

        let write_chunk = |log: &mut SaveLog, kept: &mut Scrollback, data: &[u8], at: Instant| {
            let offset = kept.end();
            kept.push(data);
            log.output_at(offset, data, kept, at);
        };

        // Block both the open fd and the rewrite temp path.
        log.file = File::open(OpenOptions::new().read(true), &path).unwrap();
        fs::create_dir(&partial).unwrap();

        write_chunk(&mut log, &mut kept, b"one ", t0);
        assert!(log.parts().failed);

        // First retry at RETRY_BASE still fails (partial is a directory) and doubles backoff.
        let t1 = t0 + RETRY_BASE;
        write_chunk(&mut log, &mut kept, b"two ", t1);
        assert!(log.parts().failed);

        // Remove the obstacle, but before the doubled backoff passes it still waits.
        fs::remove_dir(&partial).unwrap();
        write_chunk(&mut log, &mut kept, b"three ", t1 + RETRY_BASE);
        assert!(log.parts().failed);

        // At doubled backoff, recovery succeeds and flushes everything kept.
        let t2 = t1 + RETRY_BASE * 2;
        write_chunk(&mut log, &mut kept, b"four", t2);
        assert!(!log.parts().failed);
        assert_eq!(
            read_saved_tail(&path, 4096).unwrap().unwrap(),
            b"one two three four"
        );
    }
}
