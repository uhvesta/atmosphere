//! D398: how `serve` says it listens. Whoever started it waits on that
//! word, never on its socket: `READY=1` on the `--ready-fd` pipe its
//! starter passed in, and the same datagram to `NOTIFY_SOCKET` when a
//! service manager set one (systemd's `Type=notify`).

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::Write;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixDatagram;
use std::path::PathBuf;

use rustix::io::{FdFlags, fcntl_setfd};

/// What a starter reads, and what systemd's notify protocol takes.
const READY: &[u8] = b"READY=1";

/// The service manager's socket, if one is set. It stays in the
/// environment: systemd takes notices only from the daemon's own pid
/// (`NotifyAccess=main`), and a relaunch by exec keeps that pid and needs
/// the socket to say it is ready again.
pub(crate) fn notify_socket() -> Option<OsString> {
    std::env::var_os("NOTIFY_SOCKET").filter(|socket| !socket.is_empty())
}

/// Takes the pipe the starter passed in as `fd`. It is closed on exec, so
/// the holders and agents the daemon starts can't keep the starter waiting.
pub(crate) fn take_pipe(fd: i32) -> Result<File, ReadyError> {
    // SAFETY: the starter passed this descriptor for the daemon alone, and
    // nothing else here takes it.
    let pipe = unsafe { OwnedFd::from_raw_fd(fd) };
    fcntl_setfd(pipe.as_fd(), FdFlags::CLOEXEC).map_err(|source| ReadyError::Pipe {
        fd,
        source: source.into(),
    })?;
    Ok(File::from(pipe))
}

/// Leaves `pipe` open across an exec, under the same descriptor number,
/// for a run that starts before this one said it listens (D439): the
/// rollback's relaunch of the old release, which tells the starter in its
/// place.
pub(crate) fn hand_on(pipe: &File) -> Result<(), ReadyError> {
    fcntl_setfd(pipe.as_fd(), FdFlags::empty()).map_err(|source| ReadyError::HandOn {
        source: source.into(),
    })
}

/// Says the daemon listens: on the pipe, which is closed after, and to the
/// service manager. A starter that has gone meanwhile has nothing to hear,
/// so a pipe nobody reads is not an error; a service manager that doesn't
/// hear is.
pub(crate) fn tell(pipe: Option<File>, notify: Option<&OsStr>) -> Result<(), ReadyError> {
    if let Some(mut pipe) = pipe {
        let _ = pipe.write_all(READY).and_then(|()| pipe.write_all(b"\n"));
    }
    let Some(socket) = notify else {
        return Ok(());
    };
    send(socket).map_err(|source| ReadyError::Notify {
        socket: PathBuf::from(socket),
        source,
    })
}

fn send(socket: &OsStr) -> std::io::Result<()> {
    let datagram = UnixDatagram::unbound()?;
    #[cfg(target_os = "linux")]
    {
        use std::os::linux::net::SocketAddrExt;
        use std::os::unix::ffi::OsStrExt;
        if let Some(name) = socket.as_bytes().strip_prefix(b"@") {
            let address = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
            return datagram.send_to_addr(READY, &address).map(drop);
        }
    }
    datagram.send_to(READY, socket).map(drop)
}

/// Why the daemon couldn't say it listens.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ReadyError {
    /// The `--ready-fd` descriptor isn't one this process holds.
    #[error(
        "--ready-fd {fd} isn't an open descriptor: start humatad2 from the app or `humatad2 bridge`, which pass it, or leave the flag out"
    )]
    Pipe {
        fd: i32,
        #[source]
        source: std::io::Error,
    },
    /// The pipe couldn't be left open for the run an exec starts.
    #[error(
        "couldn't keep the --ready-fd pipe open for the relaunched humatad2, so its starter won't hear it is ready: it is running anyway; restart it from the app or `humatad2 bridge` if the starter gave up"
    )]
    HandOn {
        #[source]
        source: std::io::Error,
    },
    /// The service manager's socket refused the notice.
    #[error(
        "couldn't tell the service manager at NOTIFY_SOCKET={} that humatad2 is ready, so it will count the start as failed and stop it: check the unit is Type=notify and the socket is the one it set",
        socket.display()
    )]
    Notify {
        socket: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::time::Duration;

    use super::*;

    fn socket_dir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("h2rd")
            .tempdir_in("/tmp")
            .expect("make a socket folder")
    }

    #[test]
    fn the_service_manager_hears_ready() {
        let dir = socket_dir();
        let path = dir.path().join("notify");
        let manager = UnixDatagram::bind(&path).expect("bind the manager's socket");
        manager
            .set_read_timeout(Some(Duration::from_secs(30)))
            .expect("set a read deadline");
        tell(None, Some(path.as_os_str())).expect("the notice is sent");
        let mut got = [0u8; 64];
        let n = manager.recv(&mut got).expect("the manager hears the notice");
        assert_eq!(&got[..n], READY);
    }

    #[test]
    fn the_starter_hears_ready_and_then_the_pipe_closes() {
        let (mut reader, writer) = std::io::pipe().expect("make a pipe");
        tell(Some(File::from(OwnedFd::from(writer))), None).expect("telling the pipe never fails");
        let mut got = Vec::new();
        reader.read_to_end(&mut got).expect("the pipe ends");
        assert!(got.starts_with(READY), "the starter reads READY=1: {got:?}");
    }

    #[test]
    fn a_starter_that_has_gone_is_not_an_error() {
        let (reader, writer) = std::io::pipe().expect("make a pipe");
        drop(reader);
        assert!(tell(Some(File::from(OwnedFd::from(writer))), None).is_ok());
    }

    #[test]
    fn a_service_manager_nobody_listens_on_is_an_error() {
        let dir = socket_dir();
        let path = dir.path().join("nobody");
        assert!(matches!(
            tell(None, Some(path.as_os_str())),
            Err(ReadyError::Notify { .. })
        ));
    }

    #[test]
    fn the_pipe_is_closed_on_exec() {
        let (_reader, writer) = std::io::pipe().expect("make a pipe");
        let fd = std::os::fd::IntoRawFd::into_raw_fd(OwnedFd::from(writer));
        let pipe = take_pipe(fd).expect("take the pipe");
        let flags = rustix::io::fcntl_getfd(&pipe).expect("read its flags");
        assert!(flags.contains(FdFlags::CLOEXEC));
    }

    #[test]
    fn a_handed_on_pipe_stays_open_across_exec() {
        let (_reader, writer) = std::io::pipe().expect("make a pipe");
        let fd = std::os::fd::IntoRawFd::into_raw_fd(OwnedFd::from(writer));
        let pipe = take_pipe(fd).expect("take the pipe");
        hand_on(&pipe).expect("hand the pipe on");
        let flags = rustix::io::fcntl_getfd(&pipe).expect("read its flags");
        assert!(!flags.contains(FdFlags::CLOEXEC));
    }
}
