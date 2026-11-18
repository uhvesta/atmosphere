//! What follows a channel's folder when it moves (MV-16, D314): the paths
//! that files inside it name, and the conversations its agents keep by
//! folder. A rename moves the folder with one `rename(2)`; a channel put
//! back from the trash under another name moves nowhere, but its files
//! still name the place it was deleted from. Both come here with the
//! [`Placement`] and make what names the old folder name the new one.
//!
//! Every step can run again: a file already rewritten names nothing to
//! rewrite, and a conversation already carried is only removed from where
//! it was.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use humata2_store::{Placement, StoreError, write_atomic};

/// The biggest file whose text is searched for the old folder's path.
const TEXT_MAX: u64 = 16 << 20;

/// How deep a note's folder is searched.
const DEPTH_MAX: usize = 8;

/// Folders a search never enters: git's objects are never text, and the
/// others are no one's notes.
const SKIPPED: [&str; 3] = ["objects", "node_modules", "target"];

/// Why a moved channel's files could not be brought up to date.
#[derive(Debug, thiserror::Error)]
pub(crate) enum MoveError {
    #[error(
        "couldn't {doing} {}: {source}; fix the permissions or free the disk space the message names",
        path.display()
    )]
    Io {
        doing: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "couldn't save the conversation at {}; free the disk space or fix the permissions of its folder",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
}

fn io(doing: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> MoveError {
    let path = path.to_owned();
    move |source| MoveError::Io { doing, path, source }
}

/// The entries of `dir`, none when it isn't there.
fn entries_of(dir: &Path) -> Result<Vec<fs::DirEntry>, MoveError> {
    match fs::read_dir(dir) {
        Ok(entries) => entries.collect::<Result<Vec<_>, _>>().map_err(io("read the folder", dir)),
        Err(source) if source.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(source) => Err(io("read the folder", dir)(source)),
    }
}

/// The folders directly in `dir` (never links to them), in order.
fn folders_in(dir: &Path) -> Result<Vec<PathBuf>, MoveError> {
    let mut found = Vec::new();
    for entry in entries_of(dir)? {
        let path = entry.path();
        // One removed since the listing (an archived session's folder freed
        // meanwhile) is no folder to look in.
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => found.push(path),
            Ok(_) => {}
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(io("look at", &path)(source)),
        }
    }
    found.sort();
    Ok(found)
}

/// Replaces `file`'s bytes with `bytes`, keeping its permissions: written
/// beside it, fsynced, then renamed over it, so a power cut leaves the old
/// file or the whole new one.
fn replace_file(file: &Path, bytes: &[u8], permissions: fs::Permissions) -> std::io::Result<()> {
    let temporary = humata2_sessions::partial(file);
    let written = (|| {
        let mut made = fs::File::create(&temporary)?;
        made.write_all(bytes)?;
        made.set_permissions(permissions)?;
        made.sync_all()?;
        drop(made);
        humata2_durable::rename(&temporary, file)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// Rewrites `file` when it is text that names the old folder. A file that
/// is too big or isn't text is left as it is.
fn rewrite_file(placement: &Placement, known: &[PathBuf], file: &Path, meta: &fs::Metadata) -> Result<(), MoveError> {
    if meta.len() > TEXT_MAX {
        return Ok(());
    }
    let bytes = match fs::read(file) {
        Ok(bytes) => bytes,
        // Removed since it was listed: nothing is left to name the old folder.
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(io("read", file)(source)),
    };
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Ok(());
    };
    if let Some(now) = placement.rewritten_among(text, known) {
        match replace_file(file, now.as_bytes(), meta.permissions()) {
            // Its folder was removed while it was rewritten (an archived
            // session freed at start, beside a restore finishing): gone is done.
            Err(source) if source.kind() == ErrorKind::NotFound && fs::symlink_metadata(file).is_err() => {}
            written => written.map_err(io("rewrite", file))?,
        }
    }
    Ok(())
}

/// Rewrites the text files under `dir`. `git` says it is a `.git` folder,
/// whose index is no text.
fn rewrite_tree(placement: &Placement, known: &[PathBuf], dir: &Path, git: bool, depth: usize) -> Result<(), MoveError> {
    for entry in entries_of(dir)? {
        let path = entry.path();
        let name = entry.file_name();
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(source) if source.kind() == ErrorKind::NotFound => continue,
            Err(source) => return Err(io("look at", &path)(source)),
        };
        if meta.is_dir() {
            if depth < DEPTH_MAX && !SKIPPED.iter().any(|skipped| name == *skipped) {
                rewrite_tree(placement, known, &path, git, depth + 1)?;
            }
        } else if meta.is_file() && !(git && name == "index") {
            rewrite_file(placement, known, &path, &meta)?;
        }
    }
    Ok(())
}

/// Rewrites what a clone's `.git` can name of its own place and its
/// origin's: its config, its logs, its hooks. A `.git` that is a file (a
/// worktree's pointer) is rewritten as one.
fn rewrite_git(placement: &Placement, known: &[PathBuf], clone: &Path) -> Result<(), MoveError> {
    let git = clone.join(".git");
    match fs::symlink_metadata(&git) {
        Ok(meta) if meta.is_dir() => rewrite_tree(placement, known, &git, true, 0),
        Ok(meta) if meta.is_file() => rewrite_file(placement, known, &git, &meta),
        _ => Ok(()),
    }
}

/// The folders inside the channel's new folder (its clones, its sessions
/// and their clones, its Desk sessions), each as it was under the old
/// path: the ones whose conversation keys the text of a file can name.
fn known_folders(placement: &Placement) -> Result<Vec<PathBuf>, MoveError> {
    let channel = &placement.new_path;
    let mut folders = Vec::new();
    for group in ["main", "sessions", "desk"] {
        for folder in folders_in(&channel.join(group))? {
            if group == "sessions" {
                folders.extend(folders_in(&folder)?);
            }
            folders.push(folder);
        }
    }
    Ok(folders
        .iter()
        .filter_map(|folder| folder.strip_prefix(channel).ok())
        .map(|rest| placement.old_path.join(rest))
        .collect())
}

/// Makes what the files inside the channel's new folder name of the old
/// one name the new: its clones' git files (`main/<repo>`,
/// `sessions/<name>/<repo>`), the notes Humata keeps in a session's
/// `.humata`, and every text file of a Desk session's folder, whose notes
/// can name the sessions they look after. The people's own working files
/// are left as they are.
pub(crate) fn rewrite_folders(placement: &Placement) -> Result<(), MoveError> {
    if placement.old_path == placement.new_path {
        return Ok(());
    }
    let channel = &placement.new_path;
    let known = known_folders(placement)?;
    for repo in folders_in(&channel.join("main"))? {
        rewrite_git(placement, &known, &repo)?;
    }
    for session in folders_in(&channel.join("sessions"))? {
        rewrite_tree(placement, &known, &session.join(".humata"), false, 0)?;
        for clone in folders_in(&session)? {
            rewrite_git(placement, &known, &clone)?;
        }
    }
    for desk in folders_in(&channel.join("desk"))? {
        rewrite_tree(placement, &known, &desk, false, 0)?;
    }
    Ok(())
}

/// The folder an agent keeps beside a conversation file of the same name
/// (Claude's `<id>/`), if there is one.
fn sibling(file: &Path) -> Option<PathBuf> {
    let name = file.file_stem()?;
    let folder = file.parent()?.join(name);
    fs::symlink_metadata(&folder).ok()?.is_dir().then_some(folder)
}

/// Moves each conversation file `from` to `to`, naming the new folder in
/// it, and removes the key's folder it leaves empty. One already moved
/// (`to` is there) is only removed from `from`; one that isn't there is
/// skipped. A file too big to search is moved as it is.
pub(crate) fn carry_conversations(placement: &Placement, moves: &[(PathBuf, PathBuf)]) -> Result<(), MoveError> {
    let known = known_folders(placement)?;
    for (from, to) in moves {
        if from == to {
            continue;
        }
        let meta = match fs::symlink_metadata(from) {
            Ok(meta) if meta.is_file() => meta,
            Ok(_) => continue,
            Err(source) if source.kind() == ErrorKind::NotFound => continue,
            Err(source) => return Err(io("look at", from)(source)),
        };
        let beside = sibling(from);
        if fs::symlink_metadata(to).is_err() {
            if let Some(parent) = to.parent() {
                humata2_durable::create_dir_all(parent, 0o700).map_err(io("create", parent))?;
            }
            let text = if meta.len() > TEXT_MAX {
                None
            } else {
                let bytes = fs::read(from).map_err(io("read", from))?;
                String::from_utf8(bytes).ok()
            };
            match text.as_deref().and_then(|text| placement.rewritten_moved(text, &known, moves)).or(text) {
                Some(now) => write_atomic(to, now.as_bytes()).map_err(|source| MoveError::Write {
                    path: to.clone(),
                    source,
                })?,
                None => humata2_durable::rename(from, to).map_err(io("move", from))?,
            }
        }
        if let (Some(folder), Some(parent)) = (beside, to.parent())
            && let Some(name) = to.file_stem()
        {
            let target = parent.join(name);
            if fs::symlink_metadata(&target).is_err() {
                humata2_durable::rename(&folder, &target).map_err(io("move", &folder))?;
                rewrite_tree(placement, &known, &target, false, 0)?;
            }
        }
        match humata2_durable::remove_file(from) {
            Err(source) if source.kind() != ErrorKind::NotFound => return Err(io("remove", from)(source)),
            _ => {}
        }
        if let Some(parent) = from.parent() {
            // Only an empty folder goes; one that still holds something stays.
            let _ = fs::remove_dir(parent);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(old: &Path, new: &Path) -> Placement {
        Placement {
            name: "Billing".to_owned(),
            folder: "billing".to_owned(),
            old_path: old.to_owned(),
            new_path: new.to_owned(),
            now_unix_ms: 1,
        }
    }

    /// MV-16: a clone's git files, a Desk note and a session's own notes
    /// come to name the new folder, and a folder that only starts like the
    /// old one is left as it was.
    #[test]
    fn files_inside_the_moved_folder_come_to_name_the_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("payments"), dir.path().join("billing"));
        let clone = new.join("sessions/fix/api");
        fs::create_dir_all(clone.join(".git/logs")).unwrap();
        fs::create_dir_all(new.join("desk/plan")).unwrap();
        fs::create_dir_all(new.join("sessions/fix/.humata")).unwrap();
        let config = format!("[remote \"origin\"]\n\turl = {}/main/api\n", old.display());
        fs::write(clone.join(".git/config"), &config).unwrap();
        fs::write(clone.join(".git/logs/HEAD"), format!("clone: from {}/main/api\n", old.display())).unwrap();
        let other = format!("see {}-2/main/api\n", old.display());
        fs::write(clone.join(".git/FETCH_HEAD"), &other).unwrap();
        fs::write(new.join("desk/plan/notes.md"), format!("work is in {}/sessions/fix\n", old.display())).unwrap();
        fs::write(new.join("sessions/fix/.humata/note"), format!("{}/shared\n", old.display())).unwrap();

        rewrite_folders(&placement(&old, &new)).unwrap();

        let now = |path: &Path| fs::read_to_string(path).unwrap();
        assert!(now(&clone.join(".git/config")).contains(&format!("{}/main/api", new.display())));
        assert!(!now(&clone.join(".git/logs/HEAD")).contains(&old.display().to_string()));
        assert_eq!(now(&clone.join(".git/FETCH_HEAD")), other);
        assert_eq!(now(&new.join("desk/plan/notes.md")), format!("work is in {}/sessions/fix\n", new.display()));
        assert_eq!(now(&new.join("sessions/fix/.humata/note")), format!("{}/shared\n", new.display()));
    }

    /// MV-16: a conversation moves to the key of the new folder with the
    /// new folder written in it, and carrying it again changes nothing.
    #[test]
    fn a_conversation_moves_to_its_new_key_with_the_new_folder_in_it() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("payments"), dir.path().join("billing"));
        let from = dir.path().join("conv/key-old/c1.jsonl");
        let to = dir.path().join("conv/key-new/c1.jsonl");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        fs::write(&from, format!("{{\"cwd\":\"{}\"}}\nsaid\n", old.display())).unwrap();
        let moves = [(from.clone(), to.clone())];

        carry_conversations(&placement(&old, &new), &moves).unwrap();
        carry_conversations(&placement(&old, &new), &moves).unwrap();

        assert_eq!(fs::read_to_string(&to).unwrap(), format!("{{\"cwd\":\"{}\"}}\nsaid\n", new.display()));
        assert!(!from.exists());
        assert!(!from.parent().unwrap().exists(), "the empty key is gone");
    }

    /// A session folder whose key is cut at 200 units: its conversation
    /// comes to name the new folder's own key, and a sibling channel's key
    /// stays as it was.
    #[test]
    fn a_conversation_of_a_session_folder_with_a_long_key_names_the_exact_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("payments"), dir.path().join("billing"));
        let name = "d".repeat(220);
        fs::create_dir_all(new.join("sessions").join(&name)).unwrap();
        let key = |folder: &Path| humata2_conversation::claude_project_name(folder);
        let (was, now) = (key(&old.join("sessions").join(&name)), key(&new.join("sessions").join(&name)));
        let sibling = key(&dir.path().join("payments-2/sessions").join(&name));
        assert!(was.len() > 200 && now.len() > 200 && was != now);
        let from = dir.path().join("conv").join(&was).join("c1.jsonl");
        let to = dir.path().join("conv").join(&now).join("c1.jsonl");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        fs::write(&from, format!("{{\"project\":\"{was}\"}}\n{{\"other\":\"{sibling}\"}}\n")).unwrap();

        carry_conversations(&placement(&old, &new), &[(from, to.clone())]).unwrap();

        let want = format!("{{\"project\":\"{now}\"}}\n{{\"other\":\"{sibling}\"}}\n");
        assert_eq!(fs::read_to_string(&to).unwrap(), want);
    }

    /// A working folder two levels under a clone with a cut key: its
    /// conversation comes to name the new folder's own key, another
    /// channel's key stays as it was.
    #[test]
    fn a_conversation_of_a_folder_two_levels_under_a_clone_names_the_exact_new_key() {
        let dir = tempfile::tempdir().unwrap();
        let (old, new) = (dir.path().join("payments"), dir.path().join("billing"));
        let rest = format!("sessions/fix/api/src/{}", "d".repeat(220));
        let key = |folder: &Path| humata2_conversation::claude_project_name(folder);
        let (was, now) = (key(&old.join(&rest)), key(&new.join(&rest)));
        let sibling = key(&dir.path().join("payments-2").join(&rest));
        assert!(was.len() > 200 && now.len() > 200 && was != now);
        let from = dir.path().join("conv").join(&was).join("c1.jsonl");
        let to = dir.path().join("conv").join(&now).join("c1.jsonl");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        fs::write(&from, format!("{{\"project\":\"{was}\"}}\n{{\"other\":\"{sibling}\"}}\n")).unwrap();

        carry_conversations(&placement(&old, &new), &[(from, to.clone())]).unwrap();

        let want = format!("{{\"project\":\"{now}\"}}\n{{\"other\":\"{sibling}\"}}\n");
        assert_eq!(fs::read_to_string(&to).unwrap(), want);
    }
}
