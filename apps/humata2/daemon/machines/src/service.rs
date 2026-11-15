//! Keeping humatad2 running on a Linux machine: a systemd user service with
//! lingering (PER-18), or `setsid` plus a crontab `@reboot` entry where
//! there is no systemd (PER-19).

use crate::check::Facts;
use crate::link::BIN_DIR;
use crate::runner::Exit;

/// The service script's exit codes for what it found wrong.
pub(crate) const LINGER_UNAVAILABLE: i32 = 75;
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
}

impl ServiceKind {
    /// The kind the machine supports, systemd first.
    pub(crate) fn for_machine(facts: &Facts) -> Option<Self> {
        if facts.systemd {
            Some(Self::SystemdUser)
        } else if facts.setsid && facts.crontab {
            Some(Self::Setsid)
        } else {
            None
        }
    }
}

/// What the service script's exit says.
pub(crate) enum ServiceOutcome {
    Running,
    LingerUnavailable,
    DidNotStart,
    Failed(Exit),
}

impl ServiceOutcome {
    pub(crate) fn from_exit(exit: Exit) -> Self {
        match exit {
            Exit::Code(0) => Self::Running,
            Exit::Code(LINGER_UNAVAILABLE) => Self::LingerUnavailable,
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

/// The script that installs and (re)starts the service, so a new humatad2
/// takes over. It exits 0 once humatad2 is running.
pub(crate) fn script(kind: ServiceKind) -> String {
    match kind {
        ServiceKind::SystemdUser => format!(
            r#"set -u
export XDG_RUNTIME_DIR="${{XDG_RUNTIME_DIR:-/run/user/$(id -u)}}"
u=$(id -un)
if [ "$(loginctl show-user "$u" -p Linger --value 2>/dev/null)" != yes ]; then
  loginctl enable-linger "$u" 2>/dev/null || {{ echo "lingering can't be turned on for $u" >&2; exit {LINGER_UNAVAILABLE}; }}
fi
dir="${{XDG_CONFIG_HOME:-$HOME/.config}}/systemd/user"
mkdir -p "$dir" || exit 1
cat > "$dir/humatad2.service.new" <<'HUMATA_UNIT'
{UNIT}HUMATA_UNIT
mv -f "$dir/humatad2.service.new" "$dir/humatad2.service" || exit 1
systemctl --user daemon-reload || exit 1
systemctl --user enable humatad2.service || exit 1
systemctl --user restart humatad2.service || exit 1
sleep 1
if ! systemctl --user is-active --quiet humatad2.service; then
  journalctl --user -u humatad2.service -n 20 --no-pager >&2 2>/dev/null || systemctl --user status humatad2.service --no-pager >&2
  exit {DID_NOT_START}
fi
"#
        ),
        ServiceKind::Setsid => format!(
            r#"set -u
bin="$HOME/{BIN_DIR}/humatad2"
# The bracket keeps pkill and pgrep from matching this script itself.
line='@reboot setsid "$HOME/{BIN_DIR}/humatad2" serve --role machine </dev/null >/dev/null 2>&1 # humatad2'
{{ crontab -l 2>/dev/null | grep -v '# humatad2$'; printf '%s\n' "$line"; }} | crontab - || exit 1
pkill -u "$(id -u)" -f '[h]umatad2 serve' 2>/dev/null
sleep 1
setsid "$bin" serve --role machine </dev/null >/dev/null 2>&1 &
sleep 1
pgrep -u "$(id -u)" -f '[h]umatad2 serve' >/dev/null || {{ echo "humatad2 exited as soon as it started; run $bin serve --role machine there to see why" >&2; exit {DID_NOT_START}; }}
"#
        ),
    }
}

/// The command an admin runs once so `user`'s services outlive their login
/// (PER-18).
pub(crate) fn linger_command(user: &str) -> String {
    match crate::quote::quote("the user name", user) {
        Ok(user) => format!("sudo loginctl enable-linger {user}"),
        Err(_) => "sudo loginctl enable-linger <user>".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_scripts_are_valid_shell() {
        for kind in [ServiceKind::SystemdUser, ServiceKind::Setsid] {
            let status = std::process::Command::new("/bin/sh")
                .args(["-n", "-c", &script(kind)])
                .status()
                .unwrap();
            assert!(status.success(), "{kind:?}");
        }
    }
}
