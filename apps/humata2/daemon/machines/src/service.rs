//! Keeping humatad2 running on a Linux machine: a systemd user service with
//! lingering (PER-18), `setsid` plus a crontab `@reboot` entry where there is
//! no systemd (PER-19), or built-in `--daemonize` (`ServiceKind::Detached`)
//! where neither systemd nor `setsid`+`crontab` is available.

use crate::check::Facts;
use crate::link::BIN_DIR;
use crate::runner::Exit;

/// The service script's exit codes for what it found wrong.
pub(crate) const LINGER_UNAVAILABLE: i32 = 75;
pub(crate) const USER_MANAGER_UNAVAILABLE: i32 = 76;
pub(crate) const DID_NOT_START: i32 = 79;

/// How humatad2 is kept running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// A systemd user service, with lingering so it runs after logout and
    /// starts at boot.
    SystemdUser,
    /// `setsid` now and a crontab `@reboot` entry for boots. Nothing restarts
    /// it if it crashes.
    Setsid,
    /// `humatad2 serve --role machine --daemonize`: built-in double-fork
    /// daemonization for machines or containers without systemd, `setsid`, or
    /// `crontab`. Reconnect starts it again if the machine restarts.
    Detached,
}

impl ServiceKind {
    /// The kind the machine supports, systemd first: a machine booted by
    /// systemd gets a user service even when the user's manager isn't
    /// running yet, since turning lingering on starts it. When neither
    /// systemd nor `setsid`+`crontab` is available, falls back to `Detached`
    /// (`humatad2 --daemonize`).
    pub(crate) fn for_machine(facts: &Facts) -> Self {
        if facts.systemd || facts.systemd_init {
            Self::SystemdUser
        } else if facts.setsid && facts.crontab {
            Self::Setsid
        } else {
            Self::Detached
        }
    }

    /// The non-systemd fallback for this machine: `Setsid` when both `setsid`
    /// and `crontab` are available, otherwise `Detached`.
    pub(crate) fn without_systemd(facts: &Facts) -> Self {
        if facts.setsid && facts.crontab {
            Self::Setsid
        } else {
            Self::Detached
        }
    }
}

/// What the service script's exit says.
pub(crate) enum ServiceOutcome {
    Running,
    LingerUnavailable,
    UserManagerUnavailable,
    DidNotStart,
    Failed(Exit),
}

impl ServiceOutcome {
    pub(crate) fn from_exit(exit: Exit) -> Self {
        match exit {
            Exit::Code(0) => Self::Running,
            Exit::Code(LINGER_UNAVAILABLE) => Self::LingerUnavailable,
            Exit::Code(USER_MANAGER_UNAVAILABLE) => Self::UserManagerUnavailable,
            Exit::Code(DID_NOT_START) => Self::DidNotStart,
            exit => Self::Failed(exit),
        }
    }
}

/// The unit. `KillMode=process` stops only humatad2 on a restart, so the
/// holders it started keep their sessions. It holds no secrets and
/// no `Environment=` lines: host variables stay in humatad2's memory.
const UNIT: &str = "[Unit]
Description=Humata 2 daemon (humatad2)

[Service]
ExecStart=%h/.humata2/bin/humatad2 serve --role machine
Restart=on-failure
RestartSec=2
KillMode=process

[Install]
WantedBy=default.target
";

/// Shell helpers every script shares. `stray` stops a machine daemon this
/// unit doesn't own, such as one a bridge, setsid, or `--daemonize` start left
/// behind, which would hold the socket and make the new one exit; it never
/// matches a `serve --stdio` peer link. `-x -f` matches the whole command
/// line, which must start with the daemon's own path: the scripts run as
/// `sh -c '<script>'` and their text holds the unit's `ExecStart=` line, so
/// a pattern matched anywhere in the command line would stop the script.
/// When `pgrep`/`pkill` are unavailable (e.g. minimal containers), falls back
/// to the PID recorded in `$HOME/.humata2/state/daemon.lock`.
const HELPERS: &str = r#"set -u
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"
unit_dir="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
drop_cron() {
  if command -v crontab >/dev/null 2>&1 && crontab -l 2>/dev/null | grep -q '# humatad2$'; then
    crontab -l 2>/dev/null | grep -v '# humatad2$' | crontab - || return 1
  fi
  return 0
}
daemon_running() {
  if command -v pgrep >/dev/null 2>&1; then
    pgrep -u "$(id -u)" -x -f '[^ ]*humatad2 serve --role machine( --daemonize)?' >/dev/null 2>&1 && return 0
  fi
  lock="$HOME/.humata2/state/daemon.lock"
  if [ -f "$lock" ]; then
    pid=$(cat "$lock" 2>/dev/null)
    case "$pid" in
      ''|*[!0-9]*) return 1 ;;
    esac
    kill -0 "$pid" 2>/dev/null && [ -S "$HOME/.humata2/humatad.sock" ] && return 0
  fi
  return 1
}
stray() {
  if command -v pkill >/dev/null 2>&1; then
    pkill -u "$(id -u)" -x -f '[^ ]*humatad2 serve --role machine( --daemonize)?' 2>/dev/null
  fi
  lock="$HOME/.humata2/state/daemon.lock"
  if [ -f "$lock" ]; then
    pid=$(cat "$lock" 2>/dev/null)
    case "$pid" in
      ''|*[!0-9]*) ;;
      *) kill "$pid" 2>/dev/null ;;
    esac
  fi
  n=0
  while daemon_running && [ "$n" -lt 10 ]; do
    sleep 1; n=$((n + 1))
  done
  return 0
}
"#;

/// The crontab line that starts humatad2 at boot without systemd. It holds
/// no single quote, so scripts can quote it whole.
const BOOT_LINE: &str = r#"@reboot setsid "$HOME/.humata2/bin/humatad2" serve --role machine </dev/null >/dev/null 2>&1 # humatad2"#;

/// A check fragment: prints `humata2.running=systemd` when the user service
/// runs from the unit [`script`] writes, with lingering on and no boot
/// entry beside it; `humata2.running=setsid` when the daemon runs with the
/// boot entry and no unit; `humata2.running=detached` when the daemon runs
/// with neither unit nor boot entry; else `humata2.running=no`. Setup then
/// restarts nothing when no file changed either.
pub(crate) fn running_script() -> String {
    let unit = UNIT.trim_end();
    format!(
        r#"unit_dir="${{XDG_CONFIG_HOME:-$HOME/.config}}/systemd/user"
expected_unit='{unit}'
boot_entry=no
if command -v crontab >/dev/null 2>&1 && crontab -l 2>/dev/null | grep -qxF '{BOOT_LINE}'; then boot_entry=yes; fi
daemon_running() {{
  if command -v pgrep >/dev/null 2>&1; then
    pgrep -u "$(id -u)" -x -f '[^ ]*humatad2 serve --role machine( --daemonize)?' >/dev/null 2>&1 && return 0
  fi
  lock="$HOME/.humata2/state/daemon.lock"
  if [ -f "$lock" ]; then
    pid=$(cat "$lock" 2>/dev/null)
    case "$pid" in
      ''|*[!0-9]*) return 1 ;;
    esac
    kill -0 "$pid" 2>/dev/null && [ -S "$HOME/.humata2/humatad.sock" ] && return 0
  fi
  return 1
}}
if [ -f "$unit_dir/humatad2.service" ] && [ "$(cat "$unit_dir/humatad2.service" 2>/dev/null)" = "$expected_unit" ] \
  && [ "$boot_entry" = no ] \
  && [ "$(loginctl show-user "$(id -un)" -p Linger --value 2>/dev/null)" = yes ] \
  && systemctl --user is-enabled --quiet humatad2.service 2>/dev/null \
  && systemctl --user is-active --quiet humatad2.service 2>/dev/null; then
  printf 'humata2.running=systemd\n'
elif [ ! -f "$unit_dir/humatad2.service" ] && [ "$boot_entry" = yes ] \
  && daemon_running; then
  printf 'humata2.running=setsid\n'
elif [ ! -f "$unit_dir/humatad2.service" ] && [ "$boot_entry" = no ] \
  && daemon_running; then
  printf 'humata2.running=detached\n'
else
  printf 'humata2.running=no\n'
fi
"#
    )
}

/// The script that installs and (re)starts the service, so a new humatad2
/// takes over. It exits 0 once humatad2 has been running for a moment.
pub(crate) fn script(kind: ServiceKind) -> String {
    let body = match kind {
        ServiceKind::SystemdUser => format!(
            r#"u=$(id -un)
linger() {{ [ "$(loginctl show-user "$u" -p Linger --value 2>/dev/null)" = yes ]; }}
if ! linger; then
  loginctl enable-linger "$u" >/dev/null 2>&1
  linger || {{ echo "lingering can't be turned on for $u" >&2; exit {LINGER_UNAVAILABLE}; }}
fi
# Lingering starts the user's manager; give it a moment.
n=0
until systemctl --user show-environment >/dev/null 2>&1; do
  n=$((n + 1))
  [ "$n" -le 10 ] || {{ echo "the systemd user manager for $u isn't running" >&2; exit {USER_MANAGER_UNAVAILABLE}; }}
  sleep 1
done
mkdir -p "$unit_dir" || exit 1
cat > "$unit_dir/humatad2.service.new" <<'HUMATA_UNIT'
{UNIT}HUMATA_UNIT
mv -f "$unit_dir/humatad2.service.new" "$unit_dir/humatad2.service" || exit 1
systemctl --user daemon-reload || exit 1
systemctl --user enable humatad2.service || exit 1
systemctl --user stop humatad2.service 2>/dev/null
stray
drop_cron || exit 1
systemctl --user start humatad2.service || exit 1
up() {{ systemctl --user is-active --quiet humatad2.service; }}
n=0
until up; do
  n=$((n + 1)); [ "$n" -le 10 ] || break
  sleep 1
done
# A daemon that exits at once can look active for a moment.
up && sleep 2
if ! up; then
  journalctl --user -u humatad2.service -n 20 --no-pager >&2 2>/dev/null || systemctl --user status humatad2.service --no-pager >&2
  exit {DID_NOT_START}
fi
"#
        ),
        ServiceKind::Setsid => format!(
            r#"bin="$HOME/{BIN_DIR}/humatad2"
# A user service from an earlier setup would start a second daemon.
if command -v systemctl >/dev/null 2>&1 && [ -f "$unit_dir/humatad2.service" ]; then
  systemctl --user disable --now humatad2.service >/dev/null 2>&1
  rm -f "$unit_dir/humatad2.service"
  systemctl --user daemon-reload >/dev/null 2>&1
fi
line='{BOOT_LINE}'
{{ crontab -l 2>/dev/null | grep -v '# humatad2$'; printf '%s\n' "$line"; }} | crontab - || exit 1
stray
setsid "$bin" serve --role machine </dev/null >/dev/null 2>&1 &
sleep 2
daemon_running || {{ echo "humatad2 exited as soon as it started; run $bin serve --role machine there to see why" >&2; exit {DID_NOT_START}; }}
"#
        ),
        ServiceKind::Detached => format!(
            r#"bin="$HOME/{BIN_DIR}/humatad2"
if command -v systemctl >/dev/null 2>&1 && [ -f "$unit_dir/humatad2.service" ]; then
  systemctl --user disable --now humatad2.service >/dev/null 2>&1
  rm -f "$unit_dir/humatad2.service"
  systemctl --user daemon-reload >/dev/null 2>&1
fi
drop_cron || exit 1
stray
"$bin" serve --role machine --daemonize || {{ echo "humatad2 failed to daemonize; run $bin serve --role machine there to see why" >&2; exit {DID_NOT_START}; }}
sleep 2
daemon_running || {{ echo "humatad2 exited as soon as it started; run $bin serve --role machine there to see why" >&2; exit {DID_NOT_START}; }}
"#
        ),
    };
    format!("{HELPERS}{body}")
}

/// The script that stops humatad2 and removes what setup put in place: the
/// user service or crontab entry, and the files in `~/.humata2/bin`. State,
/// folders and lingering stay. It exits 0 when nothing was there.
pub(crate) fn remove_script() -> String {
    let files: Vec<String> = crate::install::InstalledFile::ALL
        .iter()
        .flat_map(|file| {
            let name = file.file_name();
            [
                format!("\"$HOME/{BIN_DIR}/{name}\""),
                format!("\"$HOME/{BIN_DIR}/{}\"", file.staged_name()),
                format!("\"$HOME/{BIN_DIR}/.{name}.next\""),
                format!("\"$HOME/{BIN_DIR}\"/.{name}.*.new"),
            ]
        })
        .chain([
            format!("\"$HOME/{BIN_DIR}/bundle.json\""),
            format!("\"$HOME/{BIN_DIR}/.bundle.json.next\""),
            format!("\"$HOME/{BIN_DIR}\"/.bundle.json.*.new"),
            format!("\"$HOME/{BIN_DIR}/humatad2.previous\""),
        ])
        .collect();
    let files = files.join(" ");
    format!(
        r#"{HELPERS}if command -v systemctl >/dev/null 2>&1 && [ -f "$unit_dir/humatad2.service" ]; then
  systemctl --user disable --now humatad2.service >/dev/null 2>&1
  rm -f "$unit_dir/humatad2.service" || exit 1
  systemctl --user daemon-reload >/dev/null 2>&1
fi
drop_cron || exit 1
stray
rm -f {files} || exit 1
rmdir "$HOME/{BIN_DIR}" 2>/dev/null
exit 0
"#
    )
}

/// The command an admin runs once so `user`'s services outlive their login
/// (PER-18).
pub(crate) fn linger_command(user: &str) -> String {
    match crate::quote::quote("the user name", user) {
        Ok(user) => format!("sudo loginctl enable-linger {user}"),
        Err(_) => "sudo loginctl enable-linger <user>".to_owned(),
    }
}

/// The command an admin runs once to start `user`'s systemd manager by
/// hand.
pub(crate) fn user_manager_command(user: &str) -> String {
    match crate::quote::quote("the user name", user) {
        Ok(user) => format!("sudo systemctl start \"user@$(id -u {user}).service\""),
        Err(_) => "sudo systemctl start user@<uid>.service".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;

    /// Stand-ins for every service tool the scripts touch, keeping their
    /// state as files in `state`, so the dev machine's own crontab, systemd
    /// and processes are never touched. `sleep` is shortened too.
    const FAKES: [(&str, &str); 7] = [
        (
            "systemctl",
            r#"case "$*" in
  *show-environment*) [ -e "$STATE/manager" ] ;;
  *is-active*) [ -e "$STATE/active" ] ;;
  *start*) [ -e "$STATE/broken" ] || touch "$STATE/active" ;;
  *stop*|*disable*) rm -f "$STATE/active" ;;
  *) exit 0 ;;
esac"#,
        ),
        (
            "loginctl",
            r#"case "$1" in
  show-user) [ -e "$STATE/linger" ] && echo yes || echo no ;;
  enable-linger) [ -e "$STATE/no_linger" ] || touch "$STATE/linger" ;;
esac"#,
        ),
        ("journalctl", "echo journal"),
        (
            "crontab",
            r#"if [ "$1" = -l ]; then [ -e "$STATE/crontab" ] && cat "$STATE/crontab"; else cat > "$STATE/crontab.new" && mv "$STATE/crontab.new" "$STATE/crontab"; fi"#,
        ),
        // pkill and pgrep also ask the real pgrep whether their pattern
        // matches the script that called them, as the real ones would. On
        // macOS pgrep skips its own ancestors unless given `-a`.
        (
            "pkill",
            r#"a=; [ "$(/usr/bin/uname)" = Darwin ] && a=-a
if /usr/bin/pgrep $a "$@" 2>/dev/null | grep -qx "$PPID"; then touch "$STATE/matched_self"; fi
rm -f "$STATE/running""#,
        ),
        (
            "pgrep",
            r#"a=; [ "$(/usr/bin/uname)" = Darwin ] && a=-a
if /usr/bin/pgrep $a "$@" 2>/dev/null | grep -qx "$PPID"; then touch "$STATE/matched_self"; fi
[ -e "$STATE/running" ]"#,
        ),
        (
            "setsid",
            r#"[ -e "$STATE/broken" ] || touch "$STATE/running"; touch "$STATE/setsid_ran""#,
        ),
    ];
    const REAL: [&str; 8] = ["id", "mkdir", "cat", "mv", "grep", "rm", "rmdir", "touch"];

    struct Machine {
        _folder: tempfile::TempDir,
        home: PathBuf,
        state: PathBuf,
        bin: PathBuf,
    }

    impl Machine {
        /// A home folder and a PATH holding only the fakes and harmless
        /// real tools; without `systemd` there is no `systemctl` at all.
        fn new(systemd: bool) -> Self {
            let folder = tempfile::tempdir().unwrap();
            let home = folder.path().join("home");
            let state = folder.path().join("state");
            let bin = folder.path().join("bin");
            for dir in [&home, &state, &bin] {
                fs::create_dir_all(dir).unwrap();
            }
            for (name, body) in FAKES {
                if !systemd && name == "systemctl" {
                    continue;
                }
                let path = bin.join(name);
                fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            }
            let real = |name: &str| {
                ["/usr/bin", "/bin"]
                    .iter()
                    .map(|dir| Path::new(dir).join(name))
                    .find(|path| path.exists())
                    .unwrap()
            };
            for name in REAL {
                symlink(real(name), bin.join(name)).unwrap();
            }
            // Short, except the scripts' 2-second settle, which waits until a
            // fake started with `&` has run (or a unit is active), however
            // loaded the machine is, bounded at 10 s.
            let sleep = bin.join("sleep");
            let real_sleep = real("sleep");
            fs::write(
                &sleep,
                format!(
                    r#"#!/bin/sh
if [ "$1" = 2 ]; then
  n=0
  until [ -e "$STATE/setsid_ran" ] || [ -e "$STATE/daemonize_ran" ] || [ -e "$STATE/active" ] || [ -e "$STATE/broken" ] || [ "$n" -ge 100 ]; do
    {real} 0.005; n=$((n + 1))
  done
  exit 0
fi
exec {real} 0.005
"#,
                    real = real_sleep.display()
                ),
            )
            .unwrap();
            fs::set_permissions(&sleep, fs::Permissions::from_mode(0o755)).unwrap();
            let installed = home.join(BIN_DIR);
            fs::create_dir_all(&installed).unwrap();
            let humatad_path = installed.join("humatad2");
            fs::write(
                &humatad_path,
                "#!/bin/sh\nif [ \"$*\" = \"serve --role machine --daemonize\" ]; then\n  touch \"$STATE/daemonize_ran\"\n  [ -e \"$STATE/broken\" ] && exit 1\n  touch \"$STATE/running\"\n  exit 0\nfi\nexit 1\n",
            )
            .unwrap();
            fs::set_permissions(&humatad_path, fs::Permissions::from_mode(0o755)).unwrap();
            fs::write(installed.join("humata2_holder"), "").unwrap();
            fs::write(installed.join("humata"), "").unwrap();
            fs::write(installed.join("stack"), "").unwrap();
            fs::write(installed.join("humata2_fusefs"), "").unwrap();
            fs::write(installed.join(".humata.staged"), "").unwrap();
            fs::write(installed.join(".humatad2.next"), "").unwrap();
            fs::write(installed.join("bundle.json"), "").unwrap();
            fs::write(installed.join(".bundle.json.next"), "").unwrap();
            fs::write(installed.join("humatad2.previous"), "").unwrap();
            fs::create_dir_all(home.join(".humata2/state")).unwrap();
            Self {
                _folder: folder,
                home,
                state,
                bin,
            }
        }

        fn set(&self, flag: &str) {
            fs::write(self.state.join(flag), "").unwrap();
        }

        fn has(&self, flag: &str) -> bool {
            self.state.join(flag).exists()
        }

        fn crontab(&self) -> String {
            fs::read_to_string(self.state.join("crontab")).unwrap_or_default()
        }

        fn unit(&self) -> PathBuf {
            self.home.join(".config/systemd/user/humatad2.service")
        }

        fn run(&self, script: &str) -> Option<i32> {
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(script)
                .env_clear()
                .env("HOME", &self.home)
                .env("PATH", &self.bin)
                .env("STATE", &self.state)
                .env("XDG_RUNTIME_DIR", &self.state)
                .output()
                .unwrap();
            // Shown when a test fails, to say why the script exited.
            eprintln!("{}", String::from_utf8_lossy(&output.stderr));
            output.status.code()
        }
    }

    const OTHER_CRON: &str = "0 3 * * * backup\n";

    impl Machine {
        /// What [`running_script`] says runs now.
        fn running(&self) -> String {
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(running_script())
                .env_clear()
                .env("HOME", &self.home)
                .env("PATH", &self.bin)
                .env("STATE", &self.state)
                .env("XDG_RUNTIME_DIR", &self.state)
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }
    }

    #[test]
    fn the_check_sees_how_the_daemon_runs_after_each_start_and_after_removal() {
        let machine = Machine::new(true);
        machine.set("manager");
        assert_eq!(machine.running(), "humata2.running=no");
        assert_eq!(machine.run(&script(ServiceKind::SystemdUser)), Some(0));
        assert_eq!(machine.running(), "humata2.running=systemd");

        fs::remove_file(machine.state.join("active")).unwrap();
        assert_eq!(machine.running(), "humata2.running=no");

        assert_eq!(machine.run(&script(ServiceKind::Setsid)), Some(0));
        assert_eq!(machine.running(), "humata2.running=setsid");

        assert_eq!(machine.run(&script(ServiceKind::Detached)), Some(0));
        assert_eq!(machine.running(), "humata2.running=detached");

        assert_eq!(machine.run(&remove_script()), Some(0));
        assert_eq!(machine.running(), "humata2.running=no");
    }

    #[test]
    fn no_script_takes_itself_for_the_daemon() {
        let machine = Machine::new(true);
        machine.set("manager");
        machine.set("running");
        assert_eq!(machine.run(&script(ServiceKind::SystemdUser)), Some(0));
        machine.running();
        assert_eq!(machine.run(&script(ServiceKind::Setsid)), Some(0));
        machine.running();
        assert_eq!(machine.run(&script(ServiceKind::Detached)), Some(0));
        machine.running();
        assert_eq!(machine.run(&remove_script()), Some(0));
        assert!(!machine.has("matched_self"));
    }

    #[test]
    fn the_scripts_are_valid_shell() {
        for text in [
            script(ServiceKind::SystemdUser),
            script(ServiceKind::Setsid),
            script(ServiceKind::Detached),
            remove_script(),
            running_script(),
        ] {
            let status = Command::new("/bin/sh").args(["-n", "-c", &text]).status().unwrap();
            assert!(status.success());
        }
    }

    #[test]
    fn a_user_service_replaces_a_stray_daemon_and_an_old_crontab_start() {
        let machine = Machine::new(true);
        machine.set("manager");
        machine.set("running");
        fs::write(
            machine.state.join("crontab"),
            format!("{OTHER_CRON}@reboot setsid old serve --role machine # humatad2\n"),
        )
        .unwrap();

        assert_eq!(machine.run(&script(ServiceKind::SystemdUser)), Some(0));

        assert!(machine.has("linger"));
        assert!(machine.has("active"));
        assert!(!machine.has("running"));
        assert_eq!(machine.crontab(), OTHER_CRON);
        assert!(fs::read_to_string(machine.unit()).unwrap().contains("serve --role machine"));
    }

    #[test]
    fn a_user_service_needs_lingering() {
        let machine = Machine::new(true);
        machine.set("manager");
        machine.set("no_linger");
        assert_eq!(machine.run(&script(ServiceKind::SystemdUser)), Some(LINGER_UNAVAILABLE));
        assert!(!machine.has("active"));
    }

    #[test]
    fn a_user_service_needs_the_user_manager() {
        let machine = Machine::new(true);
        assert_eq!(
            machine.run(&script(ServiceKind::SystemdUser)),
            Some(USER_MANAGER_UNAVAILABLE)
        );
    }

    #[test]
    fn a_service_that_never_comes_up_did_not_start() {
        let machine = Machine::new(true);
        machine.set("manager");
        machine.set("broken");
        assert_eq!(machine.run(&script(ServiceKind::SystemdUser)), Some(DID_NOT_START));

        let machine = Machine::new(false);
        machine.set("broken");
        assert_eq!(machine.run(&script(ServiceKind::Setsid)), Some(DID_NOT_START));

        let machine = Machine::new(false);
        machine.set("broken");
        assert_eq!(machine.run(&script(ServiceKind::Detached)), Some(DID_NOT_START));
    }

    #[test]
    fn detached_starts_humatad2_daemonize_without_systemd_setsid_or_crontab() {
        let machine = Machine::new(false);
        // Remove setsid and crontab completely from PATH to simulate a minimal container.
        fs::remove_file(machine.bin.join("setsid")).unwrap();
        fs::remove_file(machine.bin.join("crontab")).unwrap();

        assert_eq!(machine.run(&script(ServiceKind::Detached)), Some(0));
        assert!(machine.has("daemonize_ran"));
        assert!(machine.has("running"));
        assert_eq!(machine.running(), "humata2.running=detached");
        assert!(!machine.unit().exists());
        assert!(machine.crontab().is_empty());
    }

    #[test]
    fn detached_takes_over_from_an_earlier_user_service_and_crontab() {
        let machine = Machine::new(true);
        machine.set("manager");
        fs::write(
            machine.state.join("crontab"),
            format!("{OTHER_CRON}@reboot setsid old serve --role machine # humatad2\n"),
        )
        .unwrap();
        assert_eq!(machine.run(&script(ServiceKind::SystemdUser)), Some(0));

        assert_eq!(machine.run(&script(ServiceKind::Detached)), Some(0));

        assert!(!machine.unit().exists());
        assert!(!machine.has("active"));
        assert!(machine.has("daemonize_ran"));
        assert!(machine.has("running"));
        assert_eq!(machine.crontab(), OTHER_CRON);
        assert_eq!(machine.running(), "humata2.running=detached");
    }

    #[test]
    fn setsid_starts_the_daemon_and_adds_one_boot_entry() {
        let machine = Machine::new(false);
        fs::write(machine.state.join("crontab"), OTHER_CRON).unwrap();

        assert_eq!(machine.run(&script(ServiceKind::Setsid)), Some(0));
        assert_eq!(machine.run(&script(ServiceKind::Setsid)), Some(0));

        assert!(machine.has("running"));
        let crontab = machine.crontab();
        assert!(crontab.starts_with(OTHER_CRON));
        assert_eq!(crontab.lines().filter(|line| line.ends_with("# humatad2")).count(), 1);
    }

    #[test]
    fn setsid_takes_over_from_an_earlier_user_service() {
        let machine = Machine::new(true);
        machine.set("manager");
        assert_eq!(machine.run(&script(ServiceKind::SystemdUser)), Some(0));

        assert_eq!(machine.run(&script(ServiceKind::Setsid)), Some(0));

        assert!(!machine.unit().exists());
        assert!(!machine.has("active"));
        assert!(machine.has("running"));
    }

    #[test]
    fn removing_undoes_setup_and_keeps_state() {
        for systemd in [true, false] {
            let machine = Machine::new(systemd);
            machine.set("manager");
            fs::write(machine.state.join("crontab"), OTHER_CRON).unwrap();
            let kind = if systemd { ServiceKind::SystemdUser } else { ServiceKind::Setsid };
            assert_eq!(machine.run(&script(kind)), Some(0));

            assert_eq!(machine.run(&remove_script()), Some(0));
            assert_eq!(machine.run(&remove_script()), Some(0));

            assert!(!machine.has("active"));
            assert!(!machine.has("running"));
            assert!(!machine.unit().exists());
            assert_eq!(machine.crontab(), OTHER_CRON);
            assert!(!machine.home.join(BIN_DIR).join("humatad2").exists());
            assert!(!machine.home.join(BIN_DIR).join("humata2_holder").exists());
            assert!(!machine.home.join(BIN_DIR).join("humata").exists());
            assert!(!machine.home.join(BIN_DIR).join("stack").exists());
            assert!(!machine.home.join(BIN_DIR).join("humata2_fusefs").exists());
            assert!(!machine.home.join(BIN_DIR).join(".humata.staged").exists());
            // The release's manifest, what waits to replace it and what it
            // replaced go too.
            for name in [".humatad2.next", "bundle.json", ".bundle.json.next", "humatad2.previous"] {
                assert!(!machine.home.join(BIN_DIR).join(name).exists(), "{name} is still there");
            }
            assert!(machine.home.join(".humata2/state").is_dir());
        }
    }
}
