//! D801 D: the old name of a renamed channel's folder, kept as a link to the
//! new one while a terminal that started under it still runs. The program
//! of such a terminal may name the old path (its `PWD`, a path it printed or
//! remembered), so the path has to keep working until it ends.
//!
//! The link is recorded with the channel, in `<root>/.aliases/<channel>.json`:
//! the links, the folder they point to and the holders that started under
//! them. A later rename points every link at the new folder. The link goes
//! when the last of those holders has ended, and when the daemon starts and
//! none runs. A link is only ever removed as a link: a folder of the same
//! name that has taken its place is never touched.

use std::fs;
use std::io::ErrorKind;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use rustix::fs::{CWD, RenameFlags, renameat_with};
use serde::{Deserialize, Serialize};

/// Where the records are kept, in the channels folder.
const ALIASES: &str = ".aliases";

/// A holder that started under an old name: its socket and its pid, so a
/// holder started later at the same socket (a resume) isn't taken for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holder {
    pub socket: PathBuf,
    pub pid: u32,
}

/// A channel's links and the holders that keep them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Aliases {
    /// The channel's folder the links point to.
    pub target: PathBuf,
    /// The old names of the folder, each a link to `target`.
    pub aliases: Vec<PathBuf>,
    /// The holders that started under one of them.
    pub holders: Vec<Holder>,
}

/// Why a channel's links couldn't be recorded or changed.
#[derive(Debug, thiserror::Error)]
pub enum AliasError {
    #[error(
        "couldn't {doing} {}; check that the Humata folder can be written and has room, then rename the channel again",
        path.display()
    )]
    Io {
        doing: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the record of the old folder names at {} is damaged; remove the file and the links it names, then rename the channel again",
        path.display()
    )]
    Damaged {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

fn io(doing: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> AliasError {
    let path = path.to_owned();
    move |source| AliasError::Io { doing, path, source }
}

fn record_path(root: &Path, channel: &str) -> PathBuf {
    root.join(ALIASES).join(format!("{channel}.json"))
}

/// The channel's record, empty when it has none.
pub fn load(root: &Path, channel: &str) -> Result<Aliases, AliasError> {
    let path = record_path(root, channel);
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|source| AliasError::Damaged { path, source }),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(Aliases::default()),
        Err(source) => Err(io("read", &path)(source)),
    }
}

fn save(root: &Path, channel: &str, aliases: &Aliases) -> Result<(), AliasError> {
    let path = record_path(root, channel);
    if aliases.aliases.is_empty() {
        return match fs::remove_file(&path) {
            Err(source) if source.kind() != ErrorKind::NotFound => Err(io("remove", &path)(source)),
            _ => Ok(()),
        };
    }
    if let Some(parent) = path.parent() {
        humata2_durable::create_dir_all(parent, 0o700).map_err(io("create", parent))?;
    }
    let bytes = serde_json::to_vec(aliases).map_err(|source| AliasError::Damaged { path: path.clone(), source })?;
    humata2_store::write_atomic(&path, &bytes).map_err(|source| io("write", &path)(std::io::Error::other(source)))
}

/// Whether `path` is a link the channel keeps for an old folder name.
pub fn is_alias(root: &Path, channel: &str, path: &Path) -> bool {
    load(root, channel).is_ok_and(|record| record.aliases.iter().any(|alias| alias == path))
        && fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_symlink())
}

/// Whether the process `holder` named is still the holder at its socket.
pub fn alive(holder: &Holder) -> bool {
    let process = i32::try_from(holder.pid).ok().and_then(rustix::process::Pid::from_raw);
    process.is_some_and(|pid| rustix::process::test_kill_process(pid).is_ok()) && UnixStream::connect(&holder.socket).is_ok()
}

/// Removes `path` when it is a link. Answers whether nothing of the
/// channel's is left there: a folder or file that took the name is left as
/// it is, and is no link of the channel's to keep.
fn remove_link(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::remove_file(path).is_ok(),
        Ok(_) | Err(_) => true,
    }
}

/// Frees the name of one of the channel's links so a folder can take it:
/// the rename back to an old name. Records that it is gone.
pub fn release(root: &Path, channel: &str, path: &Path) -> Result<(), AliasError> {
    let mut record = load(root, channel)?;
    if remove_link(path) {
        record.aliases.retain(|alias| alias != path);
        save(root, channel, &record)?;
    }
    Ok(())
}

/// Records, before the folder is renamed, that `old` is to become a link to
/// `new` for as long as `holders` run, with the links and holders the
/// channel already has. Answers what to put back if the rename fails.
pub fn begin(
    root: &Path,
    channel: &str,
    old: &Path,
    new: &Path,
    holders: &[Holder],
) -> Result<Aliases, AliasError> {
    let before = load(root, channel)?;
    let mut now = before.clone();
    now.target = new.to_owned();
    now.aliases.retain(|alias| alias != new);
    if !holders.is_empty() && !now.aliases.iter().any(|alias| alias == old) {
        now.aliases.push(old.to_owned());
    }
    for holder in holders {
        if !now.holders.contains(holder) {
            now.holders.push(holder.clone());
        }
    }
    save(root, channel, &now)?;
    Ok(before)
}

/// Puts the record back as [`begin`] found it.
pub fn undo(root: &Path, channel: &str, before: &Aliases) {
    let _ = save(root, channel, before);
}

/// Renames `old` to `new` and, with `link`, leaves a link at `old` to `new`
/// in the same step: the two names are swapped (`renamex_np(RENAME_SWAP)` on
/// a Mac, `RENAME_EXCHANGE` on Linux) with a placeholder link planted at
/// `new`, so there is no moment when `old` is missing. Where the folder
/// can't swap, it is renamed and the link made after.
pub fn rename_folder(old: &Path, new: &Path, link: bool) -> std::io::Result<()> {
    if !link {
        return humata2_durable::rename(old, new);
    }
    std::os::unix::fs::symlink(new, new)?;
    match renameat_with(CWD, old, CWD, new, RenameFlags::EXCHANGE) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::remove_file(new)?;
            humata2_durable::rename(old, new)?;
            std::os::unix::fs::symlink(new, old)
        }
    }
}

/// Points every link the channel keeps, `skip` aside, at its folder now,
/// each swapped in whole.
pub fn repoint(root: &Path, channel: &str, skip: &Path) -> Result<(), AliasError> {
    let record = load(root, channel)?;
    for alias in record.aliases.iter().filter(|alias| alias.as_path() != skip) {
        if !fs::symlink_metadata(alias).is_ok_and(|meta| meta.file_type().is_symlink()) {
            continue;
        }
        let mut name = alias.as_os_str().to_owned();
        name.push(".relink");
        let temporary = PathBuf::from(name);
        let _ = fs::remove_file(&temporary);
        std::os::unix::fs::symlink(&record.target, &temporary).map_err(io("link", &temporary))?;
        fs::rename(&temporary, alias).map_err(io("point", alias))?;
    }
    Ok(())
}

/// Points each link kept for something else that lives inside a folder
/// renamed from `from` to `to` (a Desk session moved into the channel,
/// recorded under its own key) at its folder's new place, so a later
/// channel rename never leaves it dangling. `own` is the renamed channel's
/// own record, which [`repoint`] already did.
pub fn repoint_under(root: &Path, own: &str, from: &Path, to: &Path) -> Result<(), AliasError> {
    let Ok(listing) = fs::read_dir(root.join(ALIASES)) else {
        return Ok(());
    };
    for entry in listing.flatten() {
        let Some(key) = entry
            .path()
            .file_stem()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
            .filter(|key| key != own)
        else {
            continue;
        };
        let mut record = load(root, &key)?;
        let Ok(inside) = record.target.strip_prefix(from).map(Path::to_owned) else {
            continue;
        };
        record.target = to.join(inside);
        save(root, &key, &record)?;
        repoint(root, &key, Path::new(""))?;
    }
    Ok(())
}

/// Removes the links of each channel whose holders have all ended, and the
/// record with them. Answers how many channels still keep links.
pub fn sweep(root: &Path) -> usize {
    let Ok(listing) = fs::read_dir(root.join(ALIASES)) else {
        return 0;
    };
    let mut kept = 0;
    for entry in listing.flatten() {
        let path = entry.path();
        let Some(channel) = path.file_stem().and_then(|name| name.to_str()).map(str::to_owned) else {
            continue;
        };
        let Ok(mut record) = load(root, &channel) else {
            continue;
        };
        if record.holders.iter().any(alive) {
            kept += 1;
            continue;
        }
        record.aliases.retain(|alias| !remove_link(alias));
        record.holders.clear();
        if save(root, &channel, &record).is_err() || !record.aliases.is_empty() {
            kept += 1;
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The old name keeps resolving to the renamed folder, and a later rename
    /// points it at the folder's newest place.
    #[test]
    fn the_old_name_follows_the_folder_through_renames() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b, c) = (root.join("a"), root.join("b"), root.join("c"));
        fs::create_dir_all(a.join("inside")).unwrap();
        let socket = root.join("h.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let holder = Holder { socket, pid: std::process::id() };

        begin(root, "ch_1", &a, &b, std::slice::from_ref(&holder)).unwrap();
        rename_folder(&a, &b, true).unwrap();
        assert!(a.join("inside").is_dir(), "the old name resolves");
        assert!(fs::symlink_metadata(&a).unwrap().file_type().is_symlink());

        begin(root, "ch_1", &b, &c, &[]).unwrap();
        rename_folder(&b, &c, false).unwrap();
        repoint(root, "ch_1", &b).unwrap();
        assert!(a.join("inside").is_dir(), "the oldest name follows too");
        assert_eq!(fs::read_link(&a).unwrap(), c);
    }

    /// With no holder answering, the links go and a folder that took a
    /// link's name is left alone.
    #[test]
    fn a_sweep_removes_the_links_once_no_holder_answers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = (root.join("a"), root.join("b"));
        fs::create_dir_all(&a).unwrap();
        let gone = Holder { socket: root.join("gone.sock"), pid: std::process::id() };
        begin(root, "ch_1", &a, &b, std::slice::from_ref(&gone)).unwrap();
        rename_folder(&a, &b, true).unwrap();

        assert_eq!(sweep(root), 0);

        assert!(fs::symlink_metadata(&a).is_err(), "the link is gone");
        assert!(b.is_dir(), "the folder stays");
        assert_eq!(load(root, "ch_1").unwrap(), Aliases::default());
    }

    /// A folder standing where a link was recorded is not removed.
    #[test]
    fn a_desk_link_into_a_channel_follows_the_channel_rename() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = (root.join("Payments"), root.join("Billing"));
        fs::create_dir_all(a.join("desk/se_1")).unwrap();
        fs::write(a.join("desk/se_1/NOTES.md"), "notes").unwrap();
        let old_desk = root.join("desk/se_1");
        fs::create_dir_all(old_desk.parent().unwrap()).unwrap();
        let holder = Holder { socket: root.join("h.sock"), pid: std::process::id() };
        begin(root, "desk-se_1", &old_desk, &a.join("desk/se_1"), std::slice::from_ref(&holder)).unwrap();
        std::os::unix::fs::symlink(a.join("desk/se_1"), &old_desk).unwrap();

        fs::rename(&a, &b).unwrap();
        repoint_under(root, "ch_1", &a, &b).unwrap();

        assert_eq!(fs::read_to_string(old_desk.join("NOTES.md")).unwrap(), "notes");
        assert_eq!(load(root, "desk-se_1").unwrap().target, b.join("desk/se_1"));
    }

    #[test]
    fn a_sweep_never_removes_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (a, b) = (root.join("a"), root.join("b"));
        fs::create_dir_all(&a).unwrap();
        begin(root, "ch_1", &a, &b, &[Holder { socket: root.join("gone.sock"), pid: std::process::id() }]).unwrap();

        sweep(root);

        assert!(a.is_dir());
    }
}
