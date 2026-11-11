use std::path::{Component, Path, PathBuf};

/// `path` under `root`, if it stays there: relative, without `..`.
pub(crate) fn inside(root: &Path, path: &Path) -> Option<PathBuf> {
    let stays = path
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    (stays && !path.as_os_str().is_empty()).then(|| root.join(path))
}
