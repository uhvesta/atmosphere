//! The tools bundle (TS-2): `humatad2`, `humata2_holder` and `humata` for
//! every platform Humata ships, with a `manifest.json` that names each
//! file and its SHA-256. Setup and takeover pick a platform's files by the
//! machine's `uname -sm`; the desktop app installs the host's own from it
//! (TS-0). A file is read only when a machine of its platform needs it, and
//! is checked against the manifest before anything is sent.
//!
//! ```json
//! {"platforms": {"linux-x86_64": {"humatad2": {"path": "linux-x86_64/humatad2", "sha256": "…"}, …}, …}}
//! ```

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::install::{self, Builds, BuildsError, InstalledFile, Payload};
use crate::platform::Platform;

/// The manifest's name inside a bundle.
pub const MANIFEST: &str = "manifest.json";

#[derive(Debug, Deserialize)]
struct Manifest {
    platforms: BTreeMap<String, BTreeMap<String, Entry>>,
}

#[derive(Debug, Clone, Deserialize)]
struct Entry {
    path: PathBuf,
    sha256: String,
}

/// One file the bundle has: where it is and what its SHA-256 must be.
#[derive(Debug, Clone, PartialEq, Eq)]
struct File {
    path: PathBuf,
    /// None for a host bundle made from the files beside the daemon: it is
    /// whatever is there.
    sha256: Option<String>,
}

/// Humata's own builds, read from disk on demand.
#[derive(Debug)]
pub struct Bundle {
    files: BTreeMap<(PlatformKey, InstalledFile), File>,
    /// Payloads already read and checked, per platform, so a second
    /// machine of the same platform reads nothing.
    loaded: Mutex<HashMap<PlatformKey, Vec<Payload>>>,
}

/// [`Platform`] in a form that sorts.
type PlatformKey = &'static str;

impl Bundle {
    /// The bundle in `root`, from its `manifest.json`. Paths in the
    /// manifest are relative to `root`.
    pub fn open(root: &Path) -> Result<Self, BuildsError> {
        let path = root.join(MANIFEST);
        let text = std::fs::read(&path).map_err(|source| BuildsError::ManifestUnreadable {
            path: path.clone(),
            source: Arc::new(source),
        })?;
        let manifest: Manifest = serde_json::from_slice(&text).map_err(|source| {
            BuildsError::ManifestInvalid {
                path: path.clone(),
                source: Arc::new(source),
            }
        })?;
        let mut files = BTreeMap::new();
        for (name, entries) in manifest.platforms {
            // A platform this Humata doesn't know is someone else's.
            let Some(platform) = Platform::from_bundle_name(&name) else {
                continue;
            };
            for file in InstalledFile::ALL {
                if let Some(entry) = entries.get(file.file_name()) {
                    files.insert(
                        (platform.bundle_name(), file),
                        File {
                            path: root.join(&entry.path),
                            sha256: Some(entry.sha256.to_ascii_lowercase()),
                        },
                    );
                }
            }
        }
        Ok(Self {
            files,
            loaded: Mutex::new(HashMap::new()),
        })
    }

    /// A bundle of only `platform`, from the three files named as they are
    /// installed in `dir`: what a development build has beside humatad2
    /// when no bundle was built.
    pub fn beside(dir: &Path, platform: Platform) -> Self {
        let files = InstalledFile::ALL
            .into_iter()
            .map(|file| {
                (
                    (platform.bundle_name(), file),
                    File {
                        path: dir.join(file.file_name()),
                        sha256: None,
                    },
                )
            })
            .collect();
        Self {
            files,
            loaded: Mutex::new(HashMap::new()),
        }
    }

    /// The platforms the bundle has at least one file for.
    pub fn platforms(&self) -> Vec<Platform> {
        Platform::SHIPPED
            .into_iter()
            .filter(|platform| {
                InstalledFile::ALL
                    .iter()
                    .any(|file| self.files.contains_key(&(platform.bundle_name(), *file)))
            })
            .collect()
    }

    fn shipped(&self) -> String {
        let names: Vec<String> = self.platforms().into_iter().map(|platform| platform.to_string()).collect();
        if names.is_empty() {
            "no platforms".to_owned()
        } else {
            names.join(", ")
        }
    }

    fn load(&self, platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        if !InstalledFile::ALL
            .iter()
            .any(|file| self.files.contains_key(&(platform.bundle_name(), *file)))
        {
            return Err(BuildsError::NoBuild {
                platform,
                shipped: self.shipped(),
            });
        }
        let mut payloads = Vec::with_capacity(InstalledFile::ALL.len());
        for file in InstalledFile::ALL {
            let entry = self
                .files
                .get(&(platform.bundle_name(), file))
                .ok_or(BuildsError::FileMissing { platform, file })?;
            let bytes = std::fs::read(&entry.path).map_err(|source| BuildsError::FileUnreadable {
                platform,
                file,
                path: entry.path.clone(),
                source: Arc::new(source),
            })?;
            if let Some(expected) = &entry.sha256 {
                let found = install::hex(&Sha256::digest(&bytes));
                if &found != expected {
                    return Err(BuildsError::Damaged {
                        platform,
                        file,
                        path: entry.path.clone(),
                        expected: expected.clone(),
                        found,
                    });
                }
            }
            payloads.push(Payload::new(file, Arc::from(bytes)));
        }
        Ok(payloads)
    }
}

impl Builds for Bundle {
    fn payloads(&self, platform: Platform) -> Result<Vec<Payload>, BuildsError> {
        let key = platform.bundle_name();
        if let Some(payloads) = self.loaded.lock().unwrap_or_else(PoisonError::into_inner).get(key) {
            return Ok(payloads.clone());
        }
        // Read outside the lock: the files are large.
        let payloads = self.load(platform)?;
        self.loaded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key, payloads.clone());
        Ok(payloads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{Arch, Os};

    const LINUX: Platform = Platform {
        os: Os::Linux,
        arch: Arch::X86_64,
    };

    fn write_bundle(root: &Path, files: &[(Platform, InstalledFile, &[u8])], damage: Option<InstalledFile>) {
        let mut platforms: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
        for (platform, file, bytes) in files {
            let relative = format!("{}/{}", platform.bundle_name(), file.file_name());
            std::fs::create_dir_all(root.join(platform.bundle_name())).unwrap();
            let written: &[u8] = if damage == Some(*file) { b"replaced" } else { bytes };
            std::fs::write(root.join(&relative), written).unwrap();
            platforms.entry(platform.bundle_name().to_owned()).or_default().insert(
                file.file_name().to_owned(),
                serde_json::json!({"path": relative, "sha256": install::hex(&Sha256::digest(bytes))}),
            );
        }
        let manifest = serde_json::json!({ "platforms": platforms });
        std::fs::write(root.join(MANIFEST), manifest.to_string()).unwrap();
    }

    fn all(platform: Platform) -> Vec<(Platform, InstalledFile, &'static [u8])> {
        vec![
            (platform, InstalledFile::Humatad, b"daemon"),
            (platform, InstalledFile::Holder, b"holder"),
            (platform, InstalledFile::Cli, b"cli"),
        ]
    }

    #[test]
    fn a_platform_s_files_come_out_checked() {
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX), None);
        let bundle = Bundle::open(root.path()).unwrap();

        let payloads = bundle.payloads(LINUX).unwrap();

        let files: Vec<(InstalledFile, Vec<u8>)> = payloads
            .iter()
            .map(|payload| (payload.file(), payload.bytes().to_vec()))
            .collect();
        assert_eq!(
            files,
            [
                (InstalledFile::Humatad, b"daemon".to_vec()),
                (InstalledFile::Holder, b"holder".to_vec()),
                (InstalledFile::Cli, b"cli".to_vec()),
            ]
        );
        assert_eq!(bundle.platforms(), [LINUX]);
    }

    #[test]
    fn a_platform_without_files_is_no_build() {
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX), None);
        let bundle = Bundle::open(root.path()).unwrap();
        let mac = Platform {
            os: Os::Darwin,
            arch: Arch::Aarch64,
        };

        assert!(matches!(bundle.payloads(mac), Err(BuildsError::NoBuild { platform, .. }) if platform == mac));
    }

    #[test]
    fn a_damaged_file_is_refused() {
        let root = tempfile::tempdir().unwrap();
        write_bundle(root.path(), &all(LINUX), Some(InstalledFile::Holder));
        let bundle = Bundle::open(root.path()).unwrap();

        assert!(matches!(
            bundle.payloads(LINUX),
            Err(BuildsError::Damaged { file: InstalledFile::Holder, .. })
        ));
    }

    #[test]
    fn a_missing_file_and_a_missing_manifest_are_reported() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            Bundle::open(root.path()),
            Err(BuildsError::ManifestUnreadable { .. })
        ));

        write_bundle(root.path(), &all(LINUX)[..2], None);
        let bundle = Bundle::open(root.path()).unwrap();
        assert!(matches!(
            bundle.payloads(LINUX),
            Err(BuildsError::FileMissing { file: InstalledFile::Cli, .. })
        ));
    }

    #[test]
    fn files_beside_the_daemon_make_a_host_bundle() {
        let dir = tempfile::tempdir().unwrap();
        for file in InstalledFile::ALL {
            std::fs::write(dir.path().join(file.file_name()), file.file_name()).unwrap();
        }
        let bundle = Bundle::beside(dir.path(), LINUX);

        let payloads = bundle.payloads(LINUX).unwrap();

        assert_eq!(payloads.len(), 3);
        assert_eq!(payloads[2].bytes().as_ref(), b"humata");
    }
}
