//! The variables a terminal has. The app starts humatad from launchd, and a
//! host is started over SSH without a login shell, so either gets a bare
//! PATH: no Homebrew, no `~/.local/bin`, no `node` for an npm-installed
//! Copilot, none of the person's exports. The login shell's are read once, at
//! start, and taken for any not set already, PATH from the shell always.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long the person's shell gets to start and print its variables.
const SHELL_WAIT: Duration = Duration::from_secs(10);

/// Printed just before the variables, so whatever the shell's startup files
/// print is skipped.
const MARK: &str = "\0humata-env\0";

/// The shell's own bookkeeping, which means nothing to a program it didn't
/// start.
const SKIPPED: [&str; 5] = ["_", "PWD", "OLDPWD", "SHLVL", "PS1"];

#[derive(Debug, thiserror::Error)]
pub enum LoginEnvError {
    #[error(
        "couldn't start your login shell {shell} to read its variables; agents get only humatad's own, so set SHELL to a shell that runs, or put the programs' folders in Hosts' variables"
    )]
    Start {
        shell: String,
        #[source]
        source: std::io::Error,
    },

    #[error(
        "your login shell {shell} took more than {} seconds to start, so its variables weren't read; agents get only humatad's own. Speed up its startup files, or put the programs' folders in Hosts' variables",
        SHELL_WAIT.as_secs()
    )]
    TimedOut { shell: String },

    #[error(
        "your login shell {shell} didn't print its variables; agents get only humatad's own. Check that `{shell} -ilc env` works in a terminal"
    )]
    NoOutput { shell: String },
}

/// The variables to set for this process: the login shell's that aren't set
/// here, and its PATH.
pub fn login_environment() -> Result<Vec<(String, String)>, LoginEnvError> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_owned());
    let printed = read(&shell)?;
    Ok(merge(&printed, |name| std::env::var_os(name).is_some()))
}

fn read(shell: &str) -> Result<String, LoginEnvError> {
    let mut child = Command::new(shell)
        .args([
            "-i",
            "-l",
            "-c",
            &format!("printf '{}'; env -0", MARK.replace('\0', "\\0")),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| LoginEnvError::Start {
            shell: shell.to_owned(),
            source,
        })?;
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut text = Vec::new();
        if let Some(stdout) = stdout.as_mut() {
            let _ = stdout.read_to_end(&mut text);
        }
        text
    });
    let deadline = Instant::now() + SHELL_WAIT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(LoginEnvError::TimedOut {
                    shell: shell.to_owned(),
                });
            }
        }
    }
    let text = String::from_utf8_lossy(&reader.join().unwrap_or_default()).into_owned();
    match text.split_once(MARK) {
        Some((_, variables)) => Ok(variables.to_owned()),
        None => Err(LoginEnvError::NoOutput {
            shell: shell.to_owned(),
        }),
    }
}

/// From `env -0`'s output, PATH and the variables `set` says aren't set.
fn merge(printed: &str, set: impl Fn(&str) -> bool) -> Vec<(String, String)> {
    printed
        .split('\0')
        .filter_map(|entry| entry.split_once('='))
        .filter(|(name, value)| !name.is_empty() && !value.is_empty() && !SKIPPED.contains(name))
        .filter(|(name, _)| *name == "PATH" || !set(name))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shells_path_and_only_unset_variables_are_taken() {
        let printed = "PATH=/opt/homebrew/bin:/usr/bin\0HOME=/elsewhere\0\
                       ANTHROPIC_API_KEY=sk\0SHLVL=2\0EMPTY=\0";
        let taken = merge(printed, |name| ["HOME", "PATH"].contains(&name));
        assert_eq!(
            taken,
            [
                ("PATH".to_owned(), "/opt/homebrew/bin:/usr/bin".to_owned()),
                ("ANTHROPIC_API_KEY".to_owned(), "sk".to_owned()),
            ]
        );
    }

    #[test]
    fn what_the_startup_files_print_is_skipped() {
        let printed = read("/bin/sh").unwrap_or_else(|error| panic!("{error}"));
        assert!(
            merge(&printed, |_| false)
                .iter()
                .any(|(name, _)| name == "PATH")
        );
    }
}
