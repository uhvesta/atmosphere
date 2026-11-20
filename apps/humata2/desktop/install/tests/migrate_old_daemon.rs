//! D374, D281 b: the app migrates a local daemon too old to hold its own
//! Migrate item. `install` has staged a newer release beside the installed
//! one (`.<name>.next` and `.bundle.json.next`). `migrate_old_daemon` checks
//! the staged files against the staged bundle, renames them into place with
//! the bundle last, and kickstarts the login item so the daemon starts at
//! the new release.
//!
//! The function these tests expect, in `humata2_install`:
//!
//! ```ignore
//! pub fn migrate_old_daemon(layout: &Layout, launchd: &dyn Launchd) -> Result<T, InstallError>
//! ```
//!
//! The Ok value `T` is the implementer's pick; these tests only need Ok or
//! Err. A staged file that isn't what the staged bundle names is an Err
//! with nothing changed. With nothing staged it is Ok and changes nothing.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use humata2_install::{install, migrate_old_daemon, staged_release, Bundle, Launchd, LaunchdError, Layout};
use humata2_machines::{InstalledFile, Payload};

/// The installed files and the staged ones, by name.
const INSTALLED: [&str; 6] = [
    "humatad2",
    "humata2_holder",
    "humata",
    "stack",
    "humata2_fusefs",
    "bundle.json",
];
const STAGED: [&str; 6] = [
    ".humatad2.next",
    ".humata2_holder.next",
    ".humata.next",
    ".stack.next",
    ".humata2_fusefs.next",
    ".bundle.json.next",
];

/// launchd as the tests see it: what was asked of it, in order.
#[derive(Default)]
struct FakeLaunchd {
    calls: Mutex<Vec<String>>,
    loaded: Mutex<bool>,
}

impl FakeLaunchd {
    fn count(&self, call: &str) -> usize {
        self.calls
            .lock()
            .expect("the fake's calls aren't poisoned")
            .iter()
            .filter(|made| made.as_str() == call)
            .count()
    }

    fn record(&self, call: &str) {
        self.calls.lock().expect("the fake's calls aren't poisoned").push(call.to_owned());
    }
}

impl Launchd for FakeLaunchd {
    fn bootstrap(&self, _plist: &Path) -> Result<(), LaunchdError> {
        self.record("bootstrap");
        *self.loaded.lock().expect("the fake isn't poisoned") = true;
        Ok(())
    }
    fn bootout(&self, _label: &str) -> Result<(), LaunchdError> {
        self.record("bootout");
        *self.loaded.lock().expect("the fake isn't poisoned") = false;
        Ok(())
    }
    fn is_loaded(&self, _label: &str) -> Result<bool, LaunchdError> {
        Ok(*self.loaded.lock().expect("the fake isn't poisoned"))
    }
    fn kickstart(&self, _label: &str) -> Result<(), LaunchdError> {
        self.record("kickstart");
        Ok(())
    }
}

/// The sha256 of `bytes`, as a bundle names it.
fn sha_hex(bytes: &[u8]) -> String {
    Payload::new(InstalledFile::Cli, bytes.into()).sha256().to_owned()
}

/// An app folder with release 1 in it, and a home its install goes to.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Self {
            dir: tempfile::tempdir().expect("a temporary folder"),
        };
        fs::create_dir_all(fixture.app()).expect("the app folder is made");
        fixture.app_file("humatad2", "daemon v1");
        fixture.app_file("humata2_holder", "holder v1");
        fixture.app_file("humata", "cli v1");
        fixture.app_file("stack", "stack v1");
        fixture.app_file("humata2_fusefs", "fusefs v1");
        fixture
    }

    fn app(&self) -> PathBuf {
        self.dir.path().join("app")
    }

    fn app_file(&self, name: &str, body: &str) {
        fs::write(self.app().join(name), body).expect("the app's file is written");
    }

    fn bundle(&self) -> Bundle {
        Bundle::new(
            self.app().join("humatad2"),
            self.app().join("humata2_holder"),
            self.app().join("humata"),
            self.app().join("stack"),
            self.app().join("humata2_fusefs"),
        )
    }

    fn layout(&self) -> Layout {
        Layout::for_home(&self.dir.path().join("home")).with_path(Some(OsString::from("/usr/bin")))
    }

    fn bin(&self, name: &str) -> PathBuf {
        self.dir.path().join("home/.humata2/bin").join(name)
    }

    /// Release 1 installed, then release 2 (a new daemon and CLI) staged
    /// beside it by the next launch's install.
    fn with_release_2_staged(&self, launchd: &FakeLaunchd) {
        install(&self.bundle(), &self.layout(), launchd).expect("release 1 installs");
        self.app_file("humatad2", "daemon v2");
        self.app_file("humata", "cli v2");
        self.app_file("humata2_fusefs", "fusefs v2");
        install(&self.bundle(), &self.layout(), launchd).expect("release 2 is staged");
        assert!(
            staged_release(&self.layout()).expect("the staged release reads").is_some(),
            "release 2 is staged"
        );
    }

    /// Every file in bin, installed and staged, by name; None when missing.
    fn files(&self) -> BTreeMap<&'static str, Option<Vec<u8>>> {
        INSTALLED
            .into_iter()
            .chain(STAGED)
            .map(|name| (name, fs::read(self.bin(name)).ok()))
            .collect()
    }
}

#[test]
fn migrating_puts_the_staged_release_in_place_and_kickstarts_the_daemon_once() {
    let fixture = Fixture::new();
    let launchd = FakeLaunchd::default();
    fixture.with_release_2_staged(&launchd);
    let staged_bundle = fs::read(fixture.bin(".bundle.json.next")).expect("the staged bundle reads");
    let kickstarts = launchd.count("kickstart");

    migrate_old_daemon(&fixture.layout(), &launchd).expect("the staged release is put in place");

    assert_eq!(fs::read(fixture.bin("bundle.json")).expect("bundle.json reads"), staged_bundle);
    let manifest: serde_json::Value = serde_json::from_slice(&staged_bundle).expect("the staged bundle is JSON");
    let platform = humata2_machines::Platform::host()
        .expect("the tests run on a platform Humata ships")
        .bundle_name();
    for name in ["humatad2", "humata2_holder", "humata", "stack", "humata2_fusefs"] {
        let installed = fs::read(fixture.bin(name)).unwrap_or_else(|error| panic!("{name} is installed: {error}"));
        assert_eq!(
            manifest["platforms"][platform][name]["sha256"],
            sha_hex(&installed),
            "{name} is the staged release's"
        );
    }
    for name in STAGED {
        assert!(!fixture.bin(name).exists(), "{name} is no longer staged");
    }
    assert_eq!(staged_release(&fixture.layout()).expect("the staged release reads"), None);
    assert_eq!(launchd.count("kickstart"), kickstarts + 1, "the daemon is started again once");
}

#[test]
fn a_staged_file_that_is_not_the_staged_bundles_is_refused_with_nothing_changed() {
    let fixture = Fixture::new();
    let launchd = FakeLaunchd::default();
    fixture.with_release_2_staged(&launchd);
    fs::write(fixture.bin(".humatad2.next"), "somebody else's daemon").expect("the staged daemon is replaced");
    let before = fixture.files();
    let kickstarts = launchd.count("kickstart");

    let refused = migrate_old_daemon(&fixture.layout(), &launchd);

    assert!(refused.is_err(), "a staged daemon the bundle doesn't name is refused");
    assert_eq!(fixture.files(), before, "nothing in bin changed");
    assert_eq!(launchd.count("kickstart"), kickstarts, "the daemon isn't started again");
}

#[test]
fn with_nothing_staged_migrating_changes_nothing() {
    let fixture = Fixture::new();
    let launchd = FakeLaunchd::default();
    install(&fixture.bundle(), &fixture.layout(), &launchd).expect("release 1 installs");
    let before = fixture.files();
    let kickstarts = launchd.count("kickstart");

    migrate_old_daemon(&fixture.layout(), &launchd).expect("nothing to do is not a failure");

    assert_eq!(fixture.files(), before, "nothing in bin changed");
    assert_eq!(launchd.count("kickstart"), kickstarts, "the daemon isn't started again");
}
