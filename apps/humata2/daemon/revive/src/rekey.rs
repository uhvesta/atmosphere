//! D801 D: a running agent keeps running when its channel's folder is
//! renamed. It keeps the folder and conversation key it started under (its
//! record is put back to them), and a note beside its holder's socket says
//! what to do before it is resumed: carry its conversation to the key the
//! channel's new folder has, name the new folder in it, and point its launch
//! at the new folder. [`apply`] does that at the start of every resume; the
//! note is removed once the resume's record is saved. Every step can run
//! again, so a crash anywhere leaves a note the next resume finishes.

use std::ffi::OsString;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use humata2_store::{AgentRecord, Placement, StoreError, write_atomic};
use serde::{Deserialize, Serialize};

use crate::ReviveConfig;

/// The biggest conversation file whose text is searched for the old folder.
const TEXT_MAX: u64 = 16 << 20;

/// What an agent's CLI is asked to do to follow its working folder: the
/// conversation `id` kept at `file` goes from `from` to `to`, staged in
/// `scratch` (`AgentKind::follow_folder`). Only kinds that record the folder
/// inside the conversation have any.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FollowStep {
    pub agent: String,
    pub id: String,
    pub file: PathBuf,
    pub from: PathBuf,
    pub to: PathBuf,
    pub home: PathBuf,
    pub scratch: PathBuf,
}

/// Why a conversation couldn't follow its agent's folder.
#[derive(Debug, thiserror::Error)]
#[error("{reason}")]
pub struct FollowFailed {
    /// What went wrong and how to fix it, as the agent's kind says it.
    pub reason: String,
}

/// What an agent's launch named when it started: the folder, the commands
/// and the conversation file, all under the channel's folder of that time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Started {
    pub folder: PathBuf,
    pub argv: Vec<OsString>,
    pub resume_argv: Vec<OsString>,
    pub conversation_file: PathBuf,
}

/// A pending re-key of one agent, kept at [`note_path`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RekeyNote {
    /// The channel's folder when the agent started.
    pub path: PathBuf,
    /// The channel's folder now. A later rename changes only this.
    pub target: PathBuf,
    pub started: Started,
    /// The folders under the channel whose conversation keys the text of a
    /// file can name, as they were under `path`.
    pub known: Vec<PathBuf>,
    pub follows: Vec<FollowStep>,
}

/// Why a pending re-key couldn't be read, kept or done.
#[derive(Debug, thiserror::Error)]
pub enum RekeyError {
    #[error(
        "couldn't read the note at {} of the conversation move that is waiting for this agent; check the file is readable by this user, then resume the session again",
        path.display()
    )]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "the note at {} of the conversation move that is waiting for this agent is damaged; remove it only if the channel's folder was never renamed under this agent, then resume the session again",
        path.display()
    )]
    Damaged {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error(
        "couldn't write the note at {} of the conversation move that waits for this agent; free the disk space or fix the permissions of that folder, then rename the channel again",
        path.display()
    )]
    Write {
        path: PathBuf,
        #[source]
        source: StoreError,
    },
    #[error(
        "couldn't move the agent's conversation from {} to {}; free the disk space or fix the permissions of those folders, then resume the session again",
        from.display(),
        to.display()
    )]
    Carry {
        from: PathBuf,
        to: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("conversation {id} couldn't follow its agent to the channel's new folder")]
    Follow {
        id: String,
        #[source]
        source: FollowFailed,
    },
    #[error("a background task stopped before it finished; restart humatad2 to try again")]
    Background {
        #[source]
        source: tokio::task::JoinError,
    },
}

/// Where the note of the agent whose holder listens at `socket` is kept.
pub fn note_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_owned();
    name.push(".rekey");
    PathBuf::from(name)
}

/// The note kept for `socket`, none when there is none.
pub fn read_note(socket: &Path) -> Result<Option<RekeyNote>, RekeyError> {
    let path = note_path(socket);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(RekeyError::Read { path, source }),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|source| RekeyError::Damaged { path, source })
}

/// Keeps `note` for `socket`, whole or not at all.
pub fn write_note(socket: &Path, note: &RekeyNote) -> Result<(), RekeyError> {
    let path = note_path(socket);
    let bytes = serde_json::to_vec(note).map_err(|source| RekeyError::Damaged { path: path.clone(), source })?;
    write_atomic(&path, &bytes).map_err(|source| RekeyError::Write { path, source })
}

/// Takes the note away; one that isn't there is already gone.
pub fn remove_note(socket: &Path) {
    let _ = fs::remove_file(note_path(socket));
}

impl RekeyNote {
    /// Whether the channel's folder was renamed under the agent: the new
    /// folder is there and the one it started under is not a folder of its
    /// own any more (gone, or the alias that points to the new one). A note
    /// of a rename that never happened is left alone.
    fn applies(&self) -> bool {
        let renamed = fs::metadata(&self.target).is_ok_and(|meta| meta.is_dir());
        let started_is_real = fs::symlink_metadata(&self.path).is_ok_and(|meta| meta.is_dir());
        renamed && !started_is_real && self.path != self.target
    }

    fn placement(&self) -> Placement {
        Placement {
            name: String::new(),
            folder: String::new(),
            old_path: self.path.clone(),
            new_path: self.target.clone(),
            now_unix_ms: 0,
        }
    }
}

/// `text` as the new folder names it, `text` itself when nothing changes.
fn moved(placement: &Placement, known: &[PathBuf], text: &str) -> Option<String> {
    placement.rewritten_among(text, known)
}

fn moved_path(placement: &Placement, known: &[PathBuf], path: &Path) -> PathBuf {
    path.to_str().and_then(|text| moved(placement, known, text)).map_or_else(|| path.to_owned(), PathBuf::from)
}

fn moved_args(placement: &Placement, known: &[PathBuf], args: &[OsString]) -> Vec<OsString> {
    args.iter()
        .map(|arg| arg.to_str().and_then(|text| moved(placement, known, text)).map_or_else(|| arg.clone(), OsString::from))
        .collect()
}

/// The folder an agent keeps beside a conversation file of the same name
/// (Claude's `<id>/`), if there is one.
fn sibling(file: &Path) -> Option<PathBuf> {
    let folder = file.parent()?.join(file.file_stem()?);
    fs::symlink_metadata(&folder).ok()?.is_dir().then_some(folder)
}

/// Moves the conversation `from` to `to`, naming the new folder in it, and
/// removes the key's folder it leaves empty. A conversation already moved
/// (`to` is there) is only removed from `from`; one that isn't there is
/// skipped.
fn carry(placement: &Placement, known: &[PathBuf], from: &Path, to: &Path) -> Result<(), RekeyError> {
    let failed = |source| RekeyError::Carry { from: from.to_owned(), to: to.to_owned(), source };
    if from == to {
        return Ok(());
    }
    let meta = match fs::symlink_metadata(from) {
        Ok(meta) if meta.is_file() => meta,
        Ok(_) => return Ok(()),
        Err(source) if source.kind() == ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(failed(source)),
    };
    let beside = sibling(from);
    if fs::symlink_metadata(to).is_err() {
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(failed)?;
        }
        let text = if meta.len() > TEXT_MAX {
            None
        } else {
            String::from_utf8(fs::read(from).map_err(failed)?).ok()
        };
        match text {
            Some(text) => {
                let moves = [(from.to_owned(), to.to_owned())];
                let now = placement.rewritten_moved(&text, known, &moves).unwrap_or(text);
                write_atomic(to, now.as_bytes()).map_err(|source| failed(std::io::Error::other(source)))?;
            }
            None => fs::rename(from, to).map_err(failed)?,
        }
    }
    if let (Some(folder), Some(parent), Some(name)) = (beside, to.parent(), to.file_stem()) {
        let target = parent.join(name);
        if fs::symlink_metadata(&target).is_err() {
            fs::rename(&folder, &target).map_err(failed)?;
        }
    }
    match fs::remove_file(from) {
        Err(source) if source.kind() != ErrorKind::NotFound => return Err(failed(source)),
        _ => {}
    }
    if let Some(parent) = from.parent() {
        // Only an empty folder goes; one that still holds something stays.
        let _ = fs::remove_dir(parent);
    }
    Ok(())
}

/// Does what the note of `record`'s agent says, if it has one that applies:
/// carries the conversation to the new folder's key, has the agent's kind
/// follow the folder, and points `record`'s launch at the new folder.
/// Answers whether it did, so the caller removes the note once `record` is
/// saved. The note is left in place on an error.
pub(crate) async fn apply(config: &ReviveConfig, record: &mut AgentRecord) -> Result<bool, RekeyError> {
    let socket = record.launch.holder_socket.clone();
    let Some(note) = tokio::task::spawn_blocking({
        let socket = socket.clone();
        move || read_note(&socket)
    })
    .await
    .map_err(|source| RekeyError::Background { source })??
    else {
        return Ok(false);
    };
    if !note.applies() {
        return Ok(false);
    }
    let placement = note.placement();
    let (known, from) = (note.known.clone(), note.started.conversation_file.clone());
    let to = moved_path(&placement, &known, &from);
    tokio::task::spawn_blocking({
        let (placement, known, from, to) = (placement.clone(), known.clone(), from, to);
        move || carry(&placement, &known, &from, &to)
    })
    .await
    .map_err(|source| RekeyError::Background { source })??;
    if let Some(versions) = &config.versions {
        for step in &note.follows {
            versions.follow(step).await.map_err(|source| RekeyError::Follow { id: step.id.clone(), source })?;
        }
    }
    let started = &note.started;
    record.launch.folder = moved_path(&placement, &known, &started.folder);
    record.launch.conversation_file = moved_path(&placement, &known, &started.conversation_file);
    record.launch.argv = moved_args(&placement, &known, &started.argv);
    record.launch.resume_argv = moved_args(&placement, &known, &started.resume_argv);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(dir: &Path) -> RekeyNote {
        RekeyNote {
            path: dir.join("payments"),
            target: dir.join("billing"),
            started: Started {
                folder: dir.join("payments/sessions/fix"),
                argv: Vec::new(),
                resume_argv: vec!["--resume".into()],
                conversation_file: dir.join("conv/key-old/c1.jsonl"),
            },
            known: Vec::new(),
            follows: Vec::new(),
        }
    }

    /// The note is whole after a write and read back as it was.
    #[test]
    fn a_note_reads_back_as_it_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("h.sock");
        let wanted = note(dir.path());
        assert_eq!(read_note(&socket).unwrap(), None);
        write_note(&socket, &wanted).unwrap();
        assert_eq!(read_note(&socket).unwrap(), Some(wanted));
        remove_note(&socket);
        assert_eq!(read_note(&socket).unwrap(), None);
    }

    /// A note applies once the new folder is there and the started-under one
    /// is gone or only a link; never while the old folder is still a folder.
    #[test]
    fn a_note_applies_only_after_the_folder_was_renamed() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = note(dir.path());
        fs::create_dir(&wanted.path).unwrap();
        assert!(!wanted.applies(), "nothing was renamed");
        fs::rename(&wanted.path, &wanted.target).unwrap();
        assert!(wanted.applies(), "renamed, the old name is gone");
        std::os::unix::fs::symlink(&wanted.target, &wanted.path).unwrap();
        assert!(wanted.applies(), "renamed, the old name is a link to the new one");
    }

    /// The conversation moves to the new key with the new folder in it, and
    /// moving it again changes nothing.
    #[test]
    fn a_conversation_moves_to_the_new_folders_key() {
        let dir = tempfile::tempdir().unwrap();
        let wanted = note(dir.path());
        let from = &wanted.started.conversation_file;
        let to = dir.path().join("conv/key-new/c1.jsonl");
        fs::create_dir_all(from.parent().unwrap()).unwrap();
        fs::write(from, format!("{{\"cwd\":\"{}\"}}\nsaid\n", wanted.path.display())).unwrap();
        let placement = wanted.placement();

        carry(&placement, &[], from, &to).unwrap();
        carry(&placement, &[], from, &to).unwrap();

        let text = fs::read_to_string(&to).unwrap();
        assert_eq!(text, format!("{{\"cwd\":\"{}\"}}\nsaid\n", wanted.target.display()));
        assert!(!from.exists());
    }
}
