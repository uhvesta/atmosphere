use std::fs;
use std::path::{Path, PathBuf};

use crate::error::LayoutError;

/// Where a machine keeps the channels: `<root>/<channel>/{main,sessions,shared}` (FC-1).
#[derive(Clone, Debug)]
pub struct Layout {
    pub root: PathBuf,
}

impl Layout {
    pub fn channel(&self, folder: &str) -> PathBuf {
        self.root.join(folder)
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

fn create_private(dir: &Path) -> Result<(), LayoutError> {
    let err = |source| LayoutError::Create {
        path: dir.to_path_buf(),
        source,
    };
    fs::create_dir_all(dir).map_err(err)?;
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
        let l = Layout { root: "/h".into() };
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
        let l = Layout {
            root: t.path().join("Humata"),
        };
        l.create_channel("ch").unwrap();
        l.create_channel("ch").unwrap();
        for d in [l.main("ch"), l.sessions("ch"), l.shared("ch")] {
            assert!(d.is_dir());
        }
    }
}
