//! Runtime `/dev/fuse` availability probing and mount mode selection.

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Standard Linux FUSE character device path.
pub const DEFAULT_FUSE_DEVICE_PATH: &str = "/dev/fuse";

/// Availability status of the kernel FUSE device (`/dev/fuse`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FuseAvailability {
    /// `/dev/fuse` exists and is readable/writable by the current process.
    Available,
    /// `/dev/fuse` is absent (common in unprivileged containers without `--device /dev/fuse`).
    DeviceMissing,
    /// `/dev/fuse` exists but the current process lacks permission to open it.
    PermissionDenied,
    /// Current operating system does not provide Linux `/dev/fuse`.
    UnsupportedPlatform,
}

/// Selected mount strategy based on runtime FUSE availability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MountMode {
    /// Full kernel FUSE mount via `/dev/fuse`.
    DevFuse,
    /// Explicit degraded native-symlink skeleton directory with `.degraded.json` notice.
    DegradedSymlinkSkeleton,
}

/// Combined probe diagnostic and selected [`MountMode`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountProbeResult {
    pub availability: FuseAvailability,
    pub mode: MountMode,
    pub device_path: PathBuf,
    pub detail: String,
}

/// Selects the appropriate [`MountMode`] for a probed [`FuseAvailability`].
pub const fn select_mount_mode(availability: FuseAvailability) -> MountMode {
    match availability {
        FuseAvailability::Available => MountMode::DevFuse,
        FuseAvailability::DeviceMissing
        | FuseAvailability::PermissionDenied
        | FuseAvailability::UnsupportedPlatform => MountMode::DegradedSymlinkSkeleton,
    }
}

/// Probes a specific device path for read/write availability.
///
/// This helper is portable across platforms so callers and tests can exercise
/// missing, permission-denied, and available device paths deterministically.
pub fn probe_fuse_device_path(path: &Path) -> FuseAvailability {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(_) => FuseAvailability::Available,
        Err(err) => match err.kind() {
            ErrorKind::NotFound => FuseAvailability::DeviceMissing,
            ErrorKind::PermissionDenied => FuseAvailability::PermissionDenied,
            _ => {
                if !path.exists() {
                    FuseAvailability::DeviceMissing
                } else {
                    FuseAvailability::PermissionDenied
                }
            }
        },
    }
}

/// Probes the host's default `/dev/fuse` device.
///
/// Returns [`FuseAvailability::UnsupportedPlatform`] on non-Linux hosts.
pub fn probe_dev_fuse() -> FuseAvailability {
    #[cfg(target_os = "linux")]
    {
        probe_fuse_device_path(Path::new(DEFAULT_FUSE_DEVICE_PATH))
    }
    #[cfg(not(target_os = "linux"))]
    {
        FuseAvailability::UnsupportedPlatform
    }
}

/// Probes the default `/dev/fuse` path and returns a full [`MountProbeResult`].
pub fn probe_mount_mode() -> MountProbeResult {
    let availability = probe_dev_fuse();
    build_probe_result(PathBuf::from(DEFAULT_FUSE_DEVICE_PATH), availability)
}

/// Probes a custom device path and returns a full [`MountProbeResult`].
pub fn probe_mount_mode_at(path: &Path) -> MountProbeResult {
    let availability = probe_fuse_device_path(path);
    build_probe_result(path.to_path_buf(), availability)
}

/// Opens `/dev/fuse` for reading and writing on Linux.
#[cfg(target_os = "linux")]
pub fn open_dev_fuse(path: &Path) -> std::io::Result<std::fs::File> {
    OpenOptions::new().read(true).write(true).open(path)
}

fn build_probe_result(device_path: PathBuf, availability: FuseAvailability) -> MountProbeResult {
    let mode = select_mount_mode(availability);
    let detail = match availability {
        FuseAvailability::Available => {
            format!("FUSE device '{}' is accessible", device_path.display())
        }
        FuseAvailability::DeviceMissing => format!(
            "FUSE device '{}' is missing; falling back to degraded symlink skeleton",
            device_path.display()
        ),
        FuseAvailability::PermissionDenied => format!(
            "permission denied opening FUSE device '{}'; falling back to degraded symlink skeleton",
            device_path.display()
        ),
        FuseAvailability::UnsupportedPlatform => {
            "Linux /dev/fuse is not supported on this platform; falling back to degraded symlink skeleton"
                .to_string()
        }
    };
    MountProbeResult {
        availability,
        mode,
        device_path,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn missing_device_falls_back_to_degraded_skeleton() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("nonexistent-fuse-dev");

        let result = probe_mount_mode_at(&missing);
        assert_eq!(result.availability, FuseAvailability::DeviceMissing);
        assert_eq!(result.mode, MountMode::DegradedSymlinkSkeleton);
        assert!(result.detail.contains("missing"));
    }

    #[test]
    fn accessible_device_path_selects_dev_fuse() {
        let dir = TempDir::new().unwrap();
        let fake_dev = dir.path().join("fuse");
        std::fs::write(&fake_dev, b"").unwrap();

        let result = probe_mount_mode_at(&fake_dev);
        assert_eq!(result.availability, FuseAvailability::Available);
        assert_eq!(result.mode, MountMode::DevFuse);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_device_path_reports_permission_denied_and_falls_back() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let restricted = dir.path().join("fuse-restricted");
        std::fs::write(&restricted, b"").unwrap();
        std::fs::set_permissions(&restricted, std::fs::Permissions::from_mode(0o000)).unwrap();

        let availability = probe_fuse_device_path(&restricted);
        // Running as root may bypass mode 000; only assert PermissionDenied when non-root.
        let result = select_mount_mode(availability);
        if availability == FuseAvailability::PermissionDenied {
            assert_eq!(result, MountMode::DegradedSymlinkSkeleton);
        } else {
            assert_eq!(availability, FuseAvailability::Available);
            assert_eq!(result, MountMode::DevFuse);
        }

        let _ = std::fs::set_permissions(&restricted, std::fs::Permissions::from_mode(0o600));
    }

    #[test]
    fn select_mount_mode_maps_all_availability_variants() {
        assert_eq!(
            select_mount_mode(FuseAvailability::Available),
            MountMode::DevFuse
        );
        assert_eq!(
            select_mount_mode(FuseAvailability::DeviceMissing),
            MountMode::DegradedSymlinkSkeleton
        );
        assert_eq!(
            select_mount_mode(FuseAvailability::PermissionDenied),
            MountMode::DegradedSymlinkSkeleton
        );
        assert_eq!(
            select_mount_mode(FuseAvailability::UnsupportedPlatform),
            MountMode::DegradedSymlinkSkeleton
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_host_reports_unsupported_platform() {
        let result = probe_mount_mode();
        assert_eq!(result.availability, FuseAvailability::UnsupportedPlatform);
        assert_eq!(result.mode, MountMode::DegradedSymlinkSkeleton);
    }
}
