//! Linux mount backend: mounts through `fusermount3` and serves the volume
//! from a thread of this process, so no system mount tool or root is needed.

use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::sync::Arc;

use fusefs_host::{MountBackend, MountConfig};
use fusefs_linux::FuseDispatcher;
use fusefs_linux::mount;

use crate::layout::HumataVolumeHost;

/// Mounts the Humata volume as a private FUSE filesystem of the current user.
pub struct LinuxFuseBackend {
    host: Arc<HumataVolumeHost>,
}

impl LinuxFuseBackend {
    pub fn new(host: Arc<HumataVolumeHost>) -> Self {
        Self { host }
    }
}

fn to_io(err: mount::MountError) -> io::Error {
    io::Error::other(err)
}

impl MountBackend for LinuxFuseBackend {
    fn mount(&self, config: &MountConfig) -> io::Result<Option<u32>> {
        if let Err(err) = fs::metadata(&config.mountpoint)
            && err.kind() == io::ErrorKind::NotFound
        {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&config.mountpoint)?;
        }
        let device = mount::mount(&config.mountpoint).map_err(to_io)?;
        // SAFETY: getuid never fails and has no preconditions.
        let uid = unsafe { libc::getuid() };
        let dispatcher = FuseDispatcher::for_owner(self.host.clone(), uid);
        let mountpoint = config.mountpoint.clone();
        std::thread::Builder::new()
            .name("humata2-fuse-serve".to_string())
            .spawn(move || {
                if let Err(err) = mount::serve(device, dispatcher) {
                    eprintln!(
                        "humata2_fusefs stopped serving {}: {err}. The mount is gone; humatad2 will remount it.",
                        mountpoint.display()
                    );
                }
            })?;
        // This process serves the mount, but the supervisor must never be
        // handed its own PID.
        Ok(None)
    }

    fn unmount(&self, mountpoint: &Path, force: bool) -> io::Result<()> {
        mount::unmount(mountpoint, force).map_err(to_io)
    }

    fn is_stale_mount(&self, mountpoint: &Path) -> io::Result<bool> {
        match fs::metadata(mountpoint) {
            Ok(_) => Ok(false),
            Err(err) => {
                let raw = err.raw_os_error();
                if raw == Some(libc::ENOTCONN) || raw == Some(libc::ESTALE) {
                    Ok(true)
                } else if err.kind() == io::ErrorKind::NotFound {
                    Ok(false)
                } else {
                    Err(err)
                }
            }
        }
    }

    fn cleanup_stale_mount(&self, mountpoint: &Path) -> io::Result<()> {
        mount::unmount(mountpoint, true).map_err(to_io)
    }

    fn is_server_alive(&self, pid: u32) -> bool {
        // The mount is served by this process, which `mount` never names, so
        // the supervisor has no PID to ask about. Answer for any other PID
        // the way the system does.
        if pid == 0 {
            return true;
        }
        // SAFETY: `kill(pid, 0)` checks process existence without a signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        rc == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
