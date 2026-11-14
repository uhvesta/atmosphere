use std::path::{Component, Path, PathBuf};

/// `path` under `root`, if it stays there: relative, without `..`.
pub(crate) fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let stays = path
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    (stays && !path.as_os_str().is_empty()).then(|| root.join(path))
}

/// Why a path a model gave isn't used. The messages are written for the
/// model, which reads them as its tool's failed output.
#[derive(Debug, thiserror::Error)]
pub(crate) enum OutsideError {
    #[error(
        "{} is outside the project folder; use a relative path without `..`.",
        .path.display()
    )]
    NotRelative { path: PathBuf },

    #[error(
        "{} is, or is under, a link to somewhere outside the project folder, so it isn't read or written. Ask the person if you need what it leads to.",
        .path.display()
    )]
    LinkOut { path: PathBuf },

    #[error(
        "Couldn't tell where {} leads, so it isn't read or written; if it's a link whose target is gone, say so to the person",
        .path.display()
    )]
    Unresolved {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// `path` under `root`, as [`inside`] gives it, if what it names is under
/// `root` once links are followed too. For a file still to be made, its
/// nearest folder that exists is what has to be there.
pub(crate) async fn resolved_inside(root: &Path, path: &Path) -> Result<PathBuf, OutsideError> {
    let target = inside(root, path).ok_or_else(|| OutsideError::NotRelative {
        path: path.to_owned(),
    })?;
    let unresolved = |source| OutsideError::Unresolved {
        path: path.to_owned(),
        source,
    };
    let real_root = tokio::fs::canonicalize(root).await.map_err(unresolved)?;
    // The nearest of the target and the folders above it that is there. A
    // link whose target is gone is there, and fails below: a write through
    // it would make the file wherever it points.
    let mut nearest = target.as_path();
    while tokio::fs::symlink_metadata(nearest).await.is_err() {
        match nearest.parent() {
            Some(parent) if parent.starts_with(root) => nearest = parent,
            _ => break,
        }
    }
    let real = tokio::fs::canonicalize(nearest).await.map_err(unresolved)?;
    if real.starts_with(&real_root) {
        Ok(target)
    } else {
        Err(OutsideError::LinkOut {
            path: path.to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_outside_the_folder_are_refused() {
        let root = Path::new("/work");
        assert_eq!(
            inside(root, Path::new("a/b")),
            Some(PathBuf::from("/work/a/b"))
        );
        assert_eq!(inside(root, Path::new("../a")), None);
        assert_eq!(inside(root, Path::new("/etc/passwd")), None);
        assert_eq!(inside(root, Path::new("")), None);
    }

    #[tokio::test]
    async fn a_path_that_is_not_relative_is_told_apart_from_a_link_that_leads_out() {
        let root = tempfile::tempdir().expect("a temporary folder");
        let elsewhere = tempfile::tempdir().expect("a temporary folder");
        std::fs::write(elsewhere.path().join("secret"), "x").expect("a file");
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("out")).expect("a link");
        std::os::unix::fs::symlink(elsewhere.path().join("secret"), root.path().join("file"))
            .expect("a link");
        let resolved = |path: &'static str| {
            let root = root.path().to_owned();
            async move { resolved_inside(&root, Path::new(path)).await }
        };

        assert!(matches!(
            resolved("../up").await,
            Err(OutsideError::NotRelative { .. })
        ));
        assert!(matches!(
            resolved("/etc/passwd").await,
            Err(OutsideError::NotRelative { .. })
        ));
        // What's there, what would be made, and a link to one file.
        for path in ["out", "out/secret", "out/new/deeper.txt", "file"] {
            assert!(
                matches!(resolved(path).await, Err(OutsideError::LinkOut { .. })),
                "{path}"
            );
        }
    }

    #[tokio::test]
    async fn a_link_whose_target_is_gone_is_not_written_through() {
        let root = tempfile::tempdir().expect("a temporary folder");
        let elsewhere = tempfile::tempdir().expect("a temporary folder");
        std::os::unix::fs::symlink(elsewhere.path().join("absent"), root.path().join("gone"))
            .expect("a link");

        assert!(
            resolved_inside(root.path(), Path::new("gone"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn paths_that_stay_in_the_folder_are_given_under_it() {
        let root = tempfile::tempdir().expect("a temporary folder");
        std::fs::create_dir_all(root.path().join("src")).expect("a folder");
        std::fs::write(root.path().join("src/a.txt"), "x").expect("a file");
        std::os::unix::fs::symlink(root.path().join("src"), root.path().join("alias"))
            .expect("a link");

        // A file that's there, one through a link in the folder, and one
        // still to be made in a folder still to be made.
        for path in ["src/a.txt", "alias/a.txt", "new/deeper/b.txt"] {
            assert_eq!(
                resolved_inside(root.path(), Path::new(path)).await.ok(),
                Some(root.path().join(path)),
                "{path}"
            );
        }
    }
}
