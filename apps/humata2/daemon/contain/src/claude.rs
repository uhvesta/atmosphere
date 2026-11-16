//! A contained Claude Code runs on its own copy of the user's config (D217
//! amendment), so nothing it changes reaches the real `~/.claude/` or
//! `~/.claude.json`.
//!
//! [`seed_claude_copy`] refreshes the copy before every launch and resume.
//! It copies `.claude.json` and the settings, `CLAUDE.md`, hooks, agents,
//! commands, skills and plugins again, and never copies `.credentials.json`.
//! The copy keeps its todos, checkpoints and this session's own project
//! entry in `.claude.json`. Its `projects` is a link to the real
//! `~/.claude/projects`, so the conversation is saved where it always is.
//! Only this session's project folder there is writable.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use crate::{ContainError, Login};

/// What is copied again from `~/.claude/` at every launch and resume.
/// `.credentials.json` is left out on purpose: the login stays in the keychain.
const SEEDED: &[&str] = &[
    "settings.json",
    "settings.local.json",
    "CLAUDE.md",
    "hooks",
    "agents",
    "commands",
    "skills",
    "plugins",
];

/// Claude Code's global config file, in home or in the config folder.
const GLOBAL_CONFIG: &str = ".claude.json";

/// A refreshed copy of the user's Claude Code config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeCopy {
    /// The copy itself. `CLAUDE_CONFIG_DIR` points here.
    pub folder: PathBuf,
    /// This session's project folders in the real `~/.claude/projects`: the
    /// only part of the real config the agent may write.
    pub projects: Vec<PathBuf>,
}

impl ClaudeCopy {
    /// The environment the contained agent runs with: the copy as its config
    /// folder and, for the keychain `login`, the usual keychain login.
    /// Setting a config folder would otherwise make Claude Code look for a
    /// login under a name of its own; an empty
    /// `CLAUDE_SECURESTORAGE_CONFIG_DIR` keeps the usual one. A token login
    /// needs no more: the token is in the launch environment already.
    pub fn env(&self, login: Login) -> Vec<(OsString, OsString)> {
        let mut env = vec![(
            OsString::from("CLAUDE_CONFIG_DIR"),
            self.folder.clone().into_os_string(),
        )];
        if login == Login::Keychain {
            env.push((OsString::from("CLAUDE_SECURESTORAGE_CONFIG_DIR"), OsString::new()));
        }
        env
    }
}

/// Refreshes `copy` from the user's config in `home`, for an agent that works
/// in `cwd`. Any failure is an error, so the session is not started on a
/// copy that is half made.
pub fn seed_claude_copy(home: &Path, copy: &Path, cwd: &Path) -> Result<ClaudeCopy, ContainError> {
    let failed = |path: &Path| {
        let path = path.to_path_buf();
        move |source: io::Error| ContainError::ClaudeCopy { path, source }
    };
    std::fs::create_dir_all(copy).map_err(failed(copy))?;

    let real = home.join(".claude");
    for name in SEEDED {
        let target = copy.join(name);
        remove(&target).map_err(failed(&target))?;
        let source = real.join(name);
        if std::fs::symlink_metadata(&source).is_ok() {
            copy_tree(&source, &target).map_err(failed(&source))?;
        }
    }

    let cwds = cwd_names(cwd);
    seed_global_config(home, copy, &cwds)?;

    let real_projects = real.join("projects");
    std::fs::create_dir_all(&real_projects).map_err(failed(&real_projects))?;
    link_projects(&real_projects, &copy.join("projects"))?;

    let mut projects = Vec::new();
    for name in &cwds {
        let folder = real_projects.join(project_folder_name(name));
        std::fs::create_dir_all(&folder).map_err(failed(&folder))?;
        if !projects.contains(&folder) {
            projects.push(folder);
        }
    }

    Ok(ClaudeCopy {
        folder: copy.to_path_buf(),
        projects,
    })
}

/// The names the agent may know its working folder by: as given, and as the
/// kernel resolves it (on macOS `/var` is a link to `/private/var`).
fn cwd_names(cwd: &Path) -> Vec<String> {
    let mut names = BTreeSet::new();
    names.insert(cwd.to_string_lossy().into_owned());
    if let Ok(resolved) = std::fs::canonicalize(cwd) {
        names.insert(resolved.to_string_lossy().into_owned());
    }
    names.into_iter().collect()
}

/// The folder Claude Code keeps a working folder's conversations in, under
/// `projects`: every character but ASCII letters and digits becomes `-`.
fn project_folder_name(cwd: &str) -> String {
    cwd.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect()
}

/// Writes the copy's `.claude.json`: the user's, with this session's own
/// project entries kept from the copy's last run.
fn seed_global_config(home: &Path, copy: &Path, cwds: &[String]) -> Result<(), ContainError> {
    let user_path = home.join(GLOBAL_CONFIG);
    let copy_path = copy.join(GLOBAL_CONFIG);
    let mut seeded = read_object(&user_path)?.unwrap_or_default();
    if let Some(previous) = read_object(&copy_path)? {
        let kept: Vec<(String, serde_json::Value)> = previous
            .get("projects")
            .and_then(serde_json::Value::as_object)
            .map(|projects| {
                cwds.iter()
                    .filter_map(|cwd| projects.get(cwd).map(|entry| (cwd.clone(), entry.clone())))
                    .collect()
            })
            .unwrap_or_default();
        if !kept.is_empty() {
            let projects = seeded
                .entry("projects")
                .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
            if !projects.is_object() {
                *projects = serde_json::Value::Object(serde_json::Map::new());
            }
            if let Some(projects) = projects.as_object_mut() {
                projects.extend(kept);
            }
        }
    }

    let text = serde_json::to_string_pretty(&serde_json::Value::Object(seeded)).map_err(|source| {
        ContainError::ClaudeConfigUnreadable {
            path: user_path.clone(),
            source,
        }
    })?;
    // Written beside it and renamed over it, so a failed write leaves the
    // last copy whole.
    let partial = copy.join(".claude.json.humata-seed");
    let failed = |source| ContainError::ClaudeCopy {
        path: copy_path.clone(),
        source,
    };
    std::fs::write(&partial, text + "\n").map_err(failed)?;
    std::fs::rename(&partial, &copy_path).map_err(failed)
}

/// A JSON object from `path`, or `None` when there is no file.
fn read_object(path: &Path) -> Result<Option<serde_json::Map<String, serde_json::Value>>, ContainError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ContainError::ClaudeCopy {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|source| ContainError::ClaudeConfigUnreadable {
            path: path.to_path_buf(),
            source,
        })?;
    match value {
        serde_json::Value::Object(object) => Ok(Some(object)),
        _ => Err(ContainError::ClaudeConfigNotAnObject {
            path: path.to_path_buf(),
        }),
    }
}

/// Makes `link` a link to `real`. A link that points elsewhere is replaced;
/// a real folder in its place is an error, since it holds conversations the
/// user would lose.
fn link_projects(real: &Path, link: &Path) -> Result<(), ContainError> {
    let failed = |source| ContainError::ClaudeCopy {
        path: link.to_path_buf(),
        source,
    };
    match std::fs::symlink_metadata(link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if std::fs::read_link(link).map_err(failed)? == real {
                return Ok(());
            }
            std::fs::remove_file(link).map_err(failed)?;
        }
        Ok(_) => {
            return Err(ContainError::ClaudeProjectsNotALink {
                path: link.to_path_buf(),
                real: real.to_path_buf(),
            });
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(failed(source)),
    }
    symlink(real, link).map_err(failed)
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(not(unix))]
fn symlink(_target: &Path, _link: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "links are only made on unix systems",
    ))
}

/// Removes a file, link or folder, if there is one.
fn remove(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(source),
    }
}

/// Copies a file or folder, keeping links as links.
fn copy_tree(source: &Path, target: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(source)?;
    if meta.file_type().is_symlink() {
        symlink(&std::fs::read_link(source)?, target)
    } else if meta.is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_tree(&entry.path(), &target.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(source, target).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;

    use super::{project_folder_name, seed_claude_copy};
    use crate::{ContainError, Login};

    /// A temp home with a Claude Code config, a session folder and the copy's
    /// place in it. Nothing here touches the real home folder.
    struct Setup {
        _dir: TempDir,
        home: PathBuf,
        session: PathBuf,
        copy: PathBuf,
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().expect("a temp folder");
        let home = dir.path().join("home");
        let claude = home.join(".claude");
        std::fs::create_dir_all(claude.join("skills/review")).expect("a folder");
        std::fs::write(claude.join("settings.json"), "{\"theme\":\"dark\"}\n").expect("a file");
        std::fs::write(claude.join("CLAUDE.md"), "be brief\n").expect("a file");
        std::fs::write(claude.join("skills/review/SKILL.md"), "review\n").expect("a file");
        std::fs::write(claude.join(".credentials.json"), "{\"token\":\"t\"}\n").expect("a file");
        std::fs::write(
            home.join(".claude.json"),
            "{\"oauthAccount\":{\"email\":\"a@b\"},\"projects\":{\"/elsewhere\":{\"allowedTools\":[]}}}\n",
        )
        .expect("a file");
        let session = dir.path().join("channels/acme/fix-login");
        std::fs::create_dir_all(&session).expect("a folder");
        let copy = session.join(".humata/claude");
        Setup {
            _dir: dir,
            home,
            session,
            copy,
        }
    }

    fn json(path: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(path).expect("the file")).expect("json")
    }

    #[test]
    fn the_copy_has_the_users_config_but_never_the_credentials() {
        let s = setup();
        seed_claude_copy(&s.home, &s.copy, &s.session).expect("a copy");

        assert_eq!(
            std::fs::read_to_string(s.copy.join("settings.json")).expect("settings"),
            "{\"theme\":\"dark\"}\n"
        );
        assert!(s.copy.join("CLAUDE.md").is_file());
        assert!(s.copy.join("skills/review/SKILL.md").is_file());
        assert!(!s.copy.join(".credentials.json").exists());
        assert_eq!(json(&s.copy.join(".claude.json"))["oauthAccount"]["email"], "a@b");
    }

    #[test]
    fn conversations_go_to_the_real_projects_folder() {
        let s = setup();
        let copy = seed_claude_copy(&s.home, &s.copy, &s.session).expect("a copy");

        let real = s.home.join(".claude/projects");
        assert_eq!(std::fs::read_link(s.copy.join("projects")).expect("a link"), real);
        assert!(!copy.projects.is_empty());
        for folder in &copy.projects {
            assert_eq!(folder.parent(), Some(real.as_path()));
            assert!(folder.is_dir());
        }
    }

    #[test]
    fn a_reseed_takes_the_users_config_again_and_keeps_the_sessions_own_state() {
        let s = setup();
        seed_claude_copy(&s.home, &s.copy, &s.session).expect("a copy");

        // The session changes its copy: settings, a todo, and its own project entry.
        std::fs::write(s.copy.join("settings.json"), "{\"theme\":\"light\"}\n").expect("a file");
        std::fs::create_dir_all(s.copy.join("todos")).expect("a folder");
        std::fs::write(s.copy.join("todos/t.json"), "[]\n").expect("a file");
        let own = s.session.to_string_lossy().into_owned();
        let mut config = json(&s.copy.join(".claude.json"));
        config["projects"][own.as_str()] = serde_json::json!({"hasTrustDialogAccepted": true});
        config["projects"]["/elsewhere"] = serde_json::json!({"changed": true});
        std::fs::write(s.copy.join(".claude.json"), config.to_string()).expect("a file");
        // The user removes a skill meanwhile.
        std::fs::remove_dir_all(s.home.join(".claude/skills")).expect("removed");

        seed_claude_copy(&s.home, &s.copy, &s.session).expect("a copy");

        assert_eq!(
            std::fs::read_to_string(s.copy.join("settings.json")).expect("settings"),
            "{\"theme\":\"dark\"}\n"
        );
        assert!(!s.copy.join("skills").exists());
        assert!(s.copy.join("todos/t.json").is_file());
        let config = json(&s.copy.join(".claude.json"));
        assert_eq!(config["projects"][own.as_str()]["hasTrustDialogAccepted"], true);
        assert_eq!(config["projects"]["/elsewhere"], serde_json::json!({"allowedTools": []}));
        // The user's own config is untouched.
        assert!(json(&s.home.join(".claude.json"))["projects"].get(own.as_str()).is_none());
    }

    #[test]
    fn a_home_without_claude_config_still_gets_a_copy() {
        let dir = tempfile::tempdir().expect("a temp folder");
        let home = dir.path().join("home");
        let session = dir.path().join("session");
        std::fs::create_dir_all(&home).expect("a folder");
        std::fs::create_dir_all(&session).expect("a folder");

        let copy = seed_claude_copy(&home, &session.join(".humata/claude"), &session).expect("a copy");
        assert!(copy.folder.join(".claude.json").is_file());
    }

    #[test]
    fn a_config_that_is_not_json_stops_the_seed() {
        let s = setup();
        std::fs::write(s.home.join(".claude.json"), "{not json").expect("a file");

        let result = seed_claude_copy(&s.home, &s.copy, &s.session);
        assert!(
            matches!(result, Err(ContainError::ClaudeConfigUnreadable { .. })),
            "got {result:?}"
        );
    }

    #[test]
    fn a_real_folder_where_the_projects_link_goes_is_not_replaced() {
        let s = setup();
        std::fs::create_dir_all(s.copy.join("projects/x")).expect("a folder");

        let result = seed_claude_copy(&s.home, &s.copy, &s.session);
        assert!(
            matches!(result, Err(ContainError::ClaudeProjectsNotALink { .. })),
            "got {result:?}"
        );
        assert!(s.copy.join("projects/x").is_dir());
    }

    #[test]
    fn the_env_names_the_copy_and_keeps_the_usual_login() {
        let s = setup();
        let copy = seed_claude_copy(&s.home, &s.copy, &s.session).expect("a copy");
        let env = copy.env(Login::Keychain);

        assert!(env.contains(&("CLAUDE_CONFIG_DIR".into(), s.copy.clone().into_os_string())));
        assert!(env.contains(&("CLAUDE_SECURESTORAGE_CONFIG_DIR".into(), "".into())));
    }

    #[test]
    fn a_token_login_leaves_the_keychain_variable_unset() {
        let s = setup();
        let copy = seed_claude_copy(&s.home, &s.copy, &s.session).expect("a copy");
        let env = copy.env(Login::Token);

        assert!(env.contains(&("CLAUDE_CONFIG_DIR".into(), s.copy.clone().into_os_string())));
        assert!(env.iter().all(|(name, _)| name != "CLAUDE_SECURESTORAGE_CONFIG_DIR"));
    }

    #[test]
    fn credentials_are_not_copied_by_any_seed() {
        let s = setup();
        seed_claude_copy(&s.home, &s.copy, &s.session).expect("a copy");
        seed_claude_copy(&s.home, &s.copy, &s.session).expect("a reseed");

        assert!(!s.copy.join(".credentials.json").exists());
        assert!(s.home.join(".claude/.credentials.json").is_file());
    }

    #[test]
    fn a_project_folder_is_named_for_its_working_folder() {
        assert_eq!(project_folder_name("/Users/a/my.repo_x"), "-Users-a-my-repo-x");
    }
}
