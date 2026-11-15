//! "Copy report" gives the summary and the absolute path of every file the
//! report holds now.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use humata2_report::{Form, PromptError, Secrets, copy_prompt, scrub, write};
use support::{Driver, add_button, draft_field, files};

fn saved(reports: &Path) -> PathBuf {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.click(add_button());
    let form = Form {
        happened: "the list showed milk twice".to_owned(),
        expected: "milk once".to_owned(),
        comments: Vec::new(),
    };
    let scrubbed = scrub(driver.stop(), form, &Secrets::new()).expect("scrubs");
    let dir = write(reports, &scrubbed).expect("writes");
    fs::canonicalize(dir).expect("the folder exists")
}

#[test]
fn the_prompt_names_every_file_by_its_absolute_path() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());

    let prompt = copy_prompt(&dir).expect("makes the prompt");

    let names = files(&dir);
    assert_eq!(names.len(), 6);
    for (name, _) in names {
        let path = dir.join(&name);
        assert!(path.is_absolute());
        assert!(prompt.contains(&path.display().to_string()), "{name} is missing:\n{prompt}");
    }
}

#[test]
fn the_prompt_carries_what_the_person_said() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());

    let prompt = copy_prompt(&dir).expect("makes the prompt");

    assert!(prompt.contains("the list showed milk twice"));
    assert!(prompt.contains("milk once"));
}

#[test]
fn a_deleted_file_isnt_listed_and_an_added_one_is() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    fs::remove_file(dir.join("daemon.log")).expect("deletes");
    fs::write(dir.join("notes.txt"), "it happened after a restart").expect("adds");

    let prompt = copy_prompt(&dir).expect("makes the prompt");

    assert!(!prompt.contains(&dir.join("daemon.log").display().to_string()));
    assert!(prompt.contains(&dir.join("notes.txt").display().to_string()));
}

#[test]
fn a_roundabout_path_still_gives_the_real_paths() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let name = dir.file_name().expect("a name");
    let parent = dir.parent().expect("a parent");
    // `parent/./name` is not the canonical path, but names the same folder.
    let roundabout = parent.join(".").join(name);

    let prompt = copy_prompt(&roundabout).expect("makes the prompt");

    assert!(prompt.contains(&dir.join("report.md").display().to_string()));
}

#[test]
fn a_folder_without_report_md_isnt_a_report() {
    let folder = tempfile::tempdir().expect("a temporary folder");
    fs::write(folder.path().join("events.jsonl"), "").expect("writes");

    assert!(matches!(copy_prompt(folder.path()), Err(PromptError::NotAReport { .. })));
}

#[test]
fn a_folder_that_isnt_there_is_an_error() {
    let folder = tempfile::tempdir().expect("a temporary folder");

    let missing = copy_prompt(&folder.path().join("gone"));

    assert!(matches!(missing, Err(PromptError::ReadFolder { .. })));
}
