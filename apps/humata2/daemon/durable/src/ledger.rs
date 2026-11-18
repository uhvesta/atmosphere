//! What was written under the simulated folders since its last sync, so a
//! power cut can be played back.
//!
//! One log, in order, for the whole process: a file's replaced bytes, and
//! folder entries made, renamed and removed. A file's sync drops its
//! bytes from the log; a folder's sync drops the entries in it. A cut
//! undoes what is left from the newest back. The log's lock is held while
//! each write is made, so a cut never runs beside one, and a cut keeps it
//! until the process is gone.

use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{self, ErrorKind};
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// A file, as its device and inode, so renames don't lose it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Id {
    dev: u64,
    ino: u64,
}

enum Op {
    /// `old` was at `offset`, and the file was `len` long.
    Data { id: Id, offset: u64, old: Vec<u8>, len: u64 },
    Created { path: PathBuf, waiting: Vec<PathBuf> },
    Renamed {
        from: PathBuf,
        to: PathBuf,
        /// What `to` was, kept aside, and which file it was.
        replaced: Option<(PathBuf, Id)>,
        waiting: Vec<PathBuf>,
    },
    Removed {
        path: PathBuf,
        kept: PathBuf,
        id: Id,
        waiting: Vec<PathBuf>,
    },
}

impl Op {
    fn waiting(&mut self) -> Option<&mut Vec<PathBuf>> {
        match self {
            Self::Data { .. } => None,
            Self::Created { waiting, .. }
            | Self::Renamed { waiting, .. }
            | Self::Removed { waiting, .. } => Some(waiting),
        }
    }
}

struct Ledger {
    roots: Vec<PathBuf>,
    spare: PathBuf,
    spares: u64,
    ops: Vec<Op>,
    paths: HashMap<Id, PathBuf>,
}

/// None when no power cut is being simulated.
fn ledger() -> Option<MutexGuard<'static, Ledger>> {
    static LEDGER: OnceLock<Option<Mutex<Ledger>>> = OnceLock::new();
    LEDGER
        .get_or_init(|| {
            let roots = std::env::var_os("HUMATA2_POWER_CUT_ROOTS")?;
            let spare = std::env::var_os("HUMATA2_POWER_CUT_SPARE")
                .map(PathBuf::from)
                .expect("HUMATA2_POWER_CUT_ROOTS needs HUMATA2_POWER_CUT_SPARE, a folder on the same file system");
            fs::create_dir_all(&spare).expect("couldn't make HUMATA2_POWER_CUT_SPARE");
            let roots = std::env::split_paths(&roots)
                .filter(|root| !root.as_os_str().is_empty())
                .map(|root| {
                    fs::create_dir_all(&root).expect("couldn't make a HUMATA2_POWER_CUT_ROOTS folder");
                    fs::canonicalize(&root).expect("couldn't resolve a HUMATA2_POWER_CUT_ROOTS folder")
                })
                .collect();
            Some(Mutex::new(Ledger {
                roots,
                spare: fs::canonicalize(&spare).expect("couldn't resolve HUMATA2_POWER_CUT_SPARE"),
                spares: 0,
                ops: Vec::new(),
                paths: HashMap::new(),
            }))
        })
        .as_ref()
        .map(|ledger| ledger.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

/// `path` with its folder resolved, so it can be compared with the roots.
/// None when its folder doesn't exist.
fn resolved(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?;
    let folder = match path.parent() {
        Some(folder) if !folder.as_os_str().is_empty() => folder,
        _ => Path::new("."),
    };
    Some(fs::canonicalize(folder).ok()?.join(name))
}

fn id_of(metadata: &fs::Metadata) -> Id {
    Id {
        dev: metadata.dev(),
        ino: metadata.ino(),
    }
}

impl Ledger {
    /// `path` resolved, if it's under a simulated folder.
    fn under(&self, path: &Path) -> Option<PathBuf> {
        let path = resolved(path)?;
        self.roots.iter().any(|root| path.starts_with(root)).then_some(path)
    }

    /// Hard-links `path` aside, to put back if what replaces or removes it
    /// is undone.
    fn keep(&mut self, path: &Path) -> io::Result<PathBuf> {
        self.spares += 1;
        let kept = self.spare.join(format!("{}-{}", std::process::id(), self.spares));
        fs::hard_link(path, &kept)?;
        Ok(kept)
    }

    fn moved(&mut self, from: &Path, to: &Path) {
        for path in self.paths.values_mut() {
            if let Ok(rest) = path.strip_prefix(from) {
                *path = if rest.as_os_str().is_empty() {
                    to.to_owned()
                } else {
                    to.join(rest)
                };
            }
        }
    }

    fn undo(&mut self) {
        while let Some(op) = self.ops.pop() {
            let undone = match op {
                Op::Data { id, offset, old, len } => match self.paths.get(&id) {
                    // A file deleted since (which the log treats as
                    // lasting) may have a new one in its place: leave that
                    // alone.
                    Some(path) => OpenOptions::new()
                        .write(true)
                        .open(path)
                        .and_then(|file| {
                            if id_of(&file.metadata()?) != id {
                                return Ok(());
                            }
                            file.set_len(len)?;
                            file.write_all_at(&old, offset)
                        })
                        .or_else(missing_is_fine),
                    None => Ok(()),
                },
                Op::Created { path, .. } => match fs::symlink_metadata(&path) {
                    Ok(found) if found.is_dir() => fs::remove_dir_all(&path),
                    Ok(_) => fs::remove_file(&path),
                    Err(error) => missing_is_fine(error),
                },
                Op::Renamed { from, to, replaced, .. } => {
                    let back = fs::rename(&to, &from);
                    self.moved(&to, &from);
                    back.and_then(|()| match replaced {
                        Some((kept, id)) => {
                            self.paths.insert(id, to.clone());
                            fs::rename(kept, &to)
                        }
                        None => Ok(()),
                    })
                }
                Op::Removed { path, kept, id, .. } => {
                    self.paths.insert(id, path.clone());
                    fs::rename(kept, path)
                }
            };
            if let Err(error) = undone {
                // The simulation is wrong from here; nothing the test then
                // checks can be trusted.
                eprintln!("humata2_durable: couldn't undo an unsynced write: {error}");
                std::process::abort();
            }
        }
    }
}

fn missing_is_fine(error: io::Error) -> io::Result<()> {
    if error.kind() == ErrorKind::NotFound {
        Ok(())
    } else {
        Err(error)
    }
}

fn folder_of(path: &Path) -> PathBuf {
    path.parent().map(Path::to_owned).unwrap_or_default()
}

/// Opens `path`, remembering it if it's under a simulated folder, and that
/// it's new if it is.
pub(crate) fn open(options: &OpenOptions, path: &Path) -> io::Result<(fs::File, Option<Id>)> {
    let Some(mut ledger) = ledger() else {
        return options.open(path).map(|file| (file, None));
    };
    let Some(resolved) = ledger.under(path) else {
        return options.open(path).map(|file| (file, None));
    };
    let existed = fs::symlink_metadata(&resolved).is_ok();
    let file = options.open(path)?;
    let id = id_of(&file.metadata()?);
    if !existed {
        let waiting = vec![folder_of(&resolved)];
        ledger.ops.push(Op::Created {
            path: resolved.clone(),
            waiting,
        });
    }
    ledger.paths.insert(id, resolved);
    Ok((file, Some(id)))
}

/// Remembers what `len` bytes at `offset` of the file replace, then
/// `write` makes the change, with the log locked.
pub(crate) fn changing<T>(
    id: Id,
    offset: u64,
    replaced: u64,
    current_len: impl FnOnce() -> io::Result<u64>,
    read: impl FnOnce(u64, usize) -> io::Result<Vec<u8>>,
    write: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let Some(mut ledger) = ledger() else {
        return write();
    };
    let len = current_len()?;
    let end = offset.saturating_add(replaced).min(len);
    let old = if end > offset {
        let want = usize::try_from(end - offset).unwrap_or(usize::MAX);
        match read(offset, want) {
            Ok(old) => old,
            // A handle opened write-only can't read what it replaces (EBADF);
            // the file's own path, while it still names this file, can.
            Err(error) => match ledger.paths.get(&id) {
                Some(path) => read_by_path(path, id, offset, want).map_err(|_| error)?,
                None => return Err(error),
            },
        }
    } else {
        Vec::new()
    };
    ledger.ops.push(Op::Data { id, offset, old, len });
    write()
}

/// Reads `len` bytes at `at` of the file at `path`, when that is still file `id`.
fn read_by_path(path: &Path, id: Id, at: u64, len: usize) -> io::Result<Vec<u8>> {
    let file = fs::File::open(path)?;
    if id_of(&file.metadata()?) != id {
        return Err(io::Error::new(io::ErrorKind::NotFound, "the path names another file now"));
    }
    read_at(&file, at, len)
}

pub(crate) fn write(file: &fs::File, id: Id, bytes: &[u8], offset: u64) -> io::Result<()> {
    changing(
        id,
        offset,
        bytes.len() as u64,
        || Ok(file.metadata()?.len()),
        |at, len| read_at(file, at, len),
        || file.write_all_at(bytes, offset),
    )
}

pub(crate) fn set_len(file: &fs::File, id: Id, len: u64) -> io::Result<()> {
    changing(
        id,
        len,
        u64::MAX,
        || Ok(file.metadata()?.len()),
        |at, len| read_at(file, at, len),
        || file.set_len(len),
    )
}

fn read_at(file: &fs::File, at: u64, len: usize) -> io::Result<Vec<u8>> {
    let mut old = vec![0; len];
    file.read_exact_at(&mut old, at)?;
    Ok(old)
}

/// The file's bytes are on disk: a cut keeps them.
pub(crate) fn synced(id: Id) {
    if let Some(mut ledger) = ledger() {
        ledger
            .ops
            .retain(|op| !matches!(op, Op::Data { id: of, .. } if *of == id));
    }
}

/// Remembers `path` under `id`, for files opened outside [`open`] (such
/// as SQLite's). False when it isn't under a simulated folder.
pub(crate) fn track(path: &Path, id: Id) -> bool {
    let Some(mut ledger) = ledger() else {
        return false;
    };
    let Some(resolved) = ledger.under(path) else {
        return false;
    };
    ledger.paths.insert(id, resolved);
    true
}

pub(crate) fn id_of_path(path: &Path) -> io::Result<Id> {
    fs::metadata(path).map(|found| id_of(&found))
}

pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    let Some(mut ledger) = ledger() else {
        return fs::rename(from, to);
    };
    let (Some(from), Some(to)) = (ledger.under(from), ledger.under(to)) else {
        return fs::rename(from, to);
    };
    let replaced = match fs::symlink_metadata(&to) {
        Ok(found) => Some((ledger.keep(&to)?, id_of(&found))),
        Err(_) => None,
    };
    if let Err(error) = fs::rename(&from, &to) {
        if let Some((kept, _)) = replaced {
            let _ = fs::remove_file(kept);
        }
        return Err(error);
    }
    if let Some((_, id)) = replaced {
        ledger.paths.remove(&id);
    }
    ledger.moved(&from, &to);
    let mut waiting = vec![folder_of(&from)];
    if folder_of(&to) != waiting[0] {
        waiting.push(folder_of(&to));
    }
    ledger.ops.push(Op::Renamed {
        from,
        to,
        replaced,
        waiting,
    });
    Ok(())
}

pub(crate) fn remove_file(path: &Path) -> io::Result<()> {
    let Some(mut ledger) = ledger() else {
        return fs::remove_file(path);
    };
    let Some(resolved) = ledger.under(path) else {
        return fs::remove_file(path);
    };
    let id = id_of(&fs::symlink_metadata(&resolved)?);
    let kept = ledger.keep(&resolved)?;
    if let Err(error) = fs::remove_file(&resolved) {
        let _ = fs::remove_file(kept);
        return Err(error);
    }
    ledger.paths.remove(&id);
    let waiting = vec![folder_of(&resolved)];
    ledger.ops.push(Op::Removed {
        path: resolved,
        kept,
        id,
        waiting,
    });
    Ok(())
}

pub(crate) fn create_dir_all(path: &Path, mode: u32) -> io::Result<()> {
    let mut missing: Vec<&Path> = path
        .ancestors()
        .take_while(|folder| !folder.as_os_str().is_empty() && fs::symlink_metadata(folder).is_err())
        .collect();
    fs::DirBuilder::new().recursive(true).mode(mode).create(path)?;
    let Some(mut ledger) = ledger() else {
        return Ok(());
    };
    // Outermost first, so they are undone innermost first.
    missing.reverse();
    for folder in missing {
        if let Some(resolved) = ledger.under(folder) {
            let waiting = vec![folder_of(&resolved)];
            ledger.ops.push(Op::Created {
                path: resolved,
                waiting,
            });
        }
    }
    Ok(())
}

/// The folder's entries are on disk: a cut keeps the files made, renamed
/// and removed in it.
pub(crate) fn dir_synced(path: &Path) {
    let Some(mut ledger) = ledger() else {
        return;
    };
    let Ok(folder) = fs::canonicalize(path) else {
        return;
    };
    let mut durable = Vec::new();
    for (at, op) in ledger.ops.iter_mut().enumerate() {
        if let Some(waiting) = op.waiting() {
            waiting.retain(|other| *other != folder);
            if waiting.is_empty() {
                durable.push(at);
            }
        }
    }
    for at in durable.into_iter().rev() {
        if let Op::Renamed {
            replaced: Some((kept, _)),
            ..
        }
        | Op::Removed { kept, .. } = ledger.ops.remove(at)
        {
            let _ = fs::remove_file(kept);
        }
    }
}

/// Puts back every write not yet synced, newest first, and forgets them.
/// For tests of this crate; [`cut`] is what fault points use.
pub fn revert_unsynced() {
    if let Some(mut ledger) = ledger() {
        ledger.undo();
    }
}

pub(crate) fn cut(name: &str) -> ! {
    // Held until the process is gone, so no other thread writes after.
    let held = ledger().map(|mut ledger| {
        ledger.undo();
        ledger
    });
    let sent = rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::KILL);
    if let Err(error) = sent {
        eprintln!("couldn't stop at {name} with SIGKILL: {error}");
        std::process::abort();
    }
    // SIGKILL arrives asynchronously; nothing goes on past here.
    let _held = held;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
