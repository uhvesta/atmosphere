//! macOS FSKit mount backend: starts [`FsKitBridgeServer`] over a private Unix
//! socket so the bundled FSKit `.appex` extension can serve [`HumataVolumeHost`],
//! then mounts via `/sbin/mount -F -o nobrowse -t humatafs`.

use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use fusefs_fskit::{FsKitAdapter, FsKitBridgeHandle, FsKitBridgeServer, MountSpec};
use fusefs_host::{CapabilityRouter, MountBackend, MountConfig};

use crate::layout::HumataVolumeHost;

/// Filesystem type (`FSShortName` in `HumataFS.appex/Contents/Info.plist`).
pub const HUMATA_FSKIT_FS_TYPE: &str = "humatafs";

/// Default URL authority for Humata's `FSGenericURLResource` (`humatafs://humata2`).
pub const HUMATA_FSKIT_AUTHORITY: &str = "humata2";

/// Darwin `sockaddr_un.sun_path` holds 104 bytes including the trailing NUL.
const MAX_DARWIN_UNIX_SOCKET_BYTES: usize = 100;

/// Picks the default Unix socket path for the FSKit `.appex` bridge.
///
/// Order:
/// 1. `HUMATA_FSKIT_BRIDGE_SOCKET` environment variable (explicit override).
/// 2. App Group container (`$HOME/Library/Group Containers/<group>/fskit.sock`)
///    when `HUMATA_FSKIT_APP_GROUP` (or compile-time `HUMATA2_APP_GROUP`) is set
///    and the resulting path fits Darwin's `sun_path` limit.
/// 3. Sibling `fskit.sock` next to `control_socket`.
pub fn default_bridge_socket(control_socket: &Path) -> PathBuf {
    if let Ok(override_path) = std::env::var("HUMATA_FSKIT_BRIDGE_SOCKET")
        && !override_path.is_empty()
    {
        return PathBuf::from(override_path);
    }

    let app_group = std::env::var("HUMATA_FSKIT_APP_GROUP")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            option_env!("HUMATA2_APP_GROUP")
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        });

    if let (Some(group), Some(home)) = (app_group, std::env::var_os("HOME")) {
        let candidate = PathBuf::from(home)
            .join("Library/Group Containers")
            .join(group)
            .join("fskit.sock");
        if candidate.as_os_str().len() <= MAX_DARWIN_UNIX_SOCKET_BYTES {
            return candidate;
        }
    }

    let sibling = control_socket
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.join("fskit.sock"))
        .unwrap_or_else(|| PathBuf::from("fskit.sock"));
    sibling
}

/// Formats a user-facing error message when `/sbin/mount -F -t humatafs` fails.
///
/// `humatad2` surfaces this text directly in Settings' "can't set up" card, so
/// it tells the owner how to turn the FSKit extension on in System Settings
/// while preserving the raw mount output for diagnostics.
pub fn classify_mount_failure(status_desc: &str, stderr: &str) -> String {
    let detail = stderr.trim();
    let suffix = if detail.is_empty() {
        status_desc.to_string()
    } else {
        detail.replace('\n', " ")
    };
    format!(
        "the Humata File System extension is not enabled or unavailable — turn it on in \
         System Settings > General > Login Items & Extensions > File System Extensions \
         ({suffix})"
    )
}

/// Abstraction over `/sbin/mount` and `/sbin/umount` so `MacFsKitBackend` can
/// be tested headlessly without requiring a kernel FSKit mount.
pub trait FsKitMountRunner: Send + Sync {
    fn run_mount(&self, spec: &MountSpec) -> io::Result<()>;
    fn run_unmount(&self, mountpoint: &Path, force: bool) -> io::Result<()>;
}

/// Real macOS mount runner invoking `/sbin/mount` and `/sbin/umount`.
#[derive(Debug, Default)]
pub struct SystemFsKitMountRunner;

impl FsKitMountRunner for SystemFsKitMountRunner {
    fn run_mount(&self, spec: &MountSpec) -> io::Result<()> {
        let output = Command::new(&spec.program).args(spec.args()).output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(io::Error::other(classify_mount_failure(
                &output.status.to_string(),
                &stderr,
            )));
        }
        Ok(())
    }

    fn run_unmount(&self, mountpoint: &Path, force: bool) -> io::Result<()> {
        let mut cmd = Command::new("/sbin/umount");
        if force {
            cmd.arg("-f");
        }
        cmd.arg(mountpoint);
        let output = cmd.output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let detail = stderr.trim();
            return Err(io::Error::other(if detail.is_empty() {
                format!("umount failed with status {}", output.status)
            } else {
                format!("umount failed ({}): {detail}", output.status)
            }));
        }
        Ok(())
    }
}

/// Mounts the Humata volume on macOS via FSKit, serving [`HumataVolumeHost`]
/// over [`FsKitBridgeServer`] to the bundled `HumataFS.appex` extension.
pub struct MacFsKitBackend {
    host: Arc<HumataVolumeHost>,
    router: Arc<CapabilityRouter>,
    bridge_socket: PathBuf,
    runner: Arc<dyn FsKitMountRunner>,
    bridge: Mutex<Option<FsKitBridgeHandle>>,
}

impl MacFsKitBackend {
    /// Creates a macOS FSKit backend using the system `/sbin/mount` and `/sbin/umount`.
    pub fn new(
        host: Arc<HumataVolumeHost>,
        router: Arc<CapabilityRouter>,
        bridge_socket: PathBuf,
    ) -> Self {
        Self::with_runner(host, router, bridge_socket, Arc::new(SystemFsKitMountRunner))
    }

    /// Creates a macOS FSKit backend with a custom mount runner (for headless tests).
    pub fn with_runner(
        host: Arc<HumataVolumeHost>,
        router: Arc<CapabilityRouter>,
        bridge_socket: PathBuf,
        runner: Arc<dyn FsKitMountRunner>,
    ) -> Self {
        Self {
            host,
            router,
            bridge_socket,
            runner,
            bridge: Mutex::new(None),
        }
    }

    /// Returns the Unix socket path where [`FsKitBridgeServer`] listens.
    pub fn bridge_socket(&self) -> &Path {
        &self.bridge_socket
    }

    fn ensure_bridge_running(&self) -> io::Result<()> {
        let mut guard = self.bridge.lock().expect("fskit bridge lock poisoned");
        if guard.is_some() {
            return Ok(());
        }
        let adapter = Arc::new(
            FsKitAdapter::new(
                Arc::clone(&self.host),
                HUMATA_FSKIT_FS_TYPE,
                HUMATA_FSKIT_AUTHORITY,
            )
            .with_capability_router(Arc::clone(&self.router)),
        );
        let handle = FsKitBridgeServer::bind(&self.bridge_socket, adapter)?.spawn();
        *guard = Some(handle);
        Ok(())
    }

    fn stop_bridge(&self) {
        let handle = self.bridge.lock().expect("fskit bridge lock poisoned").take();
        if let Some(handle) = handle {
            handle.shutdown();
        }
    }

    fn build_mount_spec(&self, config: &MountConfig) -> io::Result<MountSpec> {
        let mut spec = if config.resource_url.starts_with(&format!("{HUMATA_FSKIT_FS_TYPE}://")) {
            MountSpec::new(HUMATA_FSKIT_FS_TYPE, &config.resource_url, &config.mountpoint)?
        } else {
            MountSpec::for_humatafs_with_socket(
                HUMATA_FSKIT_AUTHORITY,
                &self.bridge_socket,
                &config.mountpoint,
            )?
        };
        spec = spec.with_nobrowse(config.nobrowse);
        Ok(spec)
    }
}

impl MountBackend for MacFsKitBackend {
    fn mount(&self, config: &MountConfig) -> io::Result<Option<u32>> {
        if self.is_stale_mount(&config.mountpoint)? {
            self.cleanup_stale_mount(&config.mountpoint)?;
        }
        if let Err(err) = fs::metadata(&config.mountpoint)
            && err.kind() == io::ErrorKind::NotFound
        {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&config.mountpoint)?;
        }

        self.ensure_bridge_running()?;
        let spec = match self.build_mount_spec(config) {
            Ok(spec) => spec,
            Err(err) => {
                self.stop_bridge();
                return Err(err);
            }
        };

        if let Err(err) = self.runner.run_mount(&spec) {
            self.stop_bridge();
            return Err(err);
        }

        // The FSKit extension serves the mount; never report this process's own PID.
        Ok(None)
    }

    fn unmount(&self, mountpoint: &Path, force: bool) -> io::Result<()> {
        let result = self.runner.run_unmount(mountpoint, force);
        self.stop_bridge();
        result
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
        self.runner.run_unmount(mountpoint, true)
    }

    fn is_server_alive(&self, pid: u32) -> bool {
        if pid == 0 {
            return true;
        }
        // SAFETY: `kill(pid, 0)` checks process existence without delivering a signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        rc == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use fusefs_fskit::{
        FsItemKind, FsKitBridgeClient, FsKitEnvelope, FsKitRequest, FsKitResponse, ProbeResult,
    };
    use fusefs_host::{MountState, MountSupervisor};
    use fusefs_vfs::ROOT_NODE_ID;

    use super::*;
    use crate::layout::{HumataTopology, MachineProjection, SessionProjection};

    struct RecordingRunner {
        fail_mount: AtomicBool,
        mounts: AtomicUsize,
        unmounts: AtomicUsize,
        last_spec: Mutex<Option<MountSpec>>,
    }

    impl RecordingRunner {
        fn new() -> Self {
            Self {
                fail_mount: AtomicBool::new(false),
                mounts: AtomicUsize::new(0),
                unmounts: AtomicUsize::new(0),
                last_spec: Mutex::new(None),
            }
        }
    }

    impl FsKitMountRunner for RecordingRunner {
        fn run_mount(&self, spec: &MountSpec) -> io::Result<()> {
            *self.last_spec.lock().unwrap() = Some(spec.clone());
            if self.fail_mount.load(Ordering::SeqCst) {
                return Err(io::Error::other(classify_mount_failure(
                    "exit status: 71",
                    "mount: exec /Library/Filesystems/humatafs.fs: No such file or directory",
                )));
            }
            self.mounts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn run_unmount(&self, _mountpoint: &Path, _force: bool) -> io::Result<()> {
            self.unmounts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn sample_host() -> Arc<HumataVolumeHost> {
        let sess = SessionProjection::new("sess-1", "mac-local", "/Users/dev/humata", 4242)
            .with_inline_file("README.md", b"hello mac\n".to_vec());
        let topo = HumataTopology {
            local_machine_id: "mac-local".into(),
            local_machine_root: PathBuf::from("/Users/dev/humata"),
            machines: vec![MachineProjection {
                machine_id: "mac-local".into(),
                display_name: "Mac".into(),
                online: true,
                machine_root: PathBuf::from("/Users/dev/humata"),
            }],
            sessions: vec![sess],
            desks: Vec::new(),
            local_context_files: Vec::new(),
            control_status_json: None,
        };
        Arc::new(HumataVolumeHost::new(std::process::id(), topo, None, None, None).unwrap())
    }

    #[test]
    fn mount_starts_bridge_serves_humata_volume_and_unmount_cleans_up() {
        let tmp = tempfile::tempdir().unwrap();
        let mountpoint = tmp.path().join("mnt");
        let bridge_sock = tmp.path().join("fskit.sock");

        let host = sample_host();
        let router = Arc::new(CapabilityRouter::new());
        let runner = Arc::new(RecordingRunner::new());
        let backend = Arc::new(MacFsKitBackend::with_runner(
            host,
            router,
            bridge_sock.clone(),
            runner.clone(),
        ));

        let config = MountConfig::new(&mountpoint, "humatafs://humata2");
        let sup = MountSupervisor::new(config, backend.clone());

        let status = sup.mount().expect("mount succeeds");
        assert_eq!(status.state, MountState::Healthy);
        assert_eq!(status.server_pid, None);
        assert_eq!(runner.mounts.load(Ordering::SeqCst), 1);
        assert!(bridge_sock.exists(), "bridge socket must exist while mounted");

        let spec = runner.last_spec.lock().unwrap().clone().unwrap();
        assert_eq!(spec.fs_type, "humatafs");
        assert!(spec.nobrowse);
        assert_eq!(spec.mountpoint, mountpoint);

        // Connect as the FSKit .appex would and exercise read + create + write + read.
        let mut client = FsKitBridgeClient::connect(&bridge_sock).unwrap();
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::ProbeResource {
                resource_url: "humatafs://humata2".into(),
            }))
            .unwrap()
        {
            FsKitResponse::Probe {
                result: ProbeResult::Usable { name, .. },
            } => assert_eq!(name, "humata2-humata2"),
            other => panic!("expected Usable probe, got {other:?}"),
        }

        // Navigate /s/sess-1/repo/README.md
        let mut cur = ROOT_NODE_ID;
        for seg in ["s", "sess-1", "repo"] {
            cur = match client
                .call(&FsKitEnvelope::new(FsKitRequest::LookupItem {
                    parent: cur,
                    name: seg.into(),
                }))
                .unwrap()
            {
                FsKitResponse::ItemAttributes { attr } => attr.ino,
                other => panic!("lookup {seg} failed: {other:?}"),
            };
        }
        let repo_ino = cur;

        let readme_ino = match client
            .call(&FsKitEnvelope::new(FsKitRequest::LookupItem {
                parent: repo_ino,
                name: "README.md".into(),
            }))
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => attr.ino,
            other => panic!("lookup README.md failed: {other:?}"),
        };
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::Read {
                item: readme_ino,
                handle: None,
                offset: 0,
                length: 64,
            }))
            .unwrap()
        {
            FsKitResponse::ReadData { data } => assert_eq!(data, b"hello mac\n"),
            other => panic!("read README.md failed: {other:?}"),
        }

        // Agent creates and writes a new file inside the session repo through the FSKit bridge.
        let new_ino = match client
            .call(&FsKitEnvelope::new(FsKitRequest::CreateItem {
                parent: repo_ino,
                name: "agent_out.txt".into(),
                kind: FsItemKind::Regular,
                mode: 0o644,
                symlink_target: None,
            }))
            .unwrap()
        {
            FsKitResponse::ItemAttributes { attr } => attr.ino,
            other => panic!("create agent_out.txt failed: {other:?}"),
        };
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::Write {
                item: new_ino,
                handle: None,
                offset: 0,
                data: b"written via fskit".to_vec(),
                expected_generation: None,
            }))
            .unwrap()
        {
            FsKitResponse::Written { bytes_written } => assert_eq!(bytes_written, 17),
            other => panic!("write agent_out.txt failed: {other:?}"),
        }
        match client
            .call(&FsKitEnvelope::new(FsKitRequest::Read {
                item: new_ino,
                handle: None,
                offset: 0,
                length: 64,
            }))
            .unwrap()
        {
            FsKitResponse::ReadData { data } => assert_eq!(data, b"written via fskit"),
            other => panic!("read agent_out.txt failed: {other:?}"),
        }

        // Unmount shuts down the bridge server and removes the socket.
        let unmounted = sup.unmount(false).expect("unmount succeeds");
        assert_eq!(unmounted.state, MountState::Unmounted);
        assert_eq!(runner.unmounts.load(Ordering::SeqCst), 1);
        assert!(!bridge_sock.exists(), "bridge socket must be removed on unmount");
    }

    #[test]
    fn failed_mount_stops_bridge_and_reports_system_settings_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let mountpoint = tmp.path().join("mnt");
        let bridge_sock = tmp.path().join("fskit.sock");

        let host = sample_host();
        let router = Arc::new(CapabilityRouter::new());
        let runner = Arc::new(RecordingRunner::new());
        runner.fail_mount.store(true, Ordering::SeqCst);

        let backend = Arc::new(MacFsKitBackend::with_runner(
            host,
            router,
            bridge_sock.clone(),
            runner,
        ));
        let sup = MountSupervisor::new(
            MountConfig::new(&mountpoint, "humatafs://humata2"),
            backend,
        );

        let err = sup.mount().unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("System Settings") && msg.contains("File System Extensions"),
            "error must explain how to enable the extension: {msg}"
        );
        assert!(
            !bridge_sock.exists(),
            "bridge socket must be cleaned up when mount fails"
        );
        assert_eq!(sup.status().state, MountState::Failed);
    }

    #[test]
    fn remount_recovers_bridge_and_increments_generation() {
        let tmp = tempfile::tempdir().unwrap();
        let mountpoint = tmp.path().join("mnt");
        let bridge_sock = tmp.path().join("fskit.sock");

        let host = sample_host();
        let router = Arc::new(CapabilityRouter::new());
        let runner = Arc::new(RecordingRunner::new());
        let backend = Arc::new(MacFsKitBackend::with_runner(
            host,
            router,
            bridge_sock.clone(),
            runner.clone(),
        ));
        let sup = MountSupervisor::new(
            MountConfig::new(&mountpoint, "humatafs://humata2"),
            backend,
        );

        let initial = sup.mount().expect("initial mount");
        assert_eq!(initial.generation, 1);

        let recovered = sup.remount().expect("remount");
        assert_eq!(recovered.state, MountState::Healthy);
        assert_eq!(recovered.generation, 2);
        assert_eq!(recovered.restart_count, 1);
        assert!(bridge_sock.exists());

        // Bridge still answers pings after remount.
        let mut client = FsKitBridgeClient::connect(&bridge_sock).unwrap();
        assert!(matches!(
            client.call(&FsKitEnvelope::new(FsKitRequest::Ping)).unwrap(),
            FsKitResponse::Pong { .. }
        ));

        sup.unmount(false).expect("unmount");
    }
}
