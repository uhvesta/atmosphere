//! The Mac's `humata` and `humatad`, linked into `~/.local/bin` so a
//! terminal finds them. Only links are made or replaced: a file of either
//! name that isn't a link is someone else's, and stays. No shell profile is
//! changed; whether the folder is on the PATH is only reported.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use humata_wire::pb;

/// The commands linked, each from the folder humatad runs from.
const COMMANDS: [&str; 2] = ["humata", "humatad"];

#[derive(Debug, thiserror::Error)]
pub enum CommandsError {
    #[error(
        "couldn't find the folder humatad runs from, to link its commands; open Humata from the Applications folder and try again"
    )]
    OwnPath {
        #[source]
        source: std::io::Error,
    },

    #[error(
        "there's no home folder to link the commands into, as HOME isn't set; start Humata from your own account"
    )]
    NoHome,

    #[error(
        "couldn't create {folder} to link the commands into; check that your home folder is writable, or create the folder yourself"
    )]
    CreateFolder {
        folder: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "couldn't link {link} to {target}; check that {} is writable. Removing what's there by hand is only safe if it's a link you made",
        link.parent().map(Path::display).map(|folder| folder.to_string()).unwrap_or_default()
    )]
    Link {
        link: PathBuf,
        target: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Links this humatad's commands into `~/.local/bin`, or with `check` only
/// says how they're linked.
pub fn link_commands(check: bool) -> Result<pb::CommandsLinked, CommandsError> {
    let own = std::env::current_exe().map_err(|source| CommandsError::OwnPath { source })?;
    let from = own.parent().map(Path::to_path_buf).unwrap_or_default();
    let home = std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .ok_or(CommandsError::NoHome)?;
    let path = std::env::var_os("PATH").unwrap_or_default();
    link(&from, &Path::new(&home).join(".local/bin"), &path, check)
}

/// Links each command in `from` into `folder`, unless `check`, and says how
/// each is linked and whether `folder` is on `path`.
fn link(
    from: &Path,
    folder: &Path,
    path: &OsStr,
    check: bool,
) -> Result<pb::CommandsLinked, CommandsError> {
    let mut links = Vec::new();
    for name in COMMANDS {
        let target = from.join(name);
        if !target.is_file() {
            continue;
        }
        let link = folder.join(name);
        let mut state = state(&link, &target);
        if !check
            && matches!(
                state,
                pb::CommandLinkState::Missing | pb::CommandLinkState::Stale
            )
        {
            make_link(folder, &link, &target)?;
            state = pb::CommandLinkState::Linked;
        }
        links.push(pb::CommandLink {
            name: name.to_owned(),
            state: state.into(),
        });
    }
    Ok(pb::CommandsLinked {
        folder: folder.display().to_string(),
        links,
        on_path: std::env::split_paths(path).any(|entry| entry == folder),
    })
}

fn state(link: &Path, target: &Path) -> pb::CommandLinkState {
    match std::fs::symlink_metadata(link) {
        Err(_) => pb::CommandLinkState::Missing,
        Ok(metadata) if !metadata.file_type().is_symlink() => pb::CommandLinkState::Other,
        Ok(_) => match std::fs::read_link(link) {
            Ok(to) if to == target => pb::CommandLinkState::Linked,
            // A link to another humata is an older copy of the app's;
            // a link to anything else of the name is someone else's.
            Ok(to) if to.file_name() == target.file_name() => pb::CommandLinkState::Stale,
            _ => pb::CommandLinkState::Other,
        },
    }
}

/// Makes `link` point at `target`, replacing a link there through a rename
/// so no moment has neither.
fn make_link(folder: &Path, link: &Path, target: &Path) -> Result<(), CommandsError> {
    std::fs::create_dir_all(folder).map_err(|source| CommandsError::CreateFolder {
        folder: folder.to_path_buf(),
        source,
    })?;
    let failed = |source| CommandsError::Link {
        link: link.to_path_buf(),
        target: target.to_path_buf(),
        source,
    };
    let fresh = link.with_extension("humata-new");
    let _ = std::fs::remove_file(&fresh);
    std::os::unix::fs::symlink(target, &fresh).map_err(failed)?;
    std::fs::rename(&fresh, link).map_err(|source| {
        let _ = std::fs::remove_file(&fresh);
        failed(source)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn helpers(root: &Path) -> PathBuf {
        let from = root.join("Humata.app/Contents/Helpers");
        std::fs::create_dir_all(&from).unwrap_or_else(|error| panic!("{error}"));
        for name in COMMANDS {
            std::fs::write(from.join(name), b"").unwrap_or_else(|error| panic!("{error}"));
        }
        from
    }

    fn states(linked: &pb::CommandsLinked) -> Vec<pb::CommandLinkState> {
        linked.links.iter().map(|link| link.state()).collect()
    }

    #[test]
    fn both_commands_get_linked_and_a_check_changes_nothing() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let from = helpers(root.path());
        let folder = root.path().join("home/.local/bin");

        let checked = link(&from, &folder, OsStr::new("/usr/bin"), true)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(states(&checked), [pb::CommandLinkState::Missing; 2]);
        assert!(!folder.exists());

        let linked = link(&from, &folder, OsStr::new("/usr/bin"), false)
            .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(states(&linked), [pb::CommandLinkState::Linked; 2]);
        assert_eq!(
            std::fs::read_link(folder.join("humata")).ok(),
            Some(from.join("humata"))
        );
        assert!(!linked.on_path);
    }

    #[test]
    fn a_file_of_the_name_stays_and_an_old_apps_link_moves() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let from = helpers(root.path());
        let folder = root.path().join("bin");
        std::fs::create_dir_all(&folder).unwrap_or_else(|error| panic!("{error}"));
        std::fs::write(folder.join("humata"), b"mine").unwrap_or_else(|error| panic!("{error}"));
        std::os::unix::fs::symlink(root.path().join("Old.app/humatad"), folder.join("humatad"))
            .unwrap_or_else(|error| panic!("{error}"));
        let path = std::env::join_paths([Path::new("/usr/bin"), &folder])
            .unwrap_or_else(|error| panic!("{error}"));

        let checked = link(&from, &folder, &path, true).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            states(&checked),
            [pb::CommandLinkState::Other, pb::CommandLinkState::Stale]
        );

        let linked = link(&from, &folder, &path, false).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(
            states(&linked),
            [pb::CommandLinkState::Other, pb::CommandLinkState::Linked]
        );
        assert_eq!(
            std::fs::read(folder.join("humata")).ok(),
            Some(b"mine".to_vec())
        );
        assert_eq!(
            std::fs::read_link(folder.join("humatad")).ok(),
            Some(from.join("humatad"))
        );
        assert!(linked.on_path);
    }

    #[test]
    fn a_link_to_some_other_program_stays() {
        let root = tempfile::tempdir().unwrap_or_else(|error| panic!("{error}"));
        let from = helpers(root.path());
        let folder = root.path().join("bin");
        std::fs::create_dir_all(&folder).unwrap_or_else(|error| panic!("{error}"));
        std::os::unix::fs::symlink("/opt/tools/other", folder.join("humata"))
            .unwrap_or_else(|error| panic!("{error}"));

        let linked =
            link(&from, &folder, OsStr::new(""), false).unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(linked.links[0].state(), pb::CommandLinkState::Other);
        assert_eq!(
            std::fs::read_link(folder.join("humata")).ok(),
            Some(PathBuf::from("/opt/tools/other"))
        );
    }
}
