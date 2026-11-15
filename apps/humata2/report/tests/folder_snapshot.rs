//! A saved report folder, every file and its mode, checked against a
//! recorded snapshot for each kind of report, so any change to what a
//! report holds shows up as a diff to review.
//!
//! The snapshots live in `tests/__snapshots__/folder_snapshot/`. To record
//! them after a change on purpose, run
//! `HUMATA2_SNAPSHOT=record bazel run //apps/humata2/report:folder_snapshot_test`
//! (or `HUMATA2_SNAPSHOT=record cargo test -p humata2-report --test
//! folder_snapshot`), then read `git diff` of the folder before committing.

mod support;

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use humata2_report::{
    Captured, Comment, FlowAction, Form, Holds, LogLine, Rpc, Secrets, scrub, snapshot, write,
};
use support::{Driver, Toy, add_button, context, draft_field, item};

const VARIABLE: &str = "HUMATA2_SNAPSHOT";
const PACKAGE: &str = "apps/humata2/report";
const SNAPSHOTS: &str = "tests/__snapshots__/folder_snapshot";

/// The folder as one text: its name, then each file, by name, with its
/// mode and text.
fn folder_text(dir: &Path) -> String {
    let name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut text = format!("{name}/\n");
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .expect("the report folder can be read")
        .map(|entry| entry.expect("a folder entry").path())
        .collect();
    entries.sort();
    for path in entries {
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned();
        let mode = fs::metadata(&path).expect("metadata").permissions().mode() & 0o777;
        let contents = fs::read_to_string(&path).expect("report files are text");
        text.push_str(&format!("\n--- {file} ({mode:o})\n{contents}"));
    }
    text
}

/// Where the snapshot `name` is read from and recorded to: the source
/// tree under `bazel run` and cargo, the runfiles under `bazel test`.
fn snapshot_paths(name: &str) -> (Vec<PathBuf>, PathBuf) {
    let file = format!("{SNAPSHOTS}/{name}.txt");
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join(&file);
    let workspace = env::var_os("BUILD_WORKSPACE_DIRECTORY")
        .map(|root| PathBuf::from(root).join(PACKAGE).join(&file));
    let runfiles = PathBuf::from(PACKAGE).join(&file);
    let record = workspace.clone().unwrap_or_else(|| manifest.clone());
    let read = workspace.into_iter().chain([runfiles, manifest]).collect();
    (read, record)
}

fn check(name: &str, actual: &str) {
    let (read, record) = snapshot_paths(name);
    match env::var(VARIABLE).as_deref() {
        Ok("record") => {
            let parent = record.parent().expect("a snapshot folder");
            fs::create_dir_all(parent).expect("makes the snapshot folder");
            fs::write(&record, actual).expect("records the snapshot");
        }
        Ok("verify") | Err(env::VarError::NotPresent) => {
            let Some(expected) = read.iter().find_map(|path| fs::read_to_string(path).ok()) else {
                panic!(
                    "there is no snapshot {name} yet (looked in {read:?}). The test produced:\n\
                     {actual}\nIf that is right, record it with {VARIABLE}=record bazel run \
                     //{PACKAGE}:folder_snapshot_test and commit {SNAPSHOTS}/{name}.txt"
                );
            };
            if expected != actual {
                let differs = expected
                    .lines()
                    .zip(actual.lines())
                    .position(|(was, now)| was != now)
                    .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()));
                panic!(
                    "the {name} report folder differs from its snapshot from line {} on. The \
                     test produced:\n{actual}\nIf the change is on purpose, record it with \
                     {VARIABLE}=record bazel run //{PACKAGE}:folder_snapshot_test and read the \
                     git diff of {SNAPSHOTS}/{name}.txt before committing; otherwise a report \
                     now holds something it shouldn't, or lost something it should hold",
                    differs + 1
                );
            }
        }
        Ok(other) => panic!("{VARIABLE} is {other:?}; set it to `record` or `verify`, or unset it"),
        Err(env::VarError::NotUnicode(_)) => {
            panic!("{VARIABLE} is not UTF-8; set it to `record` or `verify`, or unset it")
        }
    }
}

fn saved_text(captured: Captured, form: Form, secrets: &Secrets) -> String {
    let scrubbed = scrub(captured, form, secrets).expect("scrubs");
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = write(reports.path(), &scrubbed).expect("writes");
    folder_text(&dir)
}

#[test]
fn a_saved_recording_holds_what_its_snapshot_holds() {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.click(add_button());
    driver.recording.rpc(Rpc {
        at_ms: 205,
        machine: "studio".to_owned(),
        method: "humata.v1.SessionService/List".to_owned(),
        status: "ok".to_owned(),
        duration_ms: 3,
        request: Some("{}".to_owned()),
        response: Some("{\"sessions\":[\"s-1\"]}".to_owned()),
        holds: Holds::Plain,
    });
    driver.recording.log(LogLine {
        at_ms: 206,
        machine: "studio".to_owned(),
        line: "token ghp_Zx81Qm2Lp0Vb7Rt4 refreshed".to_owned(),
    });
    let form = Form {
        happened: "the list showed milk twice".to_owned(),
        expected: "milk once".to_owned(),
        comments: vec![Comment {
            element: item("milk"),
            text: "shown twice".to_owned(),
        }],
    };
    let secrets = Secrets::new().secret("GITHUB_TOKEN", "ghp_Zx81Qm2Lp0Vb7Rt4");

    check("recording", &saved_text(driver.stop(), form, &secrets));
}

#[test]
fn a_saved_snapshot_holds_what_its_snapshot_holds() {
    let mut toy = Toy::default();
    toy.play(&FlowAction::Type("milk".to_owned()));
    let form = Form {
        happened: "the draft kept milk after a restart".to_owned(),
        expected: "an empty draft".to_owned(),
        comments: Vec::new(),
    };

    check("snapshot", &saved_text(snapshot(context(), toy.trees()), form, &Secrets::new()));
}
