//! Native Messaging host manifest generator and atomic installer/uninstaller.
//!
//! Generates standard Chromium (`allowed_origins`) and Firefox
//! (`allowed_extensions`) Native Messaging host manifests for
//! `app.humata2.browser_host`, writes them atomically via sibling temp file +
//! fsync + rename, and supports clean uninstall.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::detect::{BrowserFamily, DetectedBrowser};

/// Registered Native Messaging host identifier shared by the extension and daemon.
pub const NATIVE_HOST_NAME: &str = "app.humata2.browser_host";

/// Human-readable description embedded in the Native Messaging manifest.
pub const NATIVE_HOST_DESCRIPTION: &str = "Humata 2 Native Browser Co-Pilot Host";

/// Default Firefox WebExtension add-on ID (`browser_specific_settings.gecko.id`).
pub const DEFAULT_FIREFOX_EXTENSION_ID: &str = "humata2-copilot@humata.app";

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Errors returned while generating, installing, or removing Native Messaging manifests.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("native messaging host binary path must be absolute: {0}")]
    NonAbsoluteHostPath(PathBuf),
    #[error("native messaging host name must not be empty")]
    EmptyHostName,
    #[error("at least one allowed origin or extension ID is required")]
    EmptyAllowedList,
    #[error("invalid Chromium allowed_origin (expected chrome-extension://<id>/): {0}")]
    InvalidChromiumOrigin(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Configuration for generating Native Messaging host manifests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestConfig {
    pub host_name: String,
    pub description: String,
    pub host_binary_path: PathBuf,
    pub chromium_origins: Vec<String>,
    pub firefox_extensions: Vec<String>,
}

impl ManifestConfig {
    /// Creates a manifest configuration with the default `app.humata2.browser_host` name
    /// and default Firefox extension ID.
    pub fn new(host_binary_path: impl Into<PathBuf>, chromium_origins: Vec<String>) -> Self {
        Self {
            host_name: NATIVE_HOST_NAME.to_string(),
            description: NATIVE_HOST_DESCRIPTION.to_string(),
            host_binary_path: host_binary_path.into(),
            chromium_origins,
            firefox_extensions: vec![DEFAULT_FIREFOX_EXTENSION_ID.to_string()],
        }
    }
}

/// Chromium-family Native Messaging host manifest (`allowed_origins`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChromiumManifest {
    pub name: String,
    pub description: String,
    pub path: String,
    #[serde(rename = "type")]
    pub host_type: String,
    pub allowed_origins: Vec<String>,
}

/// Firefox-family Native Messaging host manifest (`allowed_extensions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirefoxManifest {
    pub name: String,
    pub description: String,
    pub path: String,
    #[serde(rename = "type")]
    pub host_type: String,
    pub allowed_extensions: Vec<String>,
}

/// Generates, atomically installs, and uninstalls Native Messaging manifests.
pub struct ManifestInstaller;

impl ManifestInstaller {
    /// Returns the manifest file path inside `dir` (`<dir>/<host_name>.json`).
    pub fn manifest_path(dir: &Path, host_name: &str) -> PathBuf {
        dir.join(format!("{host_name}.json"))
    }

    /// Renders a validated pretty-printed JSON manifest for the given browser family.
    pub fn build_manifest_json(
        family: BrowserFamily,
        config: &ManifestConfig,
    ) -> Result<String, ManifestError> {
        if config.host_name.trim().is_empty() {
            return Err(ManifestError::EmptyHostName);
        }
        if !config.host_binary_path.is_absolute() {
            return Err(ManifestError::NonAbsoluteHostPath(
                config.host_binary_path.clone(),
            ));
        }
        let path_str = config.host_binary_path.to_string_lossy().into_owned();

        let json = match family {
            BrowserFamily::Chromium => {
                if config.chromium_origins.is_empty() {
                    return Err(ManifestError::EmptyAllowedList);
                }
                for origin in &config.chromium_origins {
                    if !is_valid_chromium_origin(origin) {
                        return Err(ManifestError::InvalidChromiumOrigin(origin.clone()));
                    }
                }
                let manifest = ChromiumManifest {
                    name: config.host_name.clone(),
                    description: config.description.clone(),
                    path: path_str,
                    host_type: "stdio".to_string(),
                    allowed_origins: config.chromium_origins.clone(),
                };
                serde_json::to_string_pretty(&manifest)?
            }
            BrowserFamily::Firefox => {
                if config.firefox_extensions.is_empty() {
                    return Err(ManifestError::EmptyAllowedList);
                }
                let manifest = FirefoxManifest {
                    name: config.host_name.clone(),
                    description: config.description.clone(),
                    path: path_str,
                    host_type: "stdio".to_string(),
                    allowed_extensions: config.firefox_extensions.clone(),
                };
                serde_json::to_string_pretty(&manifest)?
            }
        };

        Ok(format!("{json}\n"))
    }

    /// Atomically installs the Native Messaging manifest for a detected browser.
    pub fn install_for_browser(
        browser: &DetectedBrowser,
        config: &ManifestConfig,
    ) -> Result<PathBuf, ManifestError> {
        Self::install_to_dir(&browser.native_messaging_dir, browser.family, config)
    }

    /// Atomically installs the Native Messaging manifest into `dir`.
    pub fn install_to_dir(
        dir: &Path,
        family: BrowserFamily,
        config: &ManifestConfig,
    ) -> Result<PathBuf, ManifestError> {
        let payload = Self::build_manifest_json(family, config)?;
        fs::create_dir_all(dir)?;

        let final_path = Self::manifest_path(dir, &config.host_name);
        let pid = std::process::id();
        let seq = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = dir.join(format!(".{}.json.{}.{}.tmp", config.host_name, pid, seq));

        let write_result = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp_path)?;
            file.write_all(payload.as_bytes())?;
            file.sync_all()?;
            Ok(())
        })();

        if let Err(err) = write_result {
            let _ = fs::remove_file(&tmp_path);
            return Err(ManifestError::Io(err));
        }

        if let Err(err) = fs::rename(&tmp_path, &final_path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(ManifestError::Io(err));
        }

        Ok(final_path)
    }

    /// Uninstalls the Native Messaging manifest for a detected browser.
    /// Returns `Ok(true)` if a file was removed, or `Ok(false)` if it did not exist.
    pub fn uninstall_for_browser(
        browser: &DetectedBrowser,
        host_name: &str,
    ) -> Result<bool, ManifestError> {
        Self::uninstall_from_dir(&browser.native_messaging_dir, host_name)
    }

    /// Uninstalls `<dir>/<host_name>.json`.
    pub fn uninstall_from_dir(dir: &Path, host_name: &str) -> Result<bool, ManifestError> {
        if host_name.trim().is_empty() {
            return Err(ManifestError::EmptyHostName);
        }
        let path = Self::manifest_path(dir, host_name);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(ManifestError::Io(err)),
        }
    }
}

fn is_valid_chromium_origin(origin: &str) -> bool {
    let Some(rest) = origin.strip_prefix("chrome-extension://") else {
        return false;
    };
    let Some(id) = rest.strip_suffix('/') else {
        return false;
    };
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn sample_config() -> ManifestConfig {
        ManifestConfig::new(
            "/usr/local/bin/humata2-browser-host",
            vec!["chrome-extension://abcdefghijklmnopabcdefghijklmnop/".to_string()],
        )
    }

    #[test]
    fn installs_and_uninstalls_chromium_manifest_atomically() {
        let tmp = TempDir::new().expect("tempdir");
        let dir = tmp.path().join("Chrome/NativeMessagingHosts");
        let cfg = sample_config();

        let installed =
            ManifestInstaller::install_to_dir(&dir, BrowserFamily::Chromium, &cfg).unwrap();
        assert_eq!(installed, dir.join("app.humata2.browser_host.json"));
        assert!(installed.exists());

        let raw = fs::read_to_string(&installed).unwrap();
        let parsed: ChromiumManifest = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.name, NATIVE_HOST_NAME);
        assert_eq!(parsed.host_type, "stdio");
        assert_eq!(parsed.path, "/usr/local/bin/humata2-browser-host");
        assert_eq!(parsed.allowed_origins, cfg.chromium_origins);

        // No stray temp files left behind.
        let entries: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["app.humata2.browser_host.json"]);

        // Re-install overwrites cleanly.
        let mut updated = cfg.clone();
        updated.host_binary_path = PathBuf::from("/opt/humata2/bin/browser-host");
        ManifestInstaller::install_to_dir(&dir, BrowserFamily::Chromium, &updated).unwrap();
        let reparsed: ChromiumManifest =
            serde_json::from_str(&fs::read_to_string(&installed).unwrap()).unwrap();
        assert_eq!(reparsed.path, "/opt/humata2/bin/browser-host");

        // Uninstall removes the file and is idempotent.
        assert!(ManifestInstaller::uninstall_from_dir(&dir, NATIVE_HOST_NAME).unwrap());
        assert!(!installed.exists());
        assert!(!ManifestInstaller::uninstall_from_dir(&dir, NATIVE_HOST_NAME).unwrap());
    }

    #[test]
    fn installs_and_uninstalls_firefox_manifest_atomically() {
        let tmp = TempDir::new().expect("tempdir");
        let dir = tmp.path().join("Mozilla/NativeMessagingHosts");
        let cfg = sample_config();

        let installed =
            ManifestInstaller::install_to_dir(&dir, BrowserFamily::Firefox, &cfg).unwrap();
        assert_eq!(installed, dir.join("app.humata2.browser_host.json"));

        let raw = fs::read_to_string(&installed).unwrap();
        let parsed: FirefoxManifest = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed.name, NATIVE_HOST_NAME);
        assert_eq!(parsed.host_type, "stdio");
        assert_eq!(parsed.allowed_extensions, vec![DEFAULT_FIREFOX_EXTENSION_ID]);

        assert!(ManifestInstaller::uninstall_from_dir(&dir, NATIVE_HOST_NAME).unwrap());
        assert!(!installed.exists());
    }

    #[test]
    fn rejects_invalid_config() {
        let mut bad_path = sample_config();
        bad_path.host_binary_path = PathBuf::from("relative/bin");
        assert!(matches!(
            ManifestInstaller::build_manifest_json(BrowserFamily::Chromium, &bad_path),
            Err(ManifestError::NonAbsoluteHostPath(_))
        ));

        let mut bad_origin = sample_config();
        bad_origin.chromium_origins = vec!["https://example.com/".to_string()];
        assert!(matches!(
            ManifestInstaller::build_manifest_json(BrowserFamily::Chromium, &bad_origin),
            Err(ManifestError::InvalidChromiumOrigin(_))
        ));
    }
}
