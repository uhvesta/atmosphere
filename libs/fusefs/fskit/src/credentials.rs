//! Caller credential extraction from Darwin `audit_token_t` and Unix sockets.

use std::io;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

use fusefs_vfs::CallerContext;
use serde::{Deserialize, Serialize};

/// Portable representation of Darwin's `audit_token_t` (`val: [u32; 8]`).
///
/// Field layout from `<bsm/audit.h>`:
/// - `[0]`: `auid` (audit user ID)
/// - `[1]`: `euid` (effective UID)
/// - `[2]`: `egid` (effective GID)
/// - `[3]`: `ruid` (real UID)
/// - `[4]`: `rgid` (real GID)
/// - `[5]`: `pid` (process ID)
/// - `[6]`: `asid` (audit session ID)
/// - `[7]`: `pidversion`
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditToken {
    pub val: [u32; 8],
}

impl AuditToken {
    /// Constructs an [`AuditToken`] from `(pid, euid, egid)`.
    pub const fn from_ids(pid: u32, euid: u32, egid: u32) -> Self {
        Self {
            val: [euid, euid, egid, euid, egid, pid, 0, 0],
        }
    }

    pub const fn euid(&self) -> u32 {
        self.val[1]
    }

    pub const fn egid(&self) -> u32 {
        self.val[2]
    }

    pub const fn pid(&self) -> u32 {
        self.val[5]
    }

    /// Converts this audit token into a [`PeerCredentials`] record.
    pub const fn to_peer_credentials(&self) -> PeerCredentials {
        PeerCredentials {
            pid: self.pid(),
            euid: self.euid(),
            egid: self.egid(),
            via_audit_token: true,
        }
    }

    /// Converts this audit token into a [`CallerContext`].
    pub fn to_caller_context(&self, capability_root: Option<String>) -> CallerContext {
        let mut ctx = CallerContext::new(self.pid(), self.euid(), self.egid());
        if let Some(root) = capability_root {
            ctx = ctx.with_capability_root(root);
        }
        ctx
    }
}

/// Caller identity extracted from a Unix domain socket peer or an `audit_token_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerCredentials {
    pub pid: u32,
    pub euid: u32,
    pub egid: u32,
    pub via_audit_token: bool,
}

impl PeerCredentials {
    /// Converts these peer credentials into a [`CallerContext`].
    pub fn to_caller_context(&self, capability_root: Option<String>) -> CallerContext {
        let mut ctx = CallerContext::new(self.pid, self.euid, self.egid);
        if let Some(root) = capability_root {
            ctx = ctx.with_capability_root(root);
        }
        ctx
    }
}

/// Extracts peer PID/EUID/EGID from a connected Unix domain socket.
///
/// On macOS, queries `SOL_LOCAL` + `LOCAL_PEERTOKEN` (`audit_token_t`) first and
/// falls back to `LOCAL_PEERPID` + `getpeereid`.
#[cfg(target_os = "macos")]
pub fn extract_peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    const SOL_LOCAL: libc::c_int = 0;
    const LOCAL_PEERPID: libc::c_int = 0x002;
    const LOCAL_PEERTOKEN: libc::c_int = 0x006;

    #[repr(C)]
    #[derive(Default)]
    struct RawAuditToken {
        val: [u32; 8],
    }

    let fd = stream.as_raw_fd();
    let mut raw = RawAuditToken::default();
    let mut raw_len = std::mem::size_of::<RawAuditToken>() as libc::socklen_t;

    // SAFETY: `fd` is a valid connected Unix stream socket and `raw`/`raw_len` match
    // Darwin's `audit_token_t` layout for `LOCAL_PEERTOKEN`.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEERTOKEN,
            (&raw mut raw).cast::<libc::c_void>(),
            &raw mut raw_len,
        )
    };
    if rc == 0 && raw_len as usize == std::mem::size_of::<RawAuditToken>() {
        return Ok(AuditToken { val: raw.val }.to_peer_credentials());
    }

    let mut pid: libc::pid_t = 0;
    let mut pid_len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: `fd` is a valid Unix stream socket and `pid` holds `pid_t`.
    let pid_rc = unsafe {
        libc::getsockopt(
            fd,
            SOL_LOCAL,
            LOCAL_PEERPID,
            (&raw mut pid).cast::<libc::c_void>(),
            &raw mut pid_len,
        )
    };
    if pid_rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let mut euid: libc::uid_t = 0;
    let mut egid: libc::gid_t = 0;
    // SAFETY: `fd` is a valid Unix stream socket.
    let eid_rc = unsafe { libc::getpeereid(fd, &raw mut euid, &raw mut egid) };
    if eid_rc != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(PeerCredentials {
        pid: pid.max(0) as u32,
        euid,
        egid,
        via_audit_token: false,
    })
}

/// Extracts peer PID/EUID/EGID on Linux via `SO_PEERCRED`.
#[cfg(target_os = "linux")]
pub fn extract_peer_credentials(stream: &UnixStream) -> io::Result<PeerCredentials> {
    let fd = stream.as_raw_fd();
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `fd` is a valid Unix stream socket and `cred` matches `SO_PEERCRED`.
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut cred).cast::<libc::c_void>(),
            &raw mut len,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(PeerCredentials {
        pid: cred.pid.max(0) as u32,
        euid: cred.uid,
        egid: cred.gid,
        via_audit_token: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_token_extracts_pid_euid_egid_and_builds_caller_context() {
        let token = AuditToken::from_ids(9001, 501, 20);
        assert_eq!(token.pid(), 9001);
        assert_eq!(token.euid(), 501);
        assert_eq!(token.egid(), 20);

        let ctx = token.to_caller_context(Some("/work/repo".into()));
        assert_eq!(ctx.pid, 9001);
        assert_eq!(ctx.uid, 501);
        assert_eq!(ctx.gid, 20);
        assert_eq!(ctx.capability_root.as_deref(), Some("/work/repo"));
    }
}
