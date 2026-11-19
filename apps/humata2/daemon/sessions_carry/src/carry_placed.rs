//! A conversation written for the machine it is going to (MV-16, MV-17).
//!
//! A move changes a session's folder, so the conversation names a path the
//! new machine doesn't have. Rewriting that path on arrival, as unpacking
//! does otherwise, makes the bytes the old machine sends differ from the
//! ones the new machine holds of an earlier copy of the conversation, and no
//! chunk matches. So the old machine asks the new one where the folder will
//! be ([`Starter::carry_target`]), has the conversation's adapter place the
//! staged files for it, and chunks the placed bytes. The manifest records
//! the folder they were placed for, and unpacking places from there, which
//! rewrites none of the folder's paths a second time.
//!
//! The adapter writes into a CLI home, so it writes into a scratch one under
//! the operation's folder; the old home and config dir it put there are
//! named back as they were, so unpacking, which knows the new machine's
//! home, still rewrites them. Only Claude's conversations are placed this
//! way for now.

use std::fs;
use std::path::{Path, PathBuf};

use humata2_conversation::{ConversationError, Format, Packed, Placement, adapter};

use super::{AGENT, AgentFiles, CarryError, io};
use humata2_sessions_core::agents::AgentKind;
use humata2_sessions_core::error::StartError;
use humata2_sessions_core::fork::{Through, fork_conversation};
use humata2_sessions_core::start::Starter;

/// The folder, under an operation's own, the placing is done in.
pub(super) const SCRATCH: &str = "placing";

/// What [`place_staged`] needs to know.
pub(super) struct Placing<'a> {
    /// The agent's name, for what is said when its adapter refuses.
    pub(super) agent: &'a str,
    /// Where the session's folder is on this machine.
    pub(super) from_folder: &'a Path,
    /// Where it will be on the machine it goes to.
    pub(super) target: &'a Path,
    /// The staged files; the agent's are under `agent/`.
    pub(super) stage: &'a Path,
    /// Where the adapter writes while it places.
    pub(super) scratch: PathBuf,
}

impl crate::CarryOps<'_> {
    /// Where the session folder `rel` of `channel` will be on this machine,
    /// as the agent's CLI will record it once the folder is there: the
    /// nearest folder that exists is resolved (a symlinked parent, macOS's
    /// `/var`), and the rest follows it.
    pub fn carry_target(&self, channel: &str, rel: &str) -> Result<PathBuf, CarryError> {
        let channel = self.carry_channel(channel)?;
        let folder = self.carried_folder(&channel, rel)?;
        Ok(resolved(&folder))
    }
}

/// `folder` with its nearest existing ancestor resolved.
fn resolved(folder: &Path) -> PathBuf {
    let mut missing = Vec::new();
    let mut at = folder;
    loop {
        if let Ok(real) = fs::canonicalize(at) {
            return missing.iter().rev().fold(real, |path, name| path.join(name));
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => {
                missing.push(name);
                at = parent;
            }
            _ => return folder.to_owned(),
        }
    }
}

/// Writes the conversation staged under `placing.stage` for `placing.target`
/// in place, and records in `files` the folder it was written for. Files
/// that aren't a Claude conversation are left as they are.
pub(super) fn place_staged(placing: &Placing<'_>, mut files: AgentFiles) -> Result<AgentFiles, CarryError> {
    if let AgentFiles::Conversation { packed, old_home, old_cli_home, placed_for, .. } = &mut files
        && packed.format == Format::ClaudeJsonl
    {
        let from = Placement {
            home: old_home.clone(),
            cli_home: old_cli_home.clone(),
            folder: placing.from_folder.to_owned(),
        };
        write_placed(placing, packed, &from)?;
        *placed_for = Some(placing.target.to_owned());
    }
    Ok(files)
}

fn write_placed(placing: &Placing<'_>, packed: &mut Packed, from: &Placement) -> Result<(), CarryError> {
    let refused = |source: ConversationError| CarryError::Conversation { agent: placing.agent.to_owned(), source };
    let agent_dir = placing.stage.join(AGENT);
    clear(&placing.scratch)?;
    let to = Placement {
        home: placing.scratch.join("home"),
        cli_home: from.cli_home.as_ref().map(|_| placing.scratch.join("config")),
        folder: placing.target.to_owned(),
    };
    let staged = Packed { stage: agent_dir.clone(), ..packed.clone() };
    let placed = adapter(staged.format).place(&staged, from, &to).map_err(refused)?;
    let back = names_back(from, &to);
    // The adapter lists the placed files in the order of the packed ones.
    for (file, written) in packed.files.iter_mut().zip(&placed.files) {
        let bytes = fs::read(&written.path).map_err(io("read", &written.path))?;
        let restored = std::str::from_utf8(&bytes).ok().map(|text| restore(text, &back));
        let bytes = restored.unwrap_or(bytes);
        let path = agent_dir.join(&file.rel);
        fs::write(&path, &bytes).map_err(io("replace", &path))?;
        file.size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    }
    clear(&placing.scratch)
}

/// A fork's transcript as it is staged for another machine (FK-8): the
/// parent's conversation at its recorded file forked through its adapter, as a fork
/// on this machine is, so it names the fork's id `new_id` wherever the CLI
/// keeps one and holds the turns before `before_turn` (0: all of them). The
/// adapter writes into a CLI home, so it writes into `scratch`, which is
/// taken away again; the old home and config dir it put there are named back
/// as they were, so the other machine's placing rewrites them. `None` for a
/// kind no adapter knows, which the caller copies as it is; a kind with an
/// adapter whose file the adapter doesn't find is refused.
pub(super) fn forked_for_stage(
    kind: &AgentKind,
    home: &Path,
    folder: &Path,
    parent: &humata2_store::AgentLaunch,
    new_id: &str,
    before_turn: u32,
    scratch: &Path,
) -> Result<Option<Vec<u8>>, StartError> {
    let Some(from) = kind.placement(home, folder) else {
        return Ok(None);
    };
    let through = Through {
        format: kind.conversation_format,
        from: from.clone(),
        from_id: parent.conversation_id.clone(),
        to: Placement {
            home: scratch.join("home"),
            cli_home: from.cli_home.as_ref().map(|_| scratch.join("config")),
            folder: from.folder.clone(),
        },
        to_id: new_id.to_owned(),
    };
    if !through.keeps(&parent.conversation_file) {
        // Its CLI has a format here, so copying the file as it is would carry
        // the parent's id: refuse rather than copy.
        return Err(StartError::Conversation {
            id: parent.conversation_id.clone(),
            source: ConversationError::NotFound { path: parent.conversation_file.clone() },
        });
    }
    // A leftover of an earlier try would make the fork refuse as already there.
    clear_scratch(scratch)?;
    let written = fork_conversation(
        through.format,
        &through.from,
        &through.from_id,
        &through.to,
        &through.to_id,
        before_turn,
    )
    .and_then(|path| fs::read(&path).map_err(|source| StartError::Copy { path, source }));
    // The fork's own refusal is the one to report; a scratch that can't be
    // taken away is reported when the fork itself worked.
    let cleared = clear_scratch(scratch);
    let bytes = written?;
    cleared?;
    let back = names_back(&through.from, &through.to);
    let restored = std::str::from_utf8(&bytes).ok().map(|text| restore(text, &back));
    Ok(Some(restored.unwrap_or(bytes)))
}

/// Takes `path` and what is under it away; none of it there is fine.
fn clear_scratch(path: &Path) -> Result<(), StartError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StartError::Copy { path: path.to_owned(), source }),
    }
}

/// Removes `path` and what is under it; none of it is there is fine.
fn clear(path: &Path) -> Result<(), CarryError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(io("clear", path)(source)),
    }
}

/// Claude's config dir at `at`.
fn config_dir(at: &Placement) -> PathBuf {
    at.cli_home.clone().unwrap_or_else(|| at.home.join(".claude"))
}

/// The text of `path` as JSON writes it inside a string.
fn json_text(text: &str) -> String {
    let quoted = serde_json::to_string(text).unwrap_or_default();
    quoted
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_default()
        .to_owned()
}

/// What the placing wrote in the scratch home and config dir, and what to
/// name there instead: the old ones, so the new machine's own placing finds
/// and rewrites them. The config dir comes first, as it may lie in the home.
fn names_back(from: &Placement, to: &Placement) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for (written, kept) in [(config_dir(to), config_dir(from)), (to.home.clone(), from.home.clone())] {
        let (written, kept) = (written.to_string_lossy().into_owned(), kept.to_string_lossy().into_owned());
        let escaped = (json_text(&written), json_text(&kept));
        pairs.push((written, kept));
        pairs.push(escaped);
    }
    pairs.retain(|(written, kept)| written != kept && !written.is_empty());
    pairs.dedup();
    pairs
}

fn restore(text: &str, back: &[(String, String)]) -> Vec<u8> {
    let mut now = text.to_owned();
    for (written, kept) in back {
        now = now.replace(written.as_str(), kept);
    }
    now.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_that_isnt_there_yet_follows_its_nearest_real_parent() {
        let root = tempfile::tempdir().unwrap();
        let real = root.path().join("real");
        fs::create_dir_all(&real).unwrap();
        std::os::unix::fs::symlink(&real, root.path().join("link")).unwrap();
        let planned = resolved(&root.path().join("link/sessions/fix"));
        assert_eq!(planned, fs::canonicalize(&real).unwrap().join("sessions/fix"));
    }

    #[test]
    fn the_scratch_home_and_config_dir_are_named_back_as_the_old_ones() {
        let from = Placement { home: "/old/home".into(), cli_home: None, folder: "/old/work".into() };
        let to = Placement { home: "/scratch/home".into(), cli_home: None, folder: "/new/work".into() };
        let back = names_back(&from, &to);
        let text = r#"{"a":"/scratch/home/.claude/plans/x.md","b":"/scratch/home/notes"}"#;
        let restored = String::from_utf8(restore(text, &back)).unwrap();
        assert_eq!(restored, r#"{"a":"/old/home/.claude/plans/x.md","b":"/old/home/notes"}"#);
    }
}
