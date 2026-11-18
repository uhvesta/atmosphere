//! The check step (MP-19): one script that reports what setup needs to know
//! about a machine, as `humata2.<key>=<value>` lines so a login banner can't
//! be mistaken for a fact.

use std::collections::BTreeMap;

use crate::install::InstalledFile;
use crate::link::BIN_DIR;
use crate::platform::{Arch, Os, Platform};
use crate::service::{self, ServiceKind};

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    pub platform: Platform,
    pub user: String,
    pub home: String,
    pub git: bool,
    git_version: String,
    gh_version: String,
    /// Free space in the home folder; none when `df` said nothing usable.
    pub disk_free_kib: Option<u64>,
    /// `systemctl --user` works now, so humatad2 can be a user service.
    pub systemd: bool,
    /// systemd booted the machine, so the user's manager starts once
    /// lingering is on even when it isn't running yet.
    pub systemd_init: bool,
    /// Lingering is already on for the user.
    pub linger: bool,
    pub crontab: bool,
    pub setsid: bool,
    /// How humatad2 runs now from what setup put in place; none when it
    /// doesn't, or not that way.
    pub running: Option<ServiceKind>,
    installed: BTreeMap<InstalledFile, String>,
}

impl Facts {
    /// The number `git --version` printed, such as `2.47.1`; empty when git
    /// is missing or printed none.
    pub fn git_version(&self) -> &str {
        &self.git_version
    }

    /// The number `gh --version` printed, such as `2.83.0`; empty when there
    /// is no `gh` or it printed none.
    pub fn gh_version(&self) -> &str {
        &self.gh_version
    }

    /// The SHA-256 of `file` as installed now; none when it isn't there.
    pub fn installed(&self, file: InstalledFile) -> Option<&str> {
        self.installed.get(&file).map(String::as_str)
    }

}

/// The check script. It changes nothing on the machine.
pub(crate) fn script() -> String {
    let mut script = String::from(
        r#"export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
fact() { printf 'humata2.%s=%s\n' "$1" "$2"; }
has() { if command -v "$1" >/dev/null 2>&1; then echo yes; else echo no; fi; }
sha() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d ' ' -f 1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d ' ' -f 1
  else echo unknown; fi
}
fact os "$(uname -s)"
fact arch "$(uname -m)"
fact user "$(id -un)"
fact home "$HOME"
fact git "$(has git)"
version() { "$@" 2>/dev/null | sed -n '1s/^[^0-9]*\([0-9][0-9.]*\).*/\1/p'; }
if command -v git >/dev/null 2>&1; then fact git_version "$(version git --version)"; fi
if command -v gh >/dev/null 2>&1; then fact gh_version "$(version gh --version)"; fi
fact disk_free_kib "$(df -Pk "$HOME" 2>/dev/null | awk 'NR == 2 { print $4 }')"
if command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1; then fact systemd yes; else fact systemd no; fi
if [ -d /run/systemd/system ] && command -v systemctl >/dev/null 2>&1; then fact systemd_init yes; else fact systemd_init no; fi
fact linger "$(loginctl show-user "$(id -un)" -p Linger --value 2>/dev/null)"
fact crontab "$(has crontab)"
fact setsid "$(has setsid)"
"#,
    );
    script.push_str(&service::running_script());
    for file in InstalledFile::ALL {
        let name = file.file_name();
        script.push_str(&format!(
            "if [ -f \"$HOME/{BIN_DIR}/{name}\" ]; then fact sha.{name} \"$(sha \"$HOME/{BIN_DIR}/{name}\")\"; else fact sha.{name} none; fi\n"
        ));
    }
    script
}

/// Reads the check's stdout; none when `os`, `arch`, `user` or `home` is
/// missing or unreadable.
pub(crate) fn parse(stdout: &[u8]) -> Option<Facts> {
    let text = String::from_utf8_lossy(stdout);
    let facts: BTreeMap<&str, &str> = text
        .lines()
        .filter_map(|line| line.trim_end_matches('\r').strip_prefix("humata2.")?.split_once('='))
        .collect();
    let yes = |key: &str| facts.get(key) == Some(&"yes");
    let user = facts.get("user").filter(|user| {
        !user.is_empty() && !user.chars().any(|c| c.is_whitespace() || c.is_control())
    })?;
    let shas = |prefix: &str| {
        let mut found = BTreeMap::new();
        for file in InstalledFile::ALL {
            let key = format!("{prefix}.{}", file.file_name());
            if let Some(sha) = facts.get(key.as_str()) {
                if sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()) {
                    found.insert(file, sha.to_ascii_lowercase());
                }
            }
        }
        found
    };
    let version = |key: &str| {
        facts
            .get(key)
            .filter(|number| {
                !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit() || b == b'.')
            })
            .map_or_else(String::new, |number| (*number).to_owned())
    };
    let installed = shas("sha");
    Some(Facts {
        platform: Platform {
            os: Os::parse(facts.get("os")?)?,
            arch: Arch::parse(facts.get("arch")?)?,
        },
        user: (*user).to_owned(),
        home: (*facts.get("home")?).to_owned(),
        git: yes("git"),
        git_version: version("git_version"),
        gh_version: version("gh_version"),
        disk_free_kib: facts.get("disk_free_kib").and_then(|kib| kib.parse().ok()),
        systemd: yes("systemd"),
        systemd_init: yes("systemd_init"),
        linger: yes("linger"),
        crontab: yes("crontab"),
        setsid: yes("setsid"),
        running: match facts.get("running") {
            Some(&"systemd") => Some(ServiceKind::SystemdUser),
            Some(&"setsid") => Some(ServiceKind::Setsid),
            _ => None,
        },
        installed,
    })
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn the_check_runs_here_and_reads_back() {
        let home = tempfile::tempdir().unwrap();
        let output = Command::new("/bin/sh")
            .arg("-c")
            .arg(script())
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .output()
            .unwrap();
        let facts = parse(&output.stdout).expect("facts");
        assert_eq!(facts.home, home.path().display().to_string());
        assert_eq!(facts.installed(InstalledFile::Humatad), None);
        assert!(facts.disk_free_kib.is_some());
    }

    #[test]
    fn a_banner_and_missing_facts_are_ignored() {
        let facts = parse(
            b"Welcome! humata2 is great\nhumata2.os=Linux\r\nhumata2.arch=amd64\nhumata2.user=dev\nhumata2.home=/home/dev\nhumata2.git=yes\nhumata2.sha.humatad2=none\n",
        )
        .expect("facts");
        assert_eq!(
            facts.platform,
            Platform {
                os: Os::Linux,
                arch: Arch::X86_64
            }
        );
        assert!(facts.git);
        assert!(!facts.systemd);
        assert_eq!(facts.disk_free_kib, None);
        assert_eq!(facts.installed(InstalledFile::Humatad), None);
    }

    #[test]
    fn facts_without_a_platform_are_unreadable() {
        assert_eq!(parse(b"humata2.user=dev\nhumata2.home=/home/dev\n"), None);
    }
}
