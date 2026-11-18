use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use std::os::unix::fs::PermissionsExt;

use humata2_install::{
    install, install_machine_tools, staged_release, AgentChange, BinaryChange, Bundle, InstallError,
    Launchd, LaunchdError, Layout,
};
use humata2_machines::{InstalledFile, Payload};

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

/// The sha256 of `bytes`, as a bundle names it.
fn sha_hex(bytes: &[u8]) -> String {
    Payload::new(InstalledFile::Cli, bytes.into()).sha256().to_owned()
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
        f.bundle_write("stack", "stack v1");
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
            self.app().join("stack"),
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
    assert_eq!(done.stack, BinaryChange::Installed);
    assert_eq!(done.agent, AgentChange::Reloaded);
    assert_eq!(fs::read_to_string(fx.bin("humatad2")).unwrap(), "daemon v1");
    assert_eq!(fs::read_to_string(fx.bin("humata2_holder")).unwrap(), "holder v1");
    assert_eq!(fs::read_to_string(fx.bin("stack")).unwrap(), "stack v1");
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
    assert_eq!(done.stack, BinaryChange::Unchanged);
    assert_eq!(done.agent, AgentChange::Unchanged);
    assert_eq!(ld.count("bootstrap"), 1);
}

#[test]
fn changed_daemon_is_staged_beside_the_installed_one_not_swapped() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    let installed_id = sha_hex(&fs::read(fx.bin("bundle.json")).unwrap());
    fx.bundle_write("humatad2", "daemon v2");
    let done = install(&fx.bundle(), &fx.layout(), &ld).unwrap();

    // What runs is untouched and the agent is not reloaded.
    assert_eq!(done.humatad2, BinaryChange::Unchanged);
    assert_eq!(done.holder, BinaryChange::Unchanged);
    assert_eq!(done.agent, AgentChange::Unchanged);
    assert_eq!(fs::read_to_string(fx.bin("humatad2")).unwrap(), "daemon v1");
    assert_eq!(ld.count("bootstrap"), 1);
    assert_eq!(ld.count("bootout"), 1);
    // The new release waits whole, named by its bundle id.
    assert_eq!(fs::read_to_string(fx.bin(".humatad2.next")).unwrap(), "daemon v2");
    assert_eq!(fs::read_to_string(fx.bin(".humata2_holder.next")).unwrap(), "holder v1");
    assert_eq!(fs::read_to_string(fx.bin(".humata.next")).unwrap(), "cli v1");
    assert_eq!(fs::read_to_string(fx.bin(".stack.next")).unwrap(), "stack v1");
    let staged = staged_release(&fx.layout()).unwrap().expect("a release is staged");
    assert_eq!(staged, sha_hex(&fs::read(fx.bin(".bundle.json.next")).unwrap()));
    assert_ne!(staged, installed_id);
    assert_eq!(sha_hex(&fs::read(fx.bin("bundle.json")).unwrap()), installed_id);
}

#[test]
fn changed_holder_is_staged_not_swapped_and_no_daemon_restarts() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    fx.bundle_write("humata2_holder", "holder v2");
    let done = install(&fx.bundle(), &fx.layout(), &ld).unwrap();

    assert_eq!(done.holder, BinaryChange::Unchanged);
    assert_eq!(done.agent, AgentChange::Unchanged);
    assert_eq!(fs::read_to_string(fx.bin("humata2_holder")).unwrap(), "holder v1");
    assert_eq!(fs::read_to_string(fx.bin(".humata2_holder.next")).unwrap(), "holder v2");
    assert_eq!(ld.count("bootstrap"), 1);
}

#[test]
fn the_same_release_stages_nothing() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();

    assert_eq!(staged_release(&fx.layout()).unwrap(), None);
    for name in [".humatad2.next", ".humata2_holder.next", ".humata.next", ".stack.next"] {
        assert!(!fx.bin(name).exists(), "{name} was staged");
    }
}

#[test]
fn staging_the_same_new_release_again_gives_the_same_bundle_id() {
    let fx = Fixture::new();
    let ld = FakeLaunchd::default();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    fx.bundle_write("humata", "cli v2");
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();
    let first = staged_release(&fx.layout()).unwrap();
    install(&fx.bundle(), &fx.layout(), &ld).unwrap();

    assert!(first.is_some());
    assert_eq!(staged_release(&fx.layout()).unwrap(), first);
}

#[test]
fn a_first_install_writes_a_bundle_json_that_names_the_installed_files() {
    let fx = Fixture::new();
    install(&fx.bundle(), &fx.layout(), &FakeLaunchd::default()).unwrap();

    let text = fs::read_to_string(fx.bin("bundle.json")).unwrap();
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let platform = humata2_machines::Platform::host().unwrap().bundle_name();
    for (name, body) in [
        ("humatad2", "daemon v1"),
        ("humata2_holder", "holder v1"),
        ("humata", "cli v1"),
        ("stack", "stack v1"),
    ] {
        assert_eq!(json["platforms"][platform][name]["sha256"], sha_hex(body.as_bytes()), "{name}");
    }
    let mode = fs::metadata(fx.bin("bundle.json")).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o644);
}

#[test]
fn a_manifest_names_the_release_for_every_platform_it_lists() {
    let fx = Fixture::new();
    let platform = humata2_machines::Platform::host().unwrap().bundle_name();
    let manifest = fx.app().join("manifest.json");
    let entry = |daemon: &str| {
        serde_json::json!({
            "humatad2": {"path": "x/humatad2", "sha256": sha_hex(daemon.as_bytes())},
            "humata2_holder": {"path": "x/humata2_holder", "sha256": sha_hex(b"holder v1")},
            "humata": {"path": "x/humata", "sha256": sha_hex(b"cli v1")},
            "stack": {"path": "x/stack", "sha256": sha_hex(b"stack v1")},
        })
    };
    let text = serde_json::json!({"platforms": {platform: entry("daemon v1"), "elsewhere-os": entry("other daemon")}});
    fs::write(&manifest, text.to_string()).unwrap();

    install(&fx.bundle().with_manifest(manifest), &fx.layout(), &FakeLaunchd::default()).unwrap();

    let installed: serde_json::Value = serde_json::from_slice(&fs::read(fx.bin("bundle.json")).unwrap()).unwrap();
    assert_eq!(installed["platforms"][platform]["humatad2"]["sha256"], sha_hex(b"daemon v1"));
}

#[test]
fn a_manifest_that_does_not_describe_the_apps_files_is_refused_and_nothing_is_installed() {
    let fx = Fixture::new();
    let platform = humata2_machines::Platform::host().unwrap().bundle_name();
    let manifest = fx.app().join("manifest.json");
    let text = serde_json::json!({"platforms": {platform: {
        "humatad2": {"path": "x", "sha256": sha_hex(b"a different daemon")},
        "humata2_holder": {"path": "x", "sha256": sha_hex(b"holder v1")},
        "humata": {"path": "x", "sha256": sha_hex(b"cli v1")},
        "stack": {"path": "x", "sha256": sha_hex(b"stack v1")},
    }}});
    fs::write(&manifest, text.to_string()).unwrap();

    let err = install(&fx.bundle().with_manifest(manifest), &fx.layout(), &FakeLaunchd::default()).unwrap_err();

    assert!(matches!(err, InstallError::Release { .. }), "{err:?}");
    assert!(!fx.bin("humatad2").exists());
}

/// The app with Linux tools: a tools folder of one platform that is not the
/// host's, and a full manifest naming the host's files and those.
fn with_machine_tools(fx: &Fixture, daemon: &str) -> (Bundle, &'static str) {
    let host = humata2_machines::Platform::host().unwrap();
    let other = humata2_machines::Platform::SHIPPED.into_iter().find(|p| *p != host).unwrap().bundle_name();
    let tools = fx.app().join("machine-tools");
    fs::create_dir_all(tools.join(other)).unwrap();
    let mut entries = serde_json::Map::new();
    for (name, body) in [("humatad2", daemon), ("humata2_holder", "holder v1"), ("humata", "cli v1"), ("stack", "stack v1")] {
        fs::write(tools.join(other).join(name), body).unwrap();
        entries.insert(name.to_owned(), serde_json::json!({"path": format!("{other}/{name}"), "sha256": sha_hex(body.as_bytes())}));
    }
    let own = |name: &str, body: &str| serde_json::json!({"path": name, "sha256": sha_hex(body.as_bytes())});
    fs::write(tools.join("manifest.json"), serde_json::json!({"platforms": {other: entries}}).to_string()).unwrap();
    let full = serde_json::json!({"platforms": {
        host.bundle_name(): {"humatad2": own("humatad2", "daemon v1"), "humata2_holder": own("h", "holder v1"),
                              "humata": own("c", "cli v1"), "stack": own("s", "stack v1")},
        other: entries,
    }});
    let manifest = fx.app().join("manifest.json");
    fs::write(&manifest, full.to_string()).unwrap();
    (fx.bundle().with_manifest(manifest).with_machine_tools(tools), other)
}

fn machine_tool(fx: &Fixture, platform: &str, name: &str) -> PathBuf {
    fx.home().join(".humata2/machine-tools").join(platform).join(name)
}

#[test]
fn install_puts_the_linux_tools_in_the_humata_home_under_the_apps_release() {
    let fx = Fixture::new();
    let (bundle, other) = with_machine_tools(&fx, "linux daemon v1");

    install(&bundle, &fx.layout(), &FakeLaunchd::default()).unwrap();

    assert_eq!(fs::read_to_string(machine_tool(&fx, other, "humatad2")).unwrap(), "linux daemon v1");
    assert!(fx.home().join(".humata2/machine-tools/manifest.json").is_file());
    // The same release the host's own bin/bundle.json names.
    assert_eq!(
        fs::read(fx.home().join(".humata2/machine-tools/bundle.json")).unwrap(),
        fs::read(fx.bin("bundle.json")).unwrap()
    );
}

#[test]
fn the_linux_tools_are_left_alone_when_they_are_already_there_and_replaced_when_they_change() {
    let fx = Fixture::new();
    let (bundle, other) = with_machine_tools(&fx, "linux daemon v1");
    assert_eq!(install_machine_tools(&bundle, &fx.layout()).unwrap(), BinaryChange::Installed);
    assert_eq!(install_machine_tools(&bundle, &fx.layout()).unwrap(), BinaryChange::Unchanged);

    let (newer, _) = with_machine_tools(&fx, "linux daemon v2");
    assert_eq!(install_machine_tools(&newer, &fx.layout()).unwrap(), BinaryChange::Updated);

    assert_eq!(fs::read_to_string(machine_tool(&fx, other, "humatad2")).unwrap(), "linux daemon v2");
    assert!(!fx.home().join(".humata2/.machine-tools.partial").exists());
    assert!(!fx.home().join(".humata2/.machine-tools.old").exists());
}

#[test]
fn a_damaged_linux_tool_is_refused_and_nothing_is_installed() {
    let fx = Fixture::new();
    let (bundle, other) = with_machine_tools(&fx, "linux daemon v1");
    fs::write(fx.app().join("machine-tools").join(other).join("stack"), "tampered").unwrap();

    let err = install_machine_tools(&bundle, &fx.layout()).unwrap_err();

    assert!(matches!(err, InstallError::MachineTools { .. }), "{err:?}");
    assert!(!fx.home().join(".humata2/machine-tools").exists());
}

#[test]
fn an_app_without_linux_tools_installs_none() {
    let fx = Fixture::new();

    assert_eq!(install_machine_tools(&fx.bundle(), &fx.layout()).unwrap(), BinaryChange::Unchanged);

    assert!(!fx.home().join(".humata2/machine-tools").exists());
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
