//! Marks drawn on the window are saved as `drawing.json`, named in
//! `report.md` and in the copied prompt, and scrubbed like comments.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use humata2_report::{
    Comment, Drawing, Form, Point, Secrets, Shape, Size, Stroke, copy_prompt, fill_form, scrub, write,
};
use support::{Driver, add_button, assert_no_piece_of, draft_field, item};

fn loop_around_add() -> Stroke {
    Stroke {
        shape: Shape::Loop,
        points: vec![
            Point { x: 600, y: 0 },
            Point { x: 700, y: 0 },
            Point { x: 700, y: 40 },
            Point { x: 600, y: 40 },
            Point { x: 602, y: 2 },
        ],
        covers: vec![add_button()],
        more: 0,
    }
}

fn drawing(strokes: Vec<Stroke>) -> Drawing {
    Drawing {
        window: Size {
            width: 800,
            height: 600,
        },
        strokes,
    }
}

fn saved(reports: &Path) -> PathBuf {
    let mut driver = Driver::start();
    driver.type_into(draft_field(), "milk");
    driver.click(add_button());
    let form = Form {
        happened: "the list showed milk twice".to_owned(),
        ..Form::default()
    };
    let scrubbed = scrub(driver.stop(), form, &Secrets::new()).expect("scrubs");
    write(reports, &scrubbed).expect("writes")
}

fn form_with(strokes: Vec<Stroke>) -> Form {
    Form {
        happened: "the list showed milk twice".to_owned(),
        drawing: drawing(strokes),
        ..Form::default()
    }
}

#[test]
fn a_loop_is_saved_as_drawing_json_and_named_in_the_prompt() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let marks = drawing(vec![loop_around_add()]);

    fill_form(&dir, form_with(marks.strokes.clone()), &Secrets::new()).expect("fills in the form");

    let text = fs::read_to_string(dir.join("drawing.json")).expect("drawing.json");
    let back: Drawing = serde_json::from_str(&text).expect("parses back");
    assert_eq!(back, marks);
    let prompt = copy_prompt(&dir).expect("makes the prompt");
    assert!(prompt.contains("Add"), "{prompt}");
    assert!(prompt.contains("a circle around"), "{prompt}");
    assert!(prompt.contains(&dir.canonicalize().expect("exists").join("drawing.json").display().to_string()));
    let report = fs::read_to_string(dir.join("report.md")).expect("report.md");
    assert!(report.contains("## Drawings"), "{report}");
}

#[test]
fn removing_every_stroke_removes_drawing_json() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    fill_form(&dir, form_with(vec![loop_around_add()]), &Secrets::new()).expect("fills in");
    assert!(dir.join("drawing.json").exists());

    fill_form(&dir, form_with(Vec::new()), &Secrets::new()).expect("fills in again");

    assert!(!dir.join("drawing.json").exists());
    let report = fs::read_to_string(dir.join("report.md")).expect("report.md");
    assert!(!report.contains("Drawings"), "{report}");
}

#[test]
fn a_folder_without_strokes_has_no_drawing_and_the_same_report() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let before = fs::read_to_string(dir.join("report.md")).expect("report.md");

    fill_form(&dir, form_with(Vec::new()), &Secrets::new()).expect("fills in");

    assert!(!dir.join("drawing.json").exists());
    let after = fs::read_to_string(dir.join("report.md")).expect("report.md");
    assert_eq!(before, after);
}

#[test]
fn a_stroke_written_with_the_report_is_in_the_folder_at_once() {
    let mut driver = Driver::start();
    driver.click(add_button());
    let scrubbed = scrub(driver.stop(), form_with(vec![loop_around_add()]), &Secrets::new()).expect("scrubs");
    let reports = tempfile::tempdir().expect("a temporary folder");

    let dir = write(reports.path(), &scrubbed).expect("writes");

    assert!(dir.join("drawing.json").exists());
}

#[test]
fn comments_are_in_the_copied_prompt() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let form = Form {
        comments: vec![Comment {
            element: item("milk"),
            text: "this row is a duplicate".to_owned(),
            place: None,
        }],
        ..form_with(Vec::new())
    };
    fill_form(&dir, form, &Secrets::new()).expect("fills in");

    let prompt = copy_prompt(&dir).expect("makes the prompt");

    assert!(prompt.contains("this row is a duplicate"), "{prompt}");
}

#[test]
fn a_secret_in_a_stroke_element_is_scrubbed_everywhere() {
    let token = "ghp_Zx81Qm2Lp0Vb7Rt4";
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let mut stroke = loop_around_add();
    stroke.covers = vec![item(&format!("token {token}"))];
    let secrets = Secrets::new().secret("GITHUB_TOKEN", token);

    fill_form(&dir, form_with(vec![stroke]), &secrets).expect("fills in");

    assert!(dir.join("drawing.json").exists());
    assert_no_piece_of(&dir, token, 6);
}

#[test]
fn a_stroke_is_described_in_one_line() {
    let stroke = Stroke {
        shape: Shape::Line,
        points: vec![Point { x: 1, y: 2 }, Point { x: 50, y: 2 }],
        covers: vec![add_button(), draft_field()],
        more: 3,
    };
    let line = stroke.describe();
    assert!(line.starts_with("a line across"), "{line}");
    assert!(line.contains("Add") && line.contains("New item"), "{line}");
    assert!(line.ends_with("and 3 more"), "{line}");

    let bare = Stroke {
        shape: Shape::Loop,
        points: vec![Point { x: 9, y: 4 }],
        covers: Vec::new(),
        more: 0,
    };
    assert!(bare.describe().contains("(9, 4)"));
}
