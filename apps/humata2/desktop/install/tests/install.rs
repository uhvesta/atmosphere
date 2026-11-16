use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use humata2_install::{
    install, AgentChange, BinaryChange, Bundle, InstallError, Launchd, LaunchdError, Layout,
};

#[derive(Default)]
struct FakeLaunchd {
    calls: Mutex<Vec<String>>,
    loaded: Mutex<bool>,
}

impl FakeLaunchd {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn count(&self, prefix: &str) -> usize {
        self.calls().iter().filter(|c| c.starts_with(prefix)).count()
    }
}

impl Launchd for FakeLaunchd {
    fn bootstrap(&self, _plist: &Path) -> Result<(), LaunchdError> {
        self.calls.lock().unwrap().push("bootstrap".into());
        *self.loaded.lock().unwrap() = true;
        Ok(())
    }
    fn bootout(&self, _label: &str) -> Result<(), LaunchdError> {
        self.calls.lock().unwrap().push("bootout".into());
        *self.loaded.lock().unwrap() = false;
        Ok(())
    }
    fn is_loaded(&self, _label: &str) -> Result<bool, LaunchdError> {
        Ok(*self.loaded.lock().unwrap())
    }
    fn kickstart(&self, _label: &str) -> Result<(), LaunchdError> {
        self.calls.lock().unwrap().push("kickstart".into());
        Ok(())
    }
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let f = Self { dir };
        fs::create_dir_all(f.app()).unwrap();
        f.bundle_write("humatad2", "daemon v1");
        f.bundle_write("humata2_holder", "holder v1");
        f.bundle_write("humata", "cli v1");
        f
    }
    fn app(&self) -> PathBuf {
        self.dir.path().join("app")
    }
    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }
    fn bundle_write(&self, name: &str, body: &str) {
        fs::write(self.app().join(name), body).unwrap();
    }
    fn bundle(&self) -> Bundle {
        Bundle::new(
            self.app().join("humatad2"),
            self.app().join("humata2_holder"),
            self.app().join("humata"),
        )
    }
    fn layout(&self) -> Layout {
        Layout::for_home(&self.home()).with_path(Some(OsString::from("/usr/bin")))
    }
    fn bin(&self, name: &str) -> PathBuf {
        self.home().join(".humata2/bin").join(name)
    }
}

#[test]
fn first_install_copies_everything_and_registers() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    let done = install(&fx.bundle(), &fx.layout(), &ld).unwrap();

    assert_eq!(done.humatad2, BinaryChange::Installed);
    assert_eq!(done.holder, BinaryChange::Installed);
    assert_eq!(done.humata, BinaryChange::Installed);
    assert_eq!(done.agent, AgentChange::Reloaded);
    assert_eq!(fs::read_to_string(fx.bin("humatad2")).unwrap(), "daemon v1");
    assert_eq!(fs::read_to_string(fx.bin("humata2_holder")).unwrap(), "holder v1");
    assert!(fx
        .home()
        .join("Library/LaunchAgents/app.humata2.humatad2.plist")
        .is_file());
    assert_eq!(ld.count("bootstrap"), 1);
    assert_eq!(
        fs::read_link(fx.home().join(".local/bin/humata")).unwrap(),
        fx.bin("humata")
    );
}

#[test]
fn second_install_changes_nothing() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    let done = install(&fx.bundle(), &fx.layout(), &ld).unwrap();

    assert_eq!(done.humatad2, BinaryChange::Unchanged);
    assert_eq!(done.holder, BinaryChange::Unchanged);
    assert_eq!(done.humata, BinaryChange::Unchanged);
    assert_eq!(done.agent, AgentChange::Unchanged);
    assert_eq!(ld.count("bootstrap"), 1);
}

#[test]
fn changed_daemon_is_replaced_and_reloaded() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    fx.bundle_write("humatad2", "daemon v2");
    let done = install(&fx.bundle(), &fx.layout(), &ld).unwrap();

    assert_eq!(done.humatad2, BinaryChange::Updated);
    assert_eq!(done.holder, BinaryChange::Unchanged);
    assert_eq!(done.agent, AgentChange::Reloaded);
    assert_eq!(fs::read_to_string(fx.bin("humatad2")).unwrap(), "daemon v2");
    assert_eq!(ld.count("bootstrap"), 2);
}

#[test]
fn changed_holder_is_replaced_without_reload() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    fx.bundle_write("humata2_holder", "holder v2");
    let done = install(&fx.bundle(), &fx.layout(), &ld).unwrap();

    assert_eq!(done.holder, BinaryChange::Updated);
    assert_eq!(done.humatad2, BinaryChange::Unchanged);
    assert_eq!(done.agent, AgentChange::Unchanged);
    assert_eq!(fs::read_to_string(fx.bin("humata2_holder")).unwrap(), "holder v2");
    assert_eq!(ld.count("bootstrap"), 1);
}

#[test]
fn unloaded_agent_is_loaded_again() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    *ld.loaded.lock().unwrap() = false;
    let done = install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    assert_eq!(done.agent, AgentChange::Loaded);
}

#[test]
fn leftover_partial_does_not_block() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.home().join(".humata2/bin")).unwrap();
    fs::write(fx.home().join(".humata2/bin/.humatad2.partial"), "junk").unwrap();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    assert_eq!(fs::read_to_string(fx.bin("humatad2")).unwrap(), "daemon v1");
    assert!(!fx.home().join(".humata2/bin/.humatad2.partial").exists());
}

#[test]
fn regular_file_at_link_is_kept_and_reported() {
    let fx = Fixture::new();
    let dir = fx.home().join(".local/bin");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("humata"), "mine").unwrap();
    let ld = FakeLaunchd::default();
    let done = install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    assert_eq!(done.path_clash, Some(dir.join("humata")));
    assert_eq!(done.humatad2, BinaryChange::Installed);
    assert_eq!(ld.count("bootstrap"), 1);
    assert_eq!(fs::read_to_string(dir.join("humata")).unwrap(), "mine");
}

#[test]
fn foreign_symlink_is_kept_and_reported() {
    let fx = Fixture::new();
    let dir = fx.home().join(".local/bin");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink("/nonexistent", dir.join("humata")).unwrap();
    let done = install(&fx.bundle(), &fx.layout(), &FakeLaunchd::default()).unwrap();
    assert_eq!(done.path_clash, Some(dir.join("humata")));
    assert_eq!(fs::read_link(dir.join("humata")).unwrap(), PathBuf::from("/nonexistent"));
}

#[test]
fn dangling_link_into_bin_is_replaced() {
    let fx = Fixture::new();
    let dir = fx.home().join(".local/bin");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(fx.home().join(".humata2/bin/old"), dir.join("humata")).unwrap();
    let done = install(&fx.bundle(), &fx.layout(), &FakeLaunchd::default()).unwrap();
    assert_eq!(done.path_clash, None);
    assert_eq!(fs::read_link(dir.join("humata")).unwrap(), fx.bin("humata"));
}

#[test]
fn path_hint_depends_on_path() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    let missing = install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    assert!(missing.path_hint.is_some());

    let with = OsString::from(format!("/usr/bin:{}", fx.home().join(".local/bin").display()));
    let layout = Layout::for_home(&fx.home()).with_path(Some(with));
    let present = install(&fx.bundle(), &layout, &ld).unwrap();
    assert!(present.path_hint.is_none());
}

#[test]
fn missing_bundled_file_is_an_error() {
    let fx = Fixture::new();
    fs::remove_file(fx.app().join("humata2_holder")).unwrap();
    let err = install(&fx.bundle(), &fx.layout(), &FakeLaunchd::default()).unwrap_err();
    assert!(matches!(err, InstallError::ReadBundle { .. }));
}
