//! The host's `gh` login, the default branch prefix's source (FC-12, D196):
//! asked in the background so a start never waits on it, and again now and
//! then so a sign-in or a change shows up without a restart.

use std::process::{Command, Stdio};

#[derive(Debug, thiserror::Error)]
pub(super) enum LoginError {
    #[error("couldn't run {program} to learn the GitHub login, so branch names get no default prefix; install the GitHub CLI or set HUMATA2_GH to it")]
    Spawn {
        program: String,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} api user failed ({status}): {stderr}; run `gh auth login`, or set a branch prefix in Settings")]
    Failed {
        program: String,
        status: String,
        stderr: String,
    },
}

use crate::tool_path::gh_program;

/// The login `gh` is signed in as; empty when it prints none. Blocking: it
/// runs `gh`, so it belongs on a thread of its own.
pub(super) fn login() -> Result<String, LoginError> {
    let program = gh_program();
    let name = program.to_string_lossy().into_owned();
    let output = Command::new(&program)
        .args(["api", "user", "--jq", ".login"])
        .env("GH_NO_UPDATE_NOTIFIER", "1")
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|source| LoginError::Spawn {
            program: name.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(LoginError::Failed {
            program: name,
            status: output.status.to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// The default prefix for `login`: the login and "/", or none when `gh`
/// gave nothing a branch name could begin with.
pub(super) fn prefix_of(login: &str) -> String {
    let prefix = format!("{login}/");
    if login.is_empty() || !humata2_sessions::valid_prefix(&prefix) {
        return String::new();
    }
    prefix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_becomes_a_prefix_and_nothing_usable_becomes_none() {
        assert_eq!(prefix_of("octo"), "octo/");
        assert_eq!(prefix_of(""), "");
        assert_eq!(prefix_of("two words"), "");
    }
}
