//! A report is saved as `reports/<date>-<slug>/` with its six files,
//! readable only by this user, and never half-written.

mod support;

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;

use humata2_report::{Form, Secrets, scrub, snapshot, write};
use support::{Driver, add_button, context, draft_field, files};

const FILES: [&str; 6] = [
    "report.md",
    "events.jsonl",
    "state.txt",
    "view.txt",
    "layout.txt",
    "daemon.log",
];

fn report(happened: &str) -> humata2_report::Scrubbed {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.click(add_button());
    let form = Form {
        happened: happened.to_owned(),
        ..Form::default()
    };
    scrub(driver.stop(), form, &Secrets::new()).expect("scrubs")
}

fn mode(path: &Path) -> u32 {
    fs::metadata(path).expect("exists").permissions().mode() & 0o777
}

#[test]
fn a_report_is_a_dated_folder_of_six_files_only_its_owner_can_read() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let reports = home.path().join("reports");

    let dir = write(&reports, &report("The Add button added twice")).expect("writes");

    assert_eq!(dir.parent(), Some(reports.as_path()));
    let name = dir.file_name().and_then(|name| name.to_str()).expect("a name");
    assert!(name.starts_with("2026-11-14-"), "{name}");
    let mut names: Vec<String> = files(&dir).into_iter().map(|(name, _)| name).collect();
    names.sort();
    let mut expected: Vec<String> = FILES.iter().map(|name| (*name).to_owned()).collect();
    expected.sort();
    assert_eq!(names, expected);
    assert_eq!(mode(&reports), 0o700);
    assert_eq!(mode(&dir), 0o700);
    for file in FILES {
        assert_eq!(mode(&dir.join(file)), 0o600, "{file}");
        assert!(!fs::read_to_string(dir.join(file)).expect("text").is_empty(), "{file}");
    }
}

#[test]
fn a_snapshot_has_the_same_six_files() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let scrubbed = scrub(
        snapshot(context(), support::Toy::default().trees()),
        Form::default(),
        &Secrets::new(),
    )
    .expect("scrubs");

    let dir = write(reports.path(), &scrubbed).expect("writes");

    assert_eq!(files(&dir).len(), FILES.len());
}

#[test]
fn nothing_partial_is_left_behind() {
    let reports = tempfile::tempdir().expect("a temporary folder");

    let dir = write(reports.path(), &report("lost an item")).expect("writes");

    let entries: Vec<_> = fs::read_dir(reports.path())
        .expect("reads")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    assert_eq!(entries, vec![dir]);
}

#[test]
fn the_same_report_twice_gets_two_folders() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let scrubbed = report("lost an item");

    let first = write(reports.path(), &scrubbed).expect("writes");
    let second = write(reports.path(), &scrubbed).expect("writes again");

    assert_ne!(first, second);
    assert_eq!(files(&first), files(&second));
}

#[test]
fn a_description_with_slashes_stays_inside_the_reports_folder() {
    let reports = tempfile::tempdir().expect("a temporary folder");

    let dir = write(reports.path(), &report("../../etc/passwd broke")).expect("writes");

    assert_eq!(dir.parent(), Some(reports.path()));
}

#[test]
fn a_reports_folder_that_cant_be_made_is_an_error() {
    let home = tempfile::tempdir().expect("a temporary folder");
    let blocker = home.path().join("reports");
    fs::write(&blocker, "a file where the folder should be").expect("writes the blocker");

    let failed = write(&blocker, &report("lost an item"));

    assert!(matches!(failed, Err(humata2_report::WriteError::CreateReports { .. })));
}
