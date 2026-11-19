use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock};

use crate::error::LayoutError;

/// Where a machine keeps the channels: `<root>/<channel>/{main,sessions,shared}` (FC-1).
///
/// The root can change while the daemon runs (FC-10): clones of a `Layout`
/// share it, so every service sees the new one at once. A channel's recorded
/// folder is a name under the root, so channels that stayed where they were
/// when the root changed for new channels only are found under the older
/// roots, which are kept for that.
#[derive(Clone, Debug)]
pub struct Layout {
    places: Arc<RwLock<Places>>,
}

#[derive(Debug)]
struct Places {
    root: PathBuf,
    /// Roots that held channels before the root changed, newest first.
    older: Vec<PathBuf>,
}

impl From<PathBuf> for Layout {
    fn from(root: PathBuf) -> Self {
        Self::new(root)
    }
}

impl Layout {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            places: Arc::new(RwLock::new(Places {
                root: root.into(),
                older: Vec::new(),
            })),
        }
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Places> {
        self.places.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Places> {
        self.places.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// The folder new channels are made in.
    pub fn root(&self) -> PathBuf {
        self.read().root.clone()
    }

    /// Makes `root` the one every holder of this layout uses from now on.
    pub fn set_root(&self, root: PathBuf) {
        self.write().root = root;
    }

    /// The roots channels were left in when the root changed.
    pub fn older_roots(&self) -> Vec<PathBuf> {
        self.read().older.clone()
    }

    /// Sets the roots channels were left in, newest first.
    pub fn set_older_roots(&self, older: Vec<PathBuf>) {
        self.write().older = older;
    }

    /// Whether `dir` is the root or one of the older roots.
    pub fn is_a_root(&self, dir: &Path) -> bool {
        let places = self.read();
        places.root == dir || places.older.iter().any(|older| older == dir)
    }

    /// Where `folder` is: under the root, else under the newest older root
    /// that has it, else where it would be made, under the root.
    pub fn channel(&self, folder: &str) -> PathBuf {
        let places = self.read();
        let here = places.root.join(folder);
        if fs::symlink_metadata(&here).is_ok() {
            return here;
        }
        places
            .older
            .iter()
            .map(|older| older.join(folder))
            .find(|path| fs::symlink_metadata(path).is_ok())
            .unwrap_or(here)
    }

    pub fn main(&self, folder: &str) -> PathBuf {
        self.channel(folder).join("main")
    }

    pub fn main_clone(&self, folder: &str, repo: &str) -> PathBuf {
        self.main(folder).join(repo)
    }

    pub fn sessions(&self, folder: &str) -> PathBuf {
        self.channel(folder).join("sessions")
    }

    pub fn session(&self, folder: &str, session_folder: &str) -> PathBuf {
        self.sessions(folder).join(session_folder)
    }

    pub fn shared(&self, folder: &str) -> PathBuf {
        self.channel(folder).join("shared")
    }

    /// Makes `main/`, `sessions/` and `shared/`; already existing is fine.
    pub fn create_channel(&self, folder: &str) -> Result<(), LayoutError> {
        for dir in [
            self.main(folder),
            self.sessions(folder),
            self.shared(folder),
        ] {
            create_private(&dir)?;
        }
        Ok(())
    }
}

/// The sibling a clone is made in before it is renamed into place.
pub fn partial(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_os_string();
    name.push(".partial");
    PathBuf::from(name)
}

/// Makes `path` and its missing parents with `mode`, and puts each new
/// folder's entry on disk, so a power cut right after can't lose them.
pub fn make_dirs(path: &Path, mode: u32) -> std::io::Result<()> {
    let missing: Vec<&Path> = path
        .ancestors()
        .take_while(|folder| !folder.as_os_str().is_empty() && fs::symlink_metadata(folder).is_err())
        .collect();
    humata2_durable::create_dir_all(path, mode)?;
    for folder in missing {
        match folder.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => humata2_durable::sync_dir(parent)?,
            _ => {}
        }
    }
    Ok(())
}

fn create_private(dir: &Path) -> Result<(), LayoutError> {
    let err = |source| LayoutError::Create {
        path: dir.to_path_buf(),
        source,
    };
    make_dirs(dir, 0o700).map_err(err)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_follow_the_tree() {
        let l = Layout::new("/h");
        assert_eq!(l.main_clone("ch", "api"), Path::new("/h/ch/main/api"));
        assert_eq!(l.session("ch", "s1"), Path::new("/h/ch/sessions/s1"));
        assert_eq!(l.shared("ch"), Path::new("/h/ch/shared"));
        assert_eq!(
            partial(Path::new("/h/ch/main/api")),
            Path::new("/h/ch/main/api.partial")
        );
    }

    #[test]
    fn create_channel_makes_three_folders_and_is_repeatable() {
        let t = tempfile::tempdir().unwrap();
        let l = Layout::new(t.path().join("Humata"));
        l.create_channel("ch").unwrap();
        l.create_channel("ch").unwrap();
        for d in [l.main("ch"), l.sessions("ch"), l.shared("ch")] {
            assert!(d.is_dir());
        }
    }

    #[test]
    fn a_channel_left_under_an_older_root_is_still_found_there() {
        let t = tempfile::tempdir().unwrap();
        let (old, new) = (t.path().join("old"), t.path().join("new"));
        fs::create_dir_all(old.join("kept")).unwrap();
        let l = Layout::new(&old);
        let shared = l.clone();
        shared.set_older_roots(vec![old.clone()]);
        shared.set_root(new.clone());
        assert_eq!(l.root(), new, "a clone sees the new root");
        assert_eq!(l.channel("kept"), old.join("kept"));
        assert_eq!(l.channel("fresh"), new.join("fresh"));
        assert!(l.is_a_root(&old) && l.is_a_root(&new));
    }
}
