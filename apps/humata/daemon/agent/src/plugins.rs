//! Which plugins a Copilot session has on. Copilot reads that, over the
//! person's own settings, from a repository's
//! `.github/copilot/settings.local.json`, so a session's choices are
//! written there in each of its copies of the code, which are Humata's,
//! and kept out of git so they never show in a review or a commit. Fusion's
//! tier is set there too.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// Where Copilot reads a repository's local settings.
const LOCAL_SETTINGS: &str = ".github/copilot/settings.local.json";

#[derive(Debug, thiserror::Error)]
#[error(
    "couldn't write which plugins this session has on to {}; check that the session's copy of the code is there and writable, then send again. Removing the file only brings back the plugins as Copilot has them", path.display()
)]
pub struct OverlayError {
    path: PathBuf,
    #[source]
    source: std::io::Error,
}

/// Sets `plugins` on or off in each of `checkouts`, and Fusion's tier,
/// keeping any other local settings there.
pub(crate) fn write_copilot(
    checkouts: &[PathBuf],
    plugins: &BTreeMap<String, bool>,
    fusion_tier: Option<&str>,
) -> Result<(), OverlayError> {
    for checkout in checkouts {
        let path = checkout.join(LOCAL_SETTINGS);
        // Nothing to set, and no tier of an earlier turn's to take out.
        if plugins.is_empty() && fusion_tier.is_none() && !path.exists() {
            continue;
        }
        write_one(checkout, &path, plugins, fusion_tier)?;
    }
    Ok(())
}

fn write_one(
    checkout: &Path,
    path: &Path,
    plugins: &BTreeMap<String, bool>,
    fusion_tier: Option<&str>,
) -> Result<(), OverlayError> {
    let failed = |source| OverlayError {
        path: path.to_owned(),
        source,
    };
    // A file someone else wrote that isn't an object is replaced, as
    // Copilot couldn't read it either.
    let mut settings = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Map<String, Value>>(&bytes).ok())
        .unwrap_or_default();
    let before = settings.clone();
    match fusion_tier {
        Some(tier) => {
            settings.insert("fusionTier".to_owned(), json!(tier));
        }
        None => {
            settings.remove("fusionTier");
        }
    }
    if !plugins.is_empty() {
        let enabled = settings
            .entry("enabledPlugins")
            .or_insert_with(|| json!({}));
        if !enabled.is_object() {
            *enabled = json!({});
        }
        if let Some(enabled) = enabled.as_object_mut() {
            for (plugin, on) in plugins {
                enabled.insert(plugin.clone(), json!(on));
            }
        }
    }
    if settings == before && path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    let text = serde_json::to_vec_pretty(&settings).map_err(|error| failed(error.into()))?;
    std::fs::write(path, text).map_err(failed)?;
    exclude(checkout).map_err(failed)
}

/// Keeps the local settings out of git in the repository `checkout` is a
/// worktree of; a folder that isn't one has nothing to keep it out of.
fn exclude(checkout: &Path) -> std::io::Result<()> {
    let Some(common) = common_dir(checkout) else {
        return Ok(());
    };
    let file = common.join("info/exclude");
    let line = format!("/{LOCAL_SETTINGS}");
    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    if existing.lines().any(|kept| kept.trim() == line) {
        return Ok(());
    }
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&line);
    text.push('\n');
    std::fs::write(file, text)
}

/// The git folder shared by `checkout` and the repository it's a worktree
/// of, or its own if it's a repository.
fn common_dir(checkout: &Path) -> Option<PathBuf> {
    let dot_git = checkout.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let git_dir = PathBuf::from(pointer.strip_prefix("gitdir:")?.trim());
    let git_dir = if git_dir.is_relative() {
        checkout.join(git_dir)
    } else {
        git_dir
    };
    match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(common) => Some(git_dir.join(common.trim())),
        Err(_) => Some(git_dir),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(path: &Path) -> Value {
        serde_json::from_slice(&std::fs::read(path).expect("written")).expect("JSON")
    }

    #[test]
    fn choices_go_over_the_local_settings_and_keep_the_rest() {
        let folder = tempfile::tempdir().expect("temp");
        let checkout = folder.path().join("code");
        std::fs::create_dir_all(checkout.join(".git/info")).expect("git");
        std::fs::create_dir_all(checkout.join(".github/copilot")).expect("folder");
        std::fs::write(
            checkout.join(LOCAL_SETTINGS),
            r#"{"model": "gpt-5", "enabledPlugins": {"kept@m": true, "lint@m": true}}"#,
        )
        .expect("seed");
        let plugins = BTreeMap::from([("lint@m".to_owned(), false), ("docs@m".to_owned(), true)]);

        write_copilot(std::slice::from_ref(&checkout), &plugins, None).expect("written");
        write_copilot(std::slice::from_ref(&checkout), &plugins, None).expect("again");

        assert_eq!(
            read(&checkout.join(LOCAL_SETTINGS)),
            json!({"model": "gpt-5", "enabledPlugins": {"kept@m": true, "lint@m": false, "docs@m": true}})
        );
        let excluded =
            std::fs::read_to_string(checkout.join(".git/info/exclude")).expect("exclude");
        assert_eq!(
            excluded
                .lines()
                .filter(|line| line.contains("settings.local"))
                .count(),
            1
        );
    }

    #[test]
    fn a_worktree_is_kept_out_of_its_repositorys_git() {
        let folder = tempfile::tempdir().expect("temp");
        let common = folder.path().join("clone/.git");
        let git_dir = common.join("worktrees/s1");
        std::fs::create_dir_all(&git_dir).expect("git dir");
        std::fs::write(git_dir.join("commondir"), "../..\n").expect("commondir");
        let checkout = folder.path().join("s1");
        std::fs::create_dir_all(&checkout).expect("checkout");
        std::fs::write(
            checkout.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .expect(".git");

        write_copilot(
            &[checkout.clone()],
            &BTreeMap::from([("lint@m".to_owned(), false)]),
            None,
        )
        .expect("written");

        let excluded = std::fs::read_to_string(common.join("info/exclude")).expect("exclude");
        assert!(excluded.contains(LOCAL_SETTINGS));
        assert!(checkout.join(LOCAL_SETTINGS).exists());
    }

    #[test]
    fn a_fusion_tier_is_set_and_taken_out_again() {
        let folder = tempfile::tempdir().expect("temp");
        let checkout = folder.path().to_owned();
        write_copilot(
            std::slice::from_ref(&checkout),
            &BTreeMap::new(),
            Some("high"),
        )
        .expect("tier");
        assert_eq!(
            read(&checkout.join(LOCAL_SETTINGS))["fusionTier"],
            json!("high")
        );

        write_copilot(std::slice::from_ref(&checkout), &BTreeMap::new(), None).expect("none");
        assert_eq!(read(&checkout.join(LOCAL_SETTINGS)).get("fusionTier"), None);
    }

    #[test]
    fn no_choices_write_nothing() {
        let folder = tempfile::tempdir().expect("temp");
        write_copilot(&[folder.path().to_owned()], &BTreeMap::new(), None).expect("nothing");
        assert!(!folder.path().join(LOCAL_SETTINGS).exists());
    }
}
