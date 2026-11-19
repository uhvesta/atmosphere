//! Showing a place on This Mac in Finder (PL-10). A place on another machine
//! is not revealed; it opens a terminal there instead.

use std::path::Path;
use std::process::Command;

use crate::model::{Machine, MachineKind};

/// Why Finder could not be asked to show a place.
#[derive(Debug, thiserror::Error)]
pub enum RevealError {
    /// `open` did not start.
    #[error(
        "Finder was not asked to show {path}, because the `open` command could not start. Check that /usr/bin/open exists on this Mac, then try again"
    )]
    Spawn {
        /// The place.
        path: String,
        /// Why `open` did not start.
        #[source]
        source: std::io::Error,
    },
    /// `open` started and refused.
    #[error(
        "Finder could not show {path}: `open -R` ended with {status}. Check that the file or folder still exists, then try again"
    )]
    Refused {
        /// The place.
        path: String,
        /// How `open` ended, as its exit status reads.
        status: String,
    },
    /// The path starts with `~` and this Mac's home folder is not known.
    #[error(
        "Finder was not asked to show {path}, because this Mac's home folder is not known. Start Humata from a user session where HOME is set, or copy the path and open it yourself"
    )]
    NoHome {
        /// The place.
        path: String,
    },
    /// The file or folder to show could not be made.
    #[error(
        "{path} could not be made, so it was not opened. Check that its folder can be written by you (for example with `ls -ld` on it), then try again"
    )]
    Create {
        /// The place.
        path: String,
        /// Why it could not be made.
        #[source]
        source: std::io::Error,
    },
}

/// Where Reveal in Finder goes.
pub trait Reveal: Send + Sync + 'static {
    /// Shows `path` selected in Finder.
    fn reveal(&self, path: &str) -> Result<(), RevealError>;

    /// Opens the file at `path` in the owner's text editor, making it from
    /// `starter` first when it isn't there. A file that is there is never
    /// changed. Used for files the owner writes, such as the machine scripts
    /// rules (MACHINESCRIPTS, D680).
    fn edit(&self, path: &str, _starter: &str) -> Result<(), RevealError> {
        Err(RevealError::Refused {
            path: path.to_owned(),
            status: "this Mac's Finder client can't open files in an editor".to_owned(),
        })
    }

    /// Shows the folder at `path` in Finder, making it first when it isn't
    /// there, so the owner has somewhere to put what it holds.
    fn reveal_folder(&self, path: &str) -> Result<(), RevealError> {
        self.reveal(path)
    }
}

/// Whether `machine` is the Mac the app runs on: it has Reveal in Finder;
/// every other machine has Open terminal here.
#[must_use]
pub fn is_this_mac(machine: &Machine) -> bool {
    matches!(machine.kind, MachineKind::ThisMac)
}

/// The real Finder: runs `open -R <path>`.
#[derive(Clone, Debug)]
pub struct FinderReveal {
    home: Option<String>,
}

impl FinderReveal {
    /// Finder on this Mac, expanding `~` to `$HOME`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            home: std::env::var("HOME").ok().filter(|home| !home.is_empty()),
        }
    }

    /// Finder that expands `~` to `home`.
    #[must_use]
    pub fn with_home(home: Option<String>) -> Self {
        Self { home }
    }

    /// The path `open -R` is given: `path`, with a leading `~` made the home folder.
    pub fn target(&self, path: &str) -> Result<String, RevealError> {
        let rest = if path == "~" {
            Some("")
        } else {
            path.strip_prefix("~/")
        };
        let Some(rest) = rest else {
            return Ok(path.to_owned());
        };
        let Some(home) = &self.home else {
            return Err(RevealError::NoHome { path: path.to_owned() });
        };
        if rest.is_empty() {
            // `Path::join("")` would add a trailing slash.
            return Ok(home.clone());
        }
        Ok(Path::new(home).join(rest).display().to_string())
    }
}

impl Default for FinderReveal {
    fn default() -> Self {
        Self::new()
    }
}

impl Reveal for FinderReveal {
    fn reveal(&self, path: &str) -> Result<(), RevealError> {
        let target = self.target(path)?;
        let status = Command::new("open")
            .arg("-R")
            .arg(&target)
            .status()
            .map_err(|source| RevealError::Spawn {
                path: path.to_owned(),
                source,
            })?;
        if status.success() {
            Ok(())
        } else {
            Err(RevealError::Refused {
                path: path.to_owned(),
                status: status.to_string(),
            })
        }
    }

    fn edit(&self, path: &str, starter: &str) -> Result<(), RevealError> {
        let target = self.target(path)?;
        make_file_if_missing(Path::new(&target), starter)
            .map_err(|source| RevealError::Create { path: path.to_owned(), source })?;
        open_with(&["-t"], &target, path)
    }

    fn reveal_folder(&self, path: &str) -> Result<(), RevealError> {
        let target = self.target(path)?;
        std::fs::create_dir_all(&target)
            .map_err(|source| RevealError::Create { path: path.to_owned(), source })?;
        open_with(&[], &target, path)
    }
}

/// Runs `open` with `flags` on `target`; `path` is what errors call it.
fn open_with(flags: &[&str], target: &str, path: &str) -> Result<(), RevealError> {
    let status = Command::new("open")
        .args(flags)
        .arg(target)
        .status()
        .map_err(|source| RevealError::Spawn { path: path.to_owned(), source })?;
    if status.success() {
        Ok(())
    } else {
        Err(RevealError::Refused { path: path.to_owned(), status: status.to_string() })
    }
}

/// Makes `path` with `starter` in it, and its folder, unless it is there
/// already. A file that is there is left exactly as it is.
fn make_file_if_missing(path: &Path, starter: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(starter.as_bytes()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let base = std::env::var_os("TEST_TMPDIR").map_or_else(std::env::temp_dir, std::path::PathBuf::from);
        base.join(format!("humata2-reveal-{}-{name}", std::process::id()))
    }

    #[test]
    fn a_missing_file_is_made_from_the_starter_with_its_folders() {
        let folder = scratch("missing");
        let file = folder.join("config").join("rules.toml");
        make_file_if_missing(&file, "# starter\n").expect("it is made");
        assert_eq!(std::fs::read_to_string(&file).ok().as_deref(), Some("# starter\n"));
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn a_file_the_owner_wrote_is_never_changed() {
        let folder = scratch("present");
        std::fs::create_dir_all(&folder).expect("the folder is made");
        let file = folder.join("rules.toml");
        std::fs::write(&file, "mine\n").expect("the file is written");
        make_file_if_missing(&file, "# starter\n").expect("nothing to do");
        assert_eq!(std::fs::read_to_string(&file).ok().as_deref(), Some("mine\n"));
        let _ = std::fs::remove_dir_all(&folder);
    }
}
