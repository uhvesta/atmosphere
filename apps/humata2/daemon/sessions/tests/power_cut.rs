//! Cuts the power right after a channel's folders and its clones are made
//! (PER-25): what was made must still be there, whole, because every
//! folder and rename was put on disk before the call returned.
//!
//! The worker is this test binary run again with `power_cut_child`
//! selected; that test does nothing otherwise. Git writes its files
//! outside the simulation, so what this shows is the folders and the
//! renames into place, not git's own bytes.

use std::fs;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;

use humata2_sessions::{Layout, clone_main, clone_session, partial};

const HOME: &str = "HUMATA2_SESSIONS_HOME";
const SOURCE: &str = "HUMATA2_SESSIONS_SOURCE";

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// The worker: makes a channel, its main clone and a session clone, then
/// is cut with nothing else synced.
#[test]
fn power_cut_child() {
    let Ok(home) = std::env::var(HOME) else {
        return;
    };
    let source = std::env::var(SOURCE).unwrap();
    let layout = Layout::new(home);
    layout.create_channel("ch").unwrap();
    let main = layout.main_clone("ch", "api");
    clone_main(&source, &main).unwrap();
    clone_session(&main, &source, &layout.session("ch", "s1"), "feature").unwrap();
    humata2_durable::cut("after the clones");
}

#[test]
fn a_channel_and_its_clones_survive_a_power_cut() {
    let dir = tempfile::Builder::new().prefix("h2pc").tempdir_in("/tmp").unwrap();
    let source = dir.path().join("source");
    fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    fs::write(source.join("f"), "x").unwrap();
    git(&source, &["add", "f"]);
    git(&source, &["commit", "-m", "one"]);
    let home = dir.path().join("home");

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["power_cut_child", "--exact", "--nocapture", "--test-threads=1"])
        .env(HOME, &home)
        .env(SOURCE, &source)
        .env("HUMATA2_POWER_CUT_ROOTS", &home)
        .env("HUMATA2_POWER_CUT_SPARE", dir.path().join("spare"))
        .env_remove("TESTBRIDGE_TEST_ONLY")
        .status()
        .unwrap();
    assert_eq!(status.signal(), Some(9), "the worker didn't stop at the cut: {status}");

    let layout = Layout::new(home);
    for folder in [layout.main("ch"), layout.sessions("ch"), layout.shared("ch")] {
        assert!(folder.is_dir(), "{folder:?} was lost");
    }
    for clone in [layout.main_clone("ch", "api"), layout.session("ch", "s1")] {
        assert!(clone.join(".git/HEAD").is_file(), "no clone at {clone:?}");
        assert_eq!(fs::read_to_string(clone.join("f")).unwrap(), "x");
        assert!(!partial(&clone).exists(), "{clone:?} was left half made");
    }
}
