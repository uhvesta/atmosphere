//! Mount supervisor state machine with stale-mount cleanup and server restart recovery.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

/// Lifecycle state of a supervised FUSEFS mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountState {
    Unmounted,
    Mounting,
    Healthy,
    Restarting,
    Failed,
}

/// Configuration for a supervised mountpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountConfig {
    pub mountpoint: PathBuf,
    pub nobrowse: bool,
    pub resource_url: String,
    pub max_restart_attempts: u32,
}

impl MountConfig {
    /// Creates a default mount configuration with `nobrowse = true`.
    pub fn new(mountpoint: impl Into<PathBuf>, resource_url: impl Into<String>) -> Self {
        Self {
            mountpoint: mountpoint.into(),
            nobrowse: true,
            resource_url: resource_url.into(),
            max_restart_attempts: 3,
        }
    }
}

/// Observable status snapshot reported by [`MountSupervisor`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountStatus {
    pub mountpoint: PathBuf,
    pub nobrowse: bool,
    pub resource_url: String,
    pub state: MountState,
    pub server_pid: Option<u32>,
    pub restart_count: u32,
    pub generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Platform/backend hook for mounting, unmounting, and stale-mount recovery.
pub trait MountBackend: Send + Sync {
    /// Mounts `config` and returns the PID of the serving process.
    fn mount(&self, config: &MountConfig) -> io::Result<u32>;

    /// Unmounts `mountpoint` (using force when `force` is `true`).
    fn unmount(&self, mountpoint: &Path, force: bool) -> io::Result<()>;

    /// Returns `true` if `mountpoint` is in a stale/broken mount state.
    fn is_stale_mount(&self, mountpoint: &Path) -> io::Result<bool>;

    /// Cleans up a stale mount at `mountpoint` (e.g., forced unmount).
    fn cleanup_stale_mount(&self, mountpoint: &Path) -> io::Result<()>;

    /// Returns `true` if the server process `pid` is still alive.
    fn is_server_alive(&self, pid: u32) -> bool;
}

/// Default POSIX mount backend invoking `/sbin/mount` and `/sbin/umount`.
#[derive(Debug, Clone)]
pub struct PosixMountBackend {
    pub fs_type: String,
}

impl PosixMountBackend {
    pub fn new(fs_type: impl Into<String>) -> Self {
        Self {
            fs_type: fs_type.into(),
        }
    }
}

impl MountBackend for PosixMountBackend {
    fn mount(&self, config: &MountConfig) -> io::Result<u32> {
        let mut cmd = Command::new("/sbin/mount");
        #[cfg(target_os = "macos")]
        cmd.arg("-F");
        if config.nobrowse {
            cmd.args(["-o", "nobrowse"]);
        }
        cmd.args(["-t", &self.fs_type]);
        cmd.arg(&config.resource_url);
        cmd.arg(&config.mountpoint);

        let status = cmd.status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "mount failed with status {status}"
            )));
        }
        Ok(std::process::id())
    }

    fn unmount(&self, mountpoint: &Path, force: bool) -> io::Result<()> {
        let mut cmd = Command::new("/sbin/umount");
        if force {
            cmd.arg("-f");
        }
        cmd.arg(mountpoint);
        let status = cmd.status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "umount failed with status {status}"
            )));
        }
        Ok(())
    }

    fn is_stale_mount(&self, mountpoint: &Path) -> io::Result<bool> {
        match std::fs::metadata(mountpoint) {
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
        self.unmount(mountpoint, true)
    }

    fn is_server_alive(&self, pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        // SAFETY: `kill(pid, 0)` checks process existence without delivering a signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        rc == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

struct SupervisorInner {
    config: MountConfig,
    state: MountState,
    server_pid: Option<u32>,
    restart_count: u32,
    consecutive_failures: u32,
    generation: u64,
    last_error: Option<String>,
}

impl SupervisorInner {
    fn snapshot(&self) -> MountStatus {
        MountStatus {
            mountpoint: self.config.mountpoint.clone(),
            nobrowse: self.config.nobrowse,
            resource_url: self.config.resource_url.clone(),
            state: self.state,
            server_pid: self.server_pid,
            restart_count: self.restart_count,
            generation: self.generation,
            last_error: self.last_error.clone(),
        }
    }
}

/// Thread-safe mount lifecycle and restart/reconnect supervisor.
pub struct MountSupervisor {
    inner: RwLock<SupervisorInner>,
    backend: Arc<dyn MountBackend>,
}

impl MountSupervisor {
    /// Creates a new supervisor starting in [`MountState::Unmounted`].
    pub fn new(config: MountConfig, backend: Arc<dyn MountBackend>) -> Self {
        Self {
            inner: RwLock::new(SupervisorInner {
                config,
                state: MountState::Unmounted,
                server_pid: None,
                restart_count: 0,
                consecutive_failures: 0,
                generation: 0,
                last_error: None,
            }),
            backend,
        }
    }

    /// Returns a snapshot of the current mount status.
    pub fn status(&self) -> MountStatus {
        self.inner
            .read()
            .expect("supervisor lock poisoned")
            .snapshot()
    }

    /// Updates optional mount configuration overrides before mounting.
    pub fn update_config(
        &self,
        mountpoint: Option<PathBuf>,
        nobrowse: Option<bool>,
        resource_url: Option<String>,
    ) {
        let mut inner = self.inner.write().expect("supervisor lock poisoned");
        if let Some(mp) = mountpoint {
            inner.config.mountpoint = mp;
        }
        if let Some(nb) = nobrowse {
            inner.config.nobrowse = nb;
        }
        if let Some(url) = resource_url {
            inner.config.resource_url = url;
        }
    }

    /// Mounts the filesystem, cleaning up any pre-existing stale mount first.
    pub fn mount(&self) -> io::Result<MountStatus> {
        let config = {
            let mut inner = self.inner.write().expect("supervisor lock poisoned");
            if inner.state == MountState::Healthy
                && inner
                    .server_pid
                    .is_some_and(|pid| self.backend.is_server_alive(pid))
            {
                return Ok(inner.snapshot());
            }
            inner.state = MountState::Mounting;
            inner.last_error = None;
            inner.config.clone()
        };

        if self.backend.is_stale_mount(&config.mountpoint)? {
            self.backend.cleanup_stale_mount(&config.mountpoint)?;
        }

        match self.backend.mount(&config) {
            Ok(pid) => {
                let mut inner = self.inner.write().expect("supervisor lock poisoned");
                inner.state = MountState::Healthy;
                inner.server_pid = Some(pid);
                inner.consecutive_failures = 0;
                inner.generation = inner.generation.saturating_add(1);
                inner.last_error = None;
                Ok(inner.snapshot())
            }
            Err(err) => {
                let mut inner = self.inner.write().expect("supervisor lock poisoned");
                inner.state = MountState::Failed;
                inner.server_pid = None;
                inner.last_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    /// Unmounts the filesystem and transitions to [`MountState::Unmounted`].
    pub fn unmount(&self, force: bool) -> io::Result<MountStatus> {
        let mountpoint = {
            let inner = self.inner.read().expect("supervisor lock poisoned");
            if inner.state == MountState::Unmounted {
                return Ok(inner.snapshot());
            }
            inner.config.mountpoint.clone()
        };

        self.backend.unmount(&mountpoint, force)?;

        let mut inner = self.inner.write().expect("supervisor lock poisoned");
        inner.state = MountState::Unmounted;
        inner.server_pid = None;
        inner.last_error = None;
        Ok(inner.snapshot())
    }

    /// Performs a clean remount (stale-mount cleanup + unmount + mount).
    pub fn remount(&self) -> io::Result<MountStatus> {
        let config = {
            let mut inner = self.inner.write().expect("supervisor lock poisoned");
            inner.state = MountState::Restarting;
            inner.config.clone()
        };

        if self.backend.is_stale_mount(&config.mountpoint)? {
            self.backend.cleanup_stale_mount(&config.mountpoint)?;
        } else {
            let _ = self.backend.unmount(&config.mountpoint, true);
        }

        match self.backend.mount(&config) {
            Ok(pid) => {
                let mut inner = self.inner.write().expect("supervisor lock poisoned");
                inner.state = MountState::Healthy;
                inner.server_pid = Some(pid);
                inner.restart_count = inner.restart_count.saturating_add(1);
                inner.consecutive_failures = 0;
                inner.generation = inner.generation.saturating_add(1);
                inner.last_error = None;
                Ok(inner.snapshot())
            }
            Err(err) => {
                let mut inner = self.inner.write().expect("supervisor lock poisoned");
                inner.consecutive_failures = inner.consecutive_failures.saturating_add(1);
                inner.state = MountState::Failed;
                inner.server_pid = None;
                inner.last_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    /// Checks whether the supervised server is alive and the mountpoint is clean;
    /// if the server died or the mount became stale, recovers cleanly via restart.
    pub fn check_health_and_recover(&self) -> io::Result<MountStatus> {
        let (state, pid, mountpoint, max_attempts, failures) = {
            let inner = self.inner.read().expect("supervisor lock poisoned");
            (
                inner.state,
                inner.server_pid,
                inner.config.mountpoint.clone(),
                inner.config.max_restart_attempts,
                inner.consecutive_failures,
            )
        };

        if state == MountState::Unmounted {
            return Ok(self.status());
        }

        let alive = pid.is_some_and(|p| self.backend.is_server_alive(p));
        let stale = self.backend.is_stale_mount(&mountpoint)?;

        if state == MountState::Healthy && alive && !stale {
            return Ok(self.status());
        }

        if failures >= max_attempts {
            let mut inner = self.inner.write().expect("supervisor lock poisoned");
            inner.state = MountState::Failed;
            inner.server_pid = None;
            let msg = format!("exceeded max restart attempts ({max_attempts})");
            inner.last_error = Some(msg.clone());
            return Err(io::Error::other(msg));
        }

        self.remount()
    }

    /// Handles an explicit server kill/crash notification and attempts recovery.
    pub fn notify_server_exit(&self, reason: impl Into<String>) -> io::Result<MountStatus> {
        {
            let mut inner = self.inner.write().expect("supervisor lock poisoned");
            inner.state = MountState::Restarting;
            inner.server_pid = None;
            inner.last_error = Some(reason.into());
        }
        self.check_health_and_recover()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    /// Deterministic in-memory backend for testing mount/kill/restart recovery.
    pub struct FakeMountBackend {
        next_pid: AtomicU32,
        alive_pid: Mutex<Option<u32>>,
        stale: AtomicBool,
        fail_next_mounts: AtomicU32,
        pub stale_cleanups: AtomicU32,
        pub unmount_calls: AtomicU32,
    }

    impl FakeMountBackend {
        pub fn new() -> Self {
            Self {
                next_pid: AtomicU32::new(1000),
                alive_pid: Mutex::new(None),
                stale: AtomicBool::new(false),
                fail_next_mounts: AtomicU32::new(0),
                stale_cleanups: AtomicU32::new(0),
                unmount_calls: AtomicU32::new(0),
            }
        }

        pub fn simulate_server_kill(&self, leave_stale_mount: bool) {
            *self.alive_pid.lock().unwrap() = None;
            self.stale.store(leave_stale_mount, Ordering::SeqCst);
        }

        pub fn set_fail_next_mounts(&self, count: u32) {
            self.fail_next_mounts.store(count, Ordering::SeqCst);
        }
    }

    impl MountBackend for FakeMountBackend {
        fn mount(&self, _config: &MountConfig) -> io::Result<u32> {
            let remaining = self.fail_next_mounts.load(Ordering::SeqCst);
            if remaining > 0 {
                self.fail_next_mounts.fetch_sub(1, Ordering::SeqCst);
                return Err(io::Error::other("simulated mount failure"));
            }
            if self.stale.load(Ordering::SeqCst) {
                return Err(io::Error::other("cannot mount over stale mountpoint"));
            }
            let pid = self.next_pid.fetch_add(1, Ordering::SeqCst);
            *self.alive_pid.lock().unwrap() = Some(pid);
            Ok(pid)
        }

        fn unmount(&self, _mountpoint: &Path, _force: bool) -> io::Result<()> {
            self.unmount_calls.fetch_add(1, Ordering::SeqCst);
            *self.alive_pid.lock().unwrap() = None;
            self.stale.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn is_stale_mount(&self, _mountpoint: &Path) -> io::Result<bool> {
            Ok(self.stale.load(Ordering::SeqCst))
        }

        fn cleanup_stale_mount(&self, _mountpoint: &Path) -> io::Result<()> {
            self.stale_cleanups.fetch_add(1, Ordering::SeqCst);
            self.stale.store(false, Ordering::SeqCst);
            Ok(())
        }

        fn is_server_alive(&self, pid: u32) -> bool {
            *self.alive_pid.lock().unwrap() == Some(pid)
        }
    }

    #[test]
    fn mount_kill_stale_recovery_and_unmount_lifecycle() {
        let backend = Arc::new(FakeMountBackend::new());
        let config = MountConfig::new("/tmp/fusefs-test-mnt", "fusefs://main");
        let sup = MountSupervisor::new(config, backend.clone());

        assert_eq!(sup.status().state, MountState::Unmounted);
        assert!(sup.status().nobrowse);

        let mounted = sup.mount().expect("initial mount");
        assert_eq!(mounted.state, MountState::Healthy);
        assert_eq!(mounted.server_pid, Some(1000));
        assert_eq!(mounted.generation, 1);

        // Simulate abrupt server kill leaving a stale mountpoint behind.
        backend.simulate_server_kill(true);
        let recovered = sup
            .check_health_and_recover()
            .expect("should clean stale mount and restart");
        assert_eq!(recovered.state, MountState::Healthy);
        assert_eq!(recovered.server_pid, Some(1001));
        assert_eq!(recovered.restart_count, 1);
        assert_eq!(recovered.generation, 2);
        assert_eq!(backend.stale_cleanups.load(Ordering::SeqCst), 1);

        // Explicit server exit notification also recovers cleanly.
        backend.simulate_server_kill(true);
        let recovered2 = sup
            .notify_server_exit("SIGKILL")
            .expect("notify_server_exit recovers");
        assert_eq!(recovered2.state, MountState::Healthy);
        assert_eq!(recovered2.server_pid, Some(1002));
        assert_eq!(recovered2.restart_count, 2);
        assert_eq!(backend.stale_cleanups.load(Ordering::SeqCst), 2);

        let unmounted = sup.unmount(false).expect("unmount");
        assert_eq!(unmounted.state, MountState::Unmounted);
        assert_eq!(unmounted.server_pid, None);
    }

    #[test]
    fn stops_restarting_after_max_restart_attempts() {
        let backend = Arc::new(FakeMountBackend::new());
        let mut config = MountConfig::new("/tmp/fusefs-max-restart", "fusefs://main");
        config.max_restart_attempts = 2;
        let sup = MountSupervisor::new(config, backend.clone());

        sup.mount().expect("initial mount");
        backend.simulate_server_kill(false);
        backend.set_fail_next_mounts(5);

        assert!(sup.check_health_and_recover().is_err());
        assert!(sup.check_health_and_recover().is_err());
        let err = sup.check_health_and_recover().unwrap_err();
        assert!(err.to_string().contains("exceeded max restart attempts"));
        assert_eq!(sup.status().state, MountState::Failed);
    }
}
