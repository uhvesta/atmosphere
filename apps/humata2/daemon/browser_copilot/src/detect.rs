//! Auto-detection of installed desktop browsers on macOS and Linux.
//!
//! All filesystem lookups are rooted at an injectable `fs_root` and `home_dir`
//! so unit tests remain completely deterministic and never touch real user
//! browser profiles or system directories.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Supported browser families for Native Messaging manifest generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFamily {
    Chromium,
    Firefox,
}

/// Supported desktop browsers for Humata 2's Native Browser Co-Pilot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserKind {
    Chrome,
    Brave,
    Edge,
    Chromium,
    Arc,
    Vivaldi,
    Firefox,
}

impl BrowserKind {
    /// All supported browser kinds in deterministic detection order.
    pub const ALL: &'static [BrowserKind] = &[
        BrowserKind::Chrome,
        BrowserKind::Brave,
        BrowserKind::Edge,
        BrowserKind::Chromium,
        BrowserKind::Arc,
        BrowserKind::Vivaldi,
        BrowserKind::Firefox,
    ];

    /// Native Messaging manifest family for this browser.
    pub const fn family(self) -> BrowserFamily {
        match self {
            BrowserKind::Firefox => BrowserFamily::Firefox,
            BrowserKind::Chrome
            | BrowserKind::Brave
            | BrowserKind::Edge
            | BrowserKind::Chromium
            | BrowserKind::Arc
            | BrowserKind::Vivaldi => BrowserFamily::Chromium,
        }
    }

    /// Stable machine identifier.
    pub const fn id(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "chrome",
            BrowserKind::Brave => "brave",
            BrowserKind::Edge => "edge",
            BrowserKind::Chromium => "chromium",
            BrowserKind::Arc => "arc",
            BrowserKind::Vivaldi => "vivaldi",
            BrowserKind::Firefox => "firefox",
        }
    }

    /// Human-readable browser name.
    pub const fn display_name(self) -> &'static str {
        match self {
            BrowserKind::Chrome => "Google Chrome",
            BrowserKind::Brave => "Brave Browser",
            BrowserKind::Edge => "Microsoft Edge",
            BrowserKind::Chromium => "Chromium",
            BrowserKind::Arc => "Arc",
            BrowserKind::Vivaldi => "Vivaldi",
            BrowserKind::Firefox => "Mozilla Firefox",
        }
    }
}

/// Target operating system layout for browser discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    MacOs,
    Linux,
}

impl Platform {
    /// Returns the compile-time host platform (defaults to `Linux` on other Unix targets).
    pub const fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            Platform::MacOs
        }
        #[cfg(not(target_os = "macos"))]
        {
            Platform::Linux
        }
    }
}

/// A detected browser installation and its Native Messaging host directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedBrowser {
    pub kind: BrowserKind,
    pub family: BrowserFamily,
    pub binary_path: PathBuf,
    pub native_messaging_dir: PathBuf,
}

/// Discovers installed browsers against an injectable filesystem layout.
#[derive(Debug, Clone)]
pub struct BrowserDetector {
    platform: Platform,
    fs_root: PathBuf,
    home_dir: PathBuf,
}

impl BrowserDetector {
    /// Creates a detector using an explicit platform, filesystem root, and home directory.
    ///
    /// For real hosts, pass `PathBuf::from("/")` as `fs_root` and the user's home directory
    /// as `home_dir`. For tests, pass paths inside a `tempfile::TempDir`.
    pub fn new(platform: Platform, fs_root: impl Into<PathBuf>, home_dir: impl Into<PathBuf>) -> Self {
        Self {
            platform,
            fs_root: fs_root.into(),
            home_dir: home_dir.into(),
        }
    }

    /// Returns the configured platform.
    pub const fn platform(&self) -> Platform {
        self.platform
    }

    /// Returns the Native Messaging host manifest directory for `kind` on this platform.
    pub fn native_messaging_dir(&self, kind: BrowserKind) -> PathBuf {
        match self.platform {
            Platform::MacOs => self.macos_native_messaging_dir(kind),
            Platform::Linux => self.linux_native_messaging_dir(kind),
        }
    }

    /// Detects a single browser kind if any of its standard app bundles or binaries exist.
    pub fn detect(&self, kind: BrowserKind) -> Option<DetectedBrowser> {
        let candidates = self.candidate_paths(kind);
        let binary_path = candidates.into_iter().find(|p| path_exists(p))?;
        Some(DetectedBrowser {
            kind,
            family: kind.family(),
            binary_path,
            native_messaging_dir: self.native_messaging_dir(kind),
        })
    }

    /// Detects all installed supported browsers in deterministic order.
    pub fn detect_all(&self) -> Vec<DetectedBrowser> {
        BrowserKind::ALL
            .iter()
            .filter_map(|&kind| self.detect(kind))
            .collect()
    }

    fn macos_native_messaging_dir(&self, kind: BrowserKind) -> PathBuf {
        let app_support = self.home_dir.join("Library/Application Support");
        match kind {
            BrowserKind::Chrome => app_support.join("Google/Chrome/NativeMessagingHosts"),
            BrowserKind::Brave => {
                app_support.join("BraveSoftware/Brave-Browser/NativeMessagingHosts")
            }
            BrowserKind::Edge => app_support.join("Microsoft Edge/NativeMessagingHosts"),
            BrowserKind::Chromium => app_support.join("Chromium/NativeMessagingHosts"),
            BrowserKind::Arc => app_support.join("Arc/User Data/NativeMessagingHosts"),
            BrowserKind::Vivaldi => app_support.join("Vivaldi/NativeMessagingHosts"),
            BrowserKind::Firefox => app_support.join("Mozilla/NativeMessagingHosts"),
        }
    }

    fn linux_native_messaging_dir(&self, kind: BrowserKind) -> PathBuf {
        let config = self.home_dir.join(".config");
        match kind {
            BrowserKind::Chrome => config.join("google-chrome/NativeMessagingHosts"),
            BrowserKind::Brave => config.join("BraveSoftware/Brave-Browser/NativeMessagingHosts"),
            BrowserKind::Edge => config.join("microsoft-edge/NativeMessagingHosts"),
            BrowserKind::Chromium => config.join("chromium/NativeMessagingHosts"),
            BrowserKind::Arc => config.join("arc/NativeMessagingHosts"),
            BrowserKind::Vivaldi => config.join("vivaldi/NativeMessagingHosts"),
            BrowserKind::Firefox => self.home_dir.join(".mozilla/native-messaging-hosts"),
        }
    }

    fn candidate_paths(&self, kind: BrowserKind) -> Vec<PathBuf> {
        match self.platform {
            Platform::MacOs => {
                let app_name = match kind {
                    BrowserKind::Chrome => "Google Chrome.app",
                    BrowserKind::Brave => "Brave Browser.app",
                    BrowserKind::Edge => "Microsoft Edge.app",
                    BrowserKind::Chromium => "Chromium.app",
                    BrowserKind::Arc => "Arc.app",
                    BrowserKind::Vivaldi => "Vivaldi.app",
                    BrowserKind::Firefox => "Firefox.app",
                };
                vec![
                    self.fs_root.join("Applications").join(app_name),
                    self.home_dir.join("Applications").join(app_name),
                ]
            }
            Platform::Linux => {
                let rel_paths: &[&str] = match kind {
                    BrowserKind::Chrome => &[
                        "usr/bin/google-chrome",
                        "usr/bin/google-chrome-stable",
                        "opt/google/chrome/google-chrome",
                    ],
                    BrowserKind::Brave => &[
                        "usr/bin/brave-browser",
                        "usr/bin/brave-browser-stable",
                        "opt/brave.com/brave/brave-browser",
                        "snap/bin/brave",
                    ],
                    BrowserKind::Edge => &[
                        "usr/bin/microsoft-edge",
                        "usr/bin/microsoft-edge-stable",
                        "opt/microsoft/msedge/microsoft-edge",
                    ],
                    BrowserKind::Chromium => &[
                        "usr/bin/chromium",
                        "usr/bin/chromium-browser",
                        "snap/bin/chromium",
                    ],
                    BrowserKind::Arc => &["usr/bin/arc", "usr/local/bin/arc"],
                    BrowserKind::Vivaldi => &[
                        "usr/bin/vivaldi",
                        "usr/bin/vivaldi-stable",
                        "opt/vivaldi/vivaldi",
                    ],
                    BrowserKind::Firefox => &[
                        "usr/bin/firefox",
                        "usr/bin/firefox-esr",
                        "snap/bin/firefox",
                        "opt/firefox/firefox",
                    ],
                };
                let mut paths: Vec<PathBuf> =
                    rel_paths.iter().map(|rel| self.fs_root.join(rel)).collect();
                paths.push(self.home_dir.join(".local/bin").join(kind.id()));
                paths
            }
        }
    }
}

fn path_exists(path: &Path) -> bool {
    path.exists()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(path, b"").expect("create dummy file");
    }

    #[test]
    fn detects_browsers_on_macos_layout() {
        let tmp = TempDir::new().expect("tempdir");
        let fs_root = tmp.path().join("root");
        let home = tmp.path().join("Users/alice");

        fs::create_dir_all(fs_root.join("Applications/Google Chrome.app")).unwrap();
        fs::create_dir_all(fs_root.join("Applications/Brave Browser.app")).unwrap();
        fs::create_dir_all(fs_root.join("Applications/Microsoft Edge.app")).unwrap();
        fs::create_dir_all(home.join("Applications/Chromium.app")).unwrap();
        fs::create_dir_all(fs_root.join("Applications/Firefox.app")).unwrap();

        let detector = BrowserDetector::new(Platform::MacOs, &fs_root, &home);
        let found = detector.detect_all();

        let kinds: Vec<BrowserKind> = found.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BrowserKind::Chrome,
                BrowserKind::Brave,
                BrowserKind::Edge,
                BrowserKind::Chromium,
                BrowserKind::Firefox,
            ]
        );

        let chrome = found.iter().find(|b| b.kind == BrowserKind::Chrome).unwrap();
        assert_eq!(chrome.family, BrowserFamily::Chromium);
        assert_eq!(
            chrome.native_messaging_dir,
            home.join("Library/Application Support/Google/Chrome/NativeMessagingHosts")
        );

        let chromium = found
            .iter()
            .find(|b| b.kind == BrowserKind::Chromium)
            .unwrap();
        assert_eq!(chromium.binary_path, home.join("Applications/Chromium.app"));

        let firefox = found.iter().find(|b| b.kind == BrowserKind::Firefox).unwrap();
        assert_eq!(firefox.family, BrowserFamily::Firefox);
        assert_eq!(
            firefox.native_messaging_dir,
            home.join("Library/Application Support/Mozilla/NativeMessagingHosts")
        );
    }

    #[test]
    fn detects_browsers_on_linux_layout() {
        let tmp = TempDir::new().expect("tempdir");
        let fs_root = tmp.path().join("root");
        let home = tmp.path().join("home/bob");

        touch(&fs_root.join("usr/bin/google-chrome-stable"));
        touch(&fs_root.join("opt/brave.com/brave/brave-browser"));
        touch(&fs_root.join("usr/bin/microsoft-edge"));
        touch(&fs_root.join("usr/bin/chromium-browser"));
        touch(&fs_root.join("usr/bin/firefox"));

        let detector = BrowserDetector::new(Platform::Linux, &fs_root, &home);
        let found = detector.detect_all();

        let kinds: Vec<BrowserKind> = found.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            vec![
                BrowserKind::Chrome,
                BrowserKind::Brave,
                BrowserKind::Edge,
                BrowserKind::Chromium,
                BrowserKind::Firefox,
            ]
        );

        let brave = found.iter().find(|b| b.kind == BrowserKind::Brave).unwrap();
        assert_eq!(brave.family, BrowserFamily::Chromium);
        assert_eq!(
            brave.native_messaging_dir,
            home.join(".config/BraveSoftware/Brave-Browser/NativeMessagingHosts")
        );

        let firefox = found.iter().find(|b| b.kind == BrowserKind::Firefox).unwrap();
        assert_eq!(firefox.family, BrowserFamily::Firefox);
        assert_eq!(
            firefox.native_messaging_dir,
            home.join(".mozilla/native-messaging-hosts")
        );
    }

    #[test]
    fn returns_empty_when_no_browsers_installed() {
        let tmp = TempDir::new().expect("tempdir");
        let detector = BrowserDetector::new(Platform::Linux, tmp.path(), tmp.path().join("home"));
        assert!(detector.detect_all().is_empty());
        assert!(detector.detect(BrowserKind::Chrome).is_none());
    }
}
