//! Which terminal session a process is in, and what it descends from
//! (BOUNDARIES.md §2.2), read from the kernel: `/proc` on Linux,
//! `proc_pidinfo` on macOS. The rules built on it take a `Processes`, so
//! tests can give them processes of their own.

use std::collections::HashSet;
use std::io;

/// What the daemon reads of one process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Process {
    pub(crate) parent: i32,
    /// Its session's id: the session leader's pid.
    pub(crate) session: i32,
    /// Its controlling terminal's device, if it has one.
    pub(crate) terminal: Option<u64>,
    /// When it started, in the kernel's own units: only ever compared.
    pub(crate) started: u64,
    pub(crate) name: String,
}

/// Where processes are read from.
pub(crate) trait Processes: Send + Sync {
    fn process(&self, pid: i32) -> io::Result<Process>;

    /// A terminal device's name, as `ttys004` or `pts/3`.
    fn terminal_name(&self, device: u64) -> Option<String>;
}

/// The terminal session a CLI login binds: a process is in it when it has
/// the same session id, the same controlling terminal, and the session's
/// leader is the same process that was there at login. A process that
/// calls `setsid` leaves it (SEC-11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalSession {
    pub(crate) session: i32,
    pub(crate) leader_started: u64,
    pub(crate) terminal: u64,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum TerminalSessionError {
    #[error("couldn't read process {pid} from the kernel")]
    Read {
        pid: i32,
        #[source]
        source: io::Error,
    },
    #[error("process {pid} has no controlling terminal")]
    NoTerminal { pid: i32 },
    #[error("the process that started terminal session {session} has exited")]
    LeaderGone {
        session: i32,
        #[source]
        source: io::Error,
    },
}

/// The terminal session `pid` is in.
pub(crate) fn terminal_session(
    processes: &dyn Processes,
    pid: i32,
) -> Result<TerminalSession, TerminalSessionError> {
    let process = processes
        .process(pid)
        .map_err(|source| TerminalSessionError::Read { pid, source })?;
    let terminal = process
        .terminal
        .ok_or(TerminalSessionError::NoTerminal { pid })?;
    let leader =
        processes
            .process(process.session)
            .map_err(|source| TerminalSessionError::LeaderGone {
                session: process.session,
                source,
            })?;
    Ok(TerminalSession {
        session: process.session,
        leader_started: leader.started,
        terminal,
    })
}

/// Whether `pid`, or the leader of its terminal session, is `holders` or
/// descends from one: a process a session's holder started, even one that
/// a double fork left with init as its parent, as long as it stayed in
/// that holder's terminal session.
pub(crate) fn descends_from(
    processes: &dyn Processes,
    pid: i32,
    session: &TerminalSession,
    holders: &HashSet<i32>,
) -> bool {
    [pid, session.session]
        .into_iter()
        .any(|start| lineage(processes, start).any(|pid| holders.contains(&pid)))
}

/// `pid`, then its parent and theirs, up to init. Stops early at a
/// process it can't read, which has exited.
pub(crate) fn lineage(processes: &dyn Processes, pid: i32) -> impl Iterator<Item = i32> + '_ {
    // More generations than any system has; guards against a loop.
    const GENERATIONS_MAX: usize = 4096;
    std::iter::successors(Some(pid), move |&pid| {
        let parent = processes.process(pid).ok()?.parent;
        (parent > 0 && parent != pid).then_some(parent)
    })
    .take(GENERATIONS_MAX)
}

/// The terminal as a confirmation names it: "Terminal, ttys004", after
/// what started the session's leader and the device.
pub(crate) fn describe(processes: &dyn Processes, session: &TerminalSession) -> String {
    let device = processes
        .terminal_name(session.terminal)
        .unwrap_or_else(|| format!("terminal {}", session.terminal));
    let opener = processes
        .process(session.session)
        .and_then(|leader| processes.process(leader.parent))
        .map(|opener| opener.name)
        .ok()
        .filter(|name| !name.is_empty());
    match opener {
        Some(opener) => format!("{opener}, {device}"),
        None => device,
    }
}

/// The running system's processes.
pub(crate) struct Kernel;

#[cfg(target_os = "linux")]
impl Processes for Kernel {
    fn process(&self, pid: i32) -> io::Result<Process> {
        linux::process(pid)
    }

    fn terminal_name(&self, device: u64) -> Option<String> {
        Some(linux::terminal_name(device))
    }
}

#[cfg(target_os = "macos")]
impl Processes for Kernel {
    fn process(&self, pid: i32) -> io::Result<Process> {
        macos::process(pid)
    }

    fn terminal_name(&self, device: u64) -> Option<String> {
        macos::terminal_name(device)
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
impl Processes for Kernel {
    fn process(&self, _pid: i32) -> io::Result<Process> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "humatad2 reads processes on Linux and macOS only",
        ))
    }

    fn terminal_name(&self, _device: u64) -> Option<String> {
        None
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;

    use super::Process;

    fn bad(pid: i32) -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("/proc/{pid}/stat isn't in the format Linux documents"),
        )
    }

    /// From `/proc/<pid>/stat` (proc(5)). The name is in parentheses and
    /// may hold spaces and parentheses itself, so fields are counted from
    /// the last `)`.
    pub(super) fn process(pid: i32) -> io::Result<Process> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let (head, rest) = stat.rsplit_once(')').ok_or_else(|| bad(pid))?;
        let name = head.split_once('(').map_or("", |(_, name)| name);
        // Field 3, the state, is fields[0].
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let field = |number: usize| -> io::Result<i64> {
            fields
                .get(number - 3)
                .and_then(|text| text.parse().ok())
                .ok_or_else(|| bad(pid))
        };
        let terminal = field(7)?;
        Ok(Process {
            parent: i32::try_from(field(4)?).map_err(|_| bad(pid))?,
            session: i32::try_from(field(6)?).map_err(|_| bad(pid))?,
            terminal: (terminal != 0).then(|| u64::from(terminal as u32)),
            started: u64::try_from(field(22)?).map_err(|_| bad(pid))?,
            name: name.to_owned(),
        })
    }

    /// The name of the device `tty_nr` encodes, as `ls /dev` shows it.
    pub(super) fn terminal_name(device: u64) -> String {
        let major = (device >> 8) & 0xfff;
        let minor = (device & 0xff) | ((device >> 12) & 0xfff00);
        match major {
            136..=143 => format!("pts/{}", (major - 136) * 256 + minor),
            4 if minor < 64 => format!("tty{minor}"),
            4 => format!("ttyS{}", minor - 64),
            _ => format!("tty {major}:{minor}"),
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::io;

    use rustix::process::{Pid, getsid};

    use super::Process;

    /// `PROC_PIDTBSDINFO` in <sys/proc_info.h>.
    const PROC_PIDTBSDINFO: c_int = 3;
    /// `NODEV`: no controlling terminal.
    const NODEV: u32 = u32::MAX;
    /// `S_IFCHR`: a character device.
    const S_IFCHR: u16 = 0o020000;

    /// `struct proc_bsdinfo` in <sys/proc_info.h>. Every field is there for
    /// the layout; only some are read.
    #[repr(C)]
    #[derive(Default)]
    #[allow(dead_code)]
    struct BsdInfo {
        flags: u32,
        status: u32,
        xstatus: u32,
        pid: u32,
        ppid: u32,
        uid: u32,
        gid: u32,
        ruid: u32,
        rgid: u32,
        svuid: u32,
        svgid: u32,
        rfu_1: u32,
        comm: [c_char; 16],
        name: [c_char; 32],
        nfiles: u32,
        pgid: u32,
        pjobc: u32,
        tdev: u32,
        tpgid: u32,
        nice: i32,
        start_tvsec: u64,
        start_tvusec: u64,
    }

    // In libSystem, which every macOS program links.
    unsafe extern "C" {
        fn proc_pidinfo(
            pid: c_int,
            flavor: c_int,
            arg: u64,
            buffer: *mut c_void,
            buffersize: c_int,
        ) -> c_int;
        fn devname_r(dev: i32, kind: u16, buffer: *mut c_char, len: c_int) -> *mut c_char;
    }

    pub(super) fn process(pid: i32) -> io::Result<Process> {
        // Pid 0 is the kernel, and getsid would read a missing pid as ours.
        let Some(id) = Pid::from_raw(pid) else {
            return Err(io::Error::from(io::ErrorKind::NotFound));
        };
        let mut info = BsdInfo::default();
        let size = c_int::try_from(size_of::<BsdInfo>()).map_err(io::Error::other)?;
        // SAFETY: `info` is a live, writable `proc_bsdinfo` of `size`
        // bytes, which is all proc_pidinfo writes for this flavor.
        let written = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDTBSDINFO,
                0,
                (&raw mut info).cast::<c_void>(),
                size,
            )
        };
        if written <= 0 {
            return Err(io::Error::last_os_error());
        }
        if written != size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "proc_pidinfo answered with a proc_bsdinfo of another size",
            ));
        }
        let session = getsid(Some(id))?;
        let name = info
            .comm
            .iter()
            .take_while(|byte| **byte != 0)
            .map(|byte| *byte as u8);
        Ok(Process {
            parent: i32::try_from(info.ppid).map_err(io::Error::other)?,
            session: session.as_raw_pid(),
            terminal: (info.tdev != NODEV && info.tdev != 0).then_some(u64::from(info.tdev)),
            started: info
                .start_tvsec
                .saturating_mul(1_000_000)
                .saturating_add(info.start_tvusec),
            name: String::from_utf8_lossy(&name.collect::<Vec<u8>>()).into_owned(),
        })
    }

    pub(super) fn terminal_name(device: u64) -> Option<String> {
        let device = i32::from_ne_bytes(u32::try_from(device).ok()?.to_ne_bytes());
        let mut buffer: [c_char; 64] = [0; 64];
        // SAFETY: devname_r writes at most `len` bytes, NUL included, into
        // `buffer`, and returns it or a static string, or null.
        let name = unsafe { devname_r(device, S_IFCHR, buffer.as_mut_ptr(), 64) };
        if name.is_null() {
            return None;
        }
        // SAFETY: devname_r returned a NUL-terminated string that lives at
        // least as long as `buffer`.
        let name = unsafe { CStr::from_ptr(name) };
        Some(name.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;

    use super::*;

    /// Processes by pid, as a test lays them out.
    #[derive(Default)]
    pub(crate) struct Fake {
        pub(crate) processes: HashMap<i32, Process>,
    }

    impl Fake {
        pub(crate) fn add(&mut self, pid: i32, parent: i32, session: i32, terminal: Option<u64>) {
            self.processes.insert(
                pid,
                Process {
                    parent,
                    session,
                    terminal,
                    started: u64::try_from(pid).unwrap() * 10,
                    name: format!("p{pid}"),
                },
            );
        }
    }

    impl Processes for Fake {
        fn process(&self, pid: i32) -> io::Result<Process> {
            self.processes
                .get(&pid)
                .cloned()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
        }

        fn terminal_name(&self, device: u64) -> Option<String> {
            Some(format!("tty{device}"))
        }
    }

    /// A terminal app (10) that opened session 20 on terminal 7, a shell
    /// (21) in it, and a command (22) the shell ran.
    pub(crate) fn terminal() -> Fake {
        let mut fake = Fake::default();
        fake.add(1, 0, 1, None);
        fake.add(10, 1, 10, None);
        fake.add(20, 10, 20, Some(7));
        fake.add(21, 20, 20, Some(7));
        fake.add(22, 21, 20, Some(7));
        fake
    }

    #[test]
    fn processes_of_one_terminal_session_share_it() {
        let fake = terminal();

        let shell = terminal_session(&fake, 21).unwrap();
        let command = terminal_session(&fake, 22).unwrap();

        assert_eq!(shell, command);
    }

    #[test]
    fn a_process_that_called_setsid_is_in_another_session() {
        let mut fake = terminal();
        // A child of the command that started a session of its own; it
        // keeps no controlling terminal.
        fake.add(30, 22, 30, None);
        // One that also opened a terminal for its new session.
        fake.add(31, 22, 31, Some(8));

        assert!(matches!(
            terminal_session(&fake, 30),
            Err(TerminalSessionError::NoTerminal { pid: 30 })
        ));
        assert_ne!(
            terminal_session(&fake, 31).unwrap(),
            terminal_session(&fake, 22).unwrap()
        );
    }

    #[test]
    fn a_new_leader_with_a_reused_pid_is_another_session() {
        let mut fake = terminal();
        let before = terminal_session(&fake, 22).unwrap();
        fake.processes.get_mut(&20).unwrap().started += 1;

        assert_ne!(terminal_session(&fake, 22).unwrap(), before);
    }

    #[test]
    fn a_session_whose_leader_exited_has_no_binding() {
        let mut fake = terminal();
        fake.processes.remove(&20);

        assert!(matches!(
            terminal_session(&fake, 22),
            Err(TerminalSessionError::LeaderGone { session: 20, .. })
        ));
    }

    #[test]
    fn a_process_under_a_holder_descends_from_it() {
        let fake = terminal();
        let session = terminal_session(&fake, 22).unwrap();

        assert!(descends_from(&fake, 22, &session, &HashSet::from([10])));
        assert!(!descends_from(&fake, 22, &session, &HashSet::from([99])));
    }

    #[test]
    fn a_double_forked_process_is_still_found_through_its_session_leader() {
        let mut fake = terminal();
        // Reparented to init, but still in the holder's terminal session.
        fake.add(40, 1, 20, Some(7));
        let session = terminal_session(&fake, 40).unwrap();

        assert!(descends_from(&fake, 40, &session, &HashSet::from([10])));
    }
}
