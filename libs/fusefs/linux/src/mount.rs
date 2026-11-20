//! Real Linux FUSE mounting through `fusermount3` and the `/dev/fuse` serve loop.
//!
//! The mount is made the standard unprivileged way: a `socketpair` is handed to
//! the setuid `fusermount3` through `_FUSE_COMMFD`, which opens `/dev/fuse`,
//! mounts it and sends the descriptor back with `SCM_RIGHTS`.
//!
//! `fusermount3` fills in `fd=`, `rootmode=`, `user_id=` and `group_id=` itself
//! and refuses to take them from the caller, so only the options below are passed.
//! The mount has no `allow_other`: only the mounting user can reach it.

use std::ffi::c_void;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use fusefs_vfs::Volume;
use thiserror::Error;

use crate::dispatcher::FuseDispatcher;
use crate::fuse_abi::{FuseErrno, FuseHeader, FuseInHeader};
use crate::wire::{self, Decoded, MAX_WRITE};

/// Options passed to `fusermount3 -o`.
const MOUNT_OPTIONS: &str = "fsname=humata2,subtype=humata2,default_permissions";

const FUSERMOUNT_NAMES: [&str; 2] = ["fusermount3", "fusermount"];
const FALLBACK_DIRS: [&str; 2] = ["/usr/bin", "/bin"];

/// Why mounting or unmounting through `fusermount3` failed.
#[derive(Debug, Error)]
pub enum MountError {
    /// Neither `fusermount3` nor `fusermount` could be found.
    #[error(
        "neither fusermount3 nor fusermount was found on PATH, /usr/bin or /bin. Install fuse3 (for example `apt install fuse3`) and make sure fusermount3 is setuid root."
    )]
    ToolMissing,
    /// `fusermount3` could not be started.
    #[error(
        "could not start {tool}: {source}. Make sure fuse3 is installed and {tool} is executable."
    )]
    Spawn {
        tool: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The socket pair used to receive `/dev/fuse` could not be made or read.
    #[error(
        "could not receive the /dev/fuse descriptor from fusermount3: {source}. Check that the process may create Unix sockets and that /dev/fuse exists and is usable."
    )]
    Socket {
        #[source]
        source: io::Error,
    },
    /// `fusermount3` exited without mounting.
    #[error(
        "{tool} failed ({status}): {stderr}. Install fuse3, make sure {tool} is setuid root (`ls -l {tool}`), make sure /dev/fuse is usable by this user, and make sure the mountpoint exists, is owned by this user and is not already mounted."
    )]
    Failed {
        tool: PathBuf,
        status: String,
        stderr: String,
    },
    /// `fusermount3 -u` failed.
    #[error(
        "{tool} could not unmount {mountpoint} ({status}): {stderr}. Close programs using the mount, or unmount lazily."
    )]
    Unmount {
        tool: PathBuf,
        mountpoint: PathBuf,
        status: String,
        stderr: String,
    },
}

/// Finds `fusermount3` (or the older `fusermount`) on PATH, then in `/usr/bin` and `/bin`.
fn find_fusermount() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    dirs.extend(FALLBACK_DIRS.iter().map(PathBuf::from));
    for name in FUSERMOUNT_NAMES {
        for dir in &dirs {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn cvt(rc: libc::c_int) -> io::Result<libc::c_int> {
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(rc) }
}

/// Receives one descriptor sent with `SCM_RIGHTS` on `sock`.
fn recv_fd(sock: RawFd) -> io::Result<Option<File>> {
    let mut data = [0u8; 1];
    let mut iov = libc::iovec {
        iov_base: data.as_mut_ptr().cast::<c_void>(),
        iov_len: data.len(),
    };
    // Aligned, and larger than CMSG_SPACE(sizeof(int)).
    let mut control = [0u64; 8];
    // SAFETY: zeroed msghdr is a valid empty message header.
    let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
    msg.msg_iov = &mut iov;
    msg.msg_iovlen = 1;
    msg.msg_control = control.as_mut_ptr().cast::<c_void>();
    msg.msg_controllen = std::mem::size_of_val(&control) as _;
    loop {
        // SAFETY: `msg` points at live buffers for the duration of the call.
        let n = unsafe { libc::recvmsg(sock, &mut msg, libc::MSG_CMSG_CLOEXEC) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        break;
    }
    // SAFETY: the kernel filled `msg` and `control` for this call.
    unsafe {
        let cmsg = libc::CMSG_FIRSTHDR(&msg);
        if cmsg.is_null()
            || (*cmsg).cmsg_level != libc::SOL_SOCKET
            || (*cmsg).cmsg_type != libc::SCM_RIGHTS
        {
            return Ok(None);
        }
        let fd = std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<libc::c_int>());
        Ok(Some(File::from_raw_fd(fd)))
    }
}

/// Mounts a FUSE filesystem at `mountpoint` and returns the opened `/dev/fuse`.
///
/// The mountpoint must exist and be owned by the calling user.
pub fn mount(mountpoint: &Path) -> Result<File, MountError> {
    let tool = find_fusermount().ok_or(MountError::ToolMissing)?;

    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: `fds` has room for the two descriptors socketpair writes.
    cvt(unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    })
    .map_err(|source| MountError::Socket { source })?;
    // SAFETY: both descriptors are fresh and owned here.
    let (parent_end, child_end) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };

    // A duplicate without CLOEXEC is what the child inherits.
    // SAFETY: plain fcntl on a descriptor we own.
    let inheritable = cvt(unsafe { libc::fcntl(child_end.as_raw_fd(), libc::F_DUPFD, 3) })
        .map_err(|source| MountError::Socket { source })?;
    // SAFETY: the fcntl result is a fresh descriptor.
    let inheritable = unsafe { OwnedFd::from_raw_fd(inheritable) };

    let child = Command::new(&tool)
        .arg("-o")
        .arg(MOUNT_OPTIONS)
        .arg("--")
        .arg(mountpoint)
        .env("_FUSE_COMMFD", inheritable.as_raw_fd().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| MountError::Spawn {
            tool: tool.clone(),
            source,
        })?;
    // Only the child may hold the sending end, so EOF shows when it exits.
    drop(inheritable);
    drop(child_end);

    let received = recv_fd(parent_end.as_raw_fd());
    let output = child
        .wait_with_output()
        .map_err(|source| MountError::Spawn {
            tool: tool.clone(),
            source,
        })?;
    let failed = |output: &std::process::Output| MountError::Failed {
        tool: tool.clone(),
        status: output.status.to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    };
    if !output.status.success() {
        return Err(failed(&output));
    }
    match received {
        Ok(Some(device)) => Ok(device),
        Ok(None) => Err(failed(&output)),
        Err(source) => Err(MountError::Socket { source }),
    }
}

/// Unmounts `mountpoint` with `fusermount3 -u`; `lazy` adds `-z` for a stale mount.
pub fn unmount(mountpoint: &Path, lazy: bool) -> Result<(), MountError> {
    let tool = find_fusermount().ok_or(MountError::ToolMissing)?;
    let mut cmd = Command::new(&tool);
    cmd.arg("-u");
    if lazy {
        cmd.arg("-z");
    }
    let output = cmd
        .arg("--")
        .arg(mountpoint)
        .stdin(Stdio::null())
        .output()
        .map_err(|source| MountError::Spawn {
            tool: tool.clone(),
            source,
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(MountError::Unmount {
        tool,
        mountpoint: mountpoint.to_path_buf(),
        status: output.status.to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

/// Serves `/dev/fuse` frames through `dispatcher` until the filesystem is unmounted.
///
/// Returns `Ok(())` when the kernel reports the mount gone (`ENODEV`), and the
/// I/O error otherwise. Requests are served one at a time.
pub fn serve<V: Volume>(device: File, dispatcher: FuseDispatcher<V>) -> io::Result<()> {
    let mut buf = vec![0u8; MAX_WRITE as usize + 4096];
    loop {
        let n = match (&device).read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => n,
            Err(err) => match err.raw_os_error() {
                Some(libc::EINTR | libc::EAGAIN | libc::ENOENT) => continue,
                Some(libc::ENODEV) => return Ok(()),
                _ => return Err(err),
            },
        };
        let Some(reply) = handle_frame(&dispatcher, &buf[..n]) else {
            continue;
        };
        match (&device).write(&reply) {
            Ok(_) => {}
            Err(err) => match err.raw_os_error() {
                // The request was interrupted before the reply was written.
                Some(libc::ENOENT | libc::EINTR | libc::EAGAIN) => {}
                Some(libc::ENODEV) => return Ok(()),
                _ => return Err(err),
            },
        }
    }
}

/// Decodes, dispatches and encodes one frame; `None` means no reply is due.
fn handle_frame<V: Volume>(dispatcher: &FuseDispatcher<V>, frame: &[u8]) -> Option<Vec<u8>> {
    let (header, decoded) = match wire::decode_frame(frame) {
        Ok(ok) => ok,
        Err(err) => {
            let unique = FuseInHeader::decode(frame)?.unique;
            return Some(wire::encode_error(unique, err.errno()));
        }
    };
    match decoded {
        Decoded::Init(init) => Some(wire::encode_init_reply(header.unique, &init)),
        Decoded::Ack => Some(wire::encode_ack(header.unique)),
        Decoded::NoReply => None,
        Decoded::Unsupported(_) => Some(wire::encode_unsupported(header.unique)),
        Decoded::Dispatch(request, shape) => {
            let typed = FuseHeader::new(
                header.unique,
                fusefs_vfs::NodeId(header.nodeid),
                header.uid,
                header.gid,
                header.pid,
            );
            let response = dispatcher.dispatch(&typed, request);
            let result: Result<_, FuseErrno> = response.result;
            Some(wire::encode_reply(
                header.unique,
                &shape,
                (header.uid, header.gid),
                result,
            ))
        }
    }
}
