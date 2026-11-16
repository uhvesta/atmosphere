//! The window's picture and the marks over it: masking, `marks.svg`, and
//! what the report and the prompt say about them.

mod support;

use std::fs;
use std::path::{Path, PathBuf};

use humata2_report::{
    Bounds, Comment, Drawing, Form, PixelBox, Point, Secrets, Shape, Size, Stroke, copy_prompt,
    fill_form, mask, scrub, write, write_window,
};
use humata2_report::video::Video;
use image::{Rgba, RgbaImage};
use support::{Driver, add_button, draft_field, item};

const SECRET: &str = "ghp_Zx81Qm2Lp0Vb7Rt4";

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

fn stroke(shape: Shape, points: &[(i32, i32)]) -> Stroke {
    Stroke {
        shape,
        points: points.iter().map(|&(x, y)| Point { x, y }).collect(),
        covers: vec![add_button()],
        more: 0,
    }
}

fn marked(strokes: Vec<Stroke>, comments: Vec<Comment>) -> Form {
    Form {
        happened: "the list showed milk twice".to_owned(),
        comments,
        drawing: Drawing {
            window: Size {
                width: 800,
                height: 600,
            },
            strokes,
        },
        ..Form::default()
    }
}

fn white(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_pixel(width, height, Rgba([255, 255, 255, 255]))
}

#[test]
fn a_mask_fills_its_box_and_leaves_the_rest() {
    let mut image = white(10, 10);

    mask(
        &mut image,
        &[PixelBox {
            x: 2,
            y: 3,
            width: 4,
            height: 2,
        }],
    );

    let covered = |x, y| (2..6).contains(&x) && (3..5).contains(&y);
    for (x, y, pixel) in image.enumerate_pixels() {
        if covered(x, y) {
            assert_eq!(pixel[3], 255, "({x}, {y}) is opaque");
            assert_ne!(*pixel, Rgba([255, 255, 255, 255]), "({x}, {y}) is covered");
        } else {
            assert_eq!(*pixel, Rgba([255, 255, 255, 255]), "({x}, {y}) is untouched");
        }
    }
}

#[test]
fn a_mask_partly_or_wholly_outside_the_picture_is_cut_off() {
    let mut image = white(4, 4);

    mask(
        &mut image,
        &[
            PixelBox {
                x: -5,
                y: -5,
                width: 7,
                height: 7,
            },
            PixelBox {
                x: 100,
                y: 100,
                width: 5,
                height: 5,
            },
        ],
    );

    assert_ne!(*image.get_pixel(0, 0), Rgba([255, 255, 255, 255]));
    assert_ne!(*image.get_pixel(1, 1), Rgba([255, 255, 255, 255]));
    assert_eq!(*image.get_pixel(3, 3), Rgba([255, 255, 255, 255]));
}

#[test]
fn a_folder_with_a_picture_and_strokes_has_an_overlay_over_it() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    write_window(&dir, &white(16, 12)).expect("saves the picture");
    let strokes = vec![
        stroke(Shape::Loop, &[(10, 10), (60, 10), (60, 40), (10, 40)]),
        stroke(Shape::Line, &[(100, 100), (200, 120)]),
        stroke(Shape::Dot, &[(300, 300)]),
    ];

    fill_form(&dir, marked(strokes, Vec::new()), &Secrets::new()).expect("fills in");

    let svg = fs::read_to_string(dir.join("marks.svg")).expect("the overlay");
    assert!(svg.contains("href=\"window.png\""), "{svg}");
    assert!(svg.contains("viewBox=\"0 0 800 600\""), "{svg}");
    assert_eq!(svg.matches("<polyline").count(), 2, "{svg}");
    assert!(svg.contains("#ff453a"), "{svg}");
    assert!(dir.join("window.png").is_file());
}

#[test]
fn an_overlay_without_a_picture_does_not_point_at_one() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());

    let strokes = vec![stroke(Shape::Line, &[(1, 1), (5, 5)])];
    fill_form(&dir, marked(strokes, Vec::new()), &Secrets::new()).expect("fills in");

    let svg = fs::read_to_string(dir.join("marks.svg")).expect("the overlay");
    assert!(!svg.contains("<image"), "{svg}");
}

#[test]
fn a_secret_in_a_comment_is_not_in_the_overlay_and_its_text_is_escaped() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let comment = Comment {
        element: item("milk"),
        text: format!("<b>{SECRET}</b> & more"),
        place: Some(Bounds {
            x: 20,
            y: 30,
            width: 100,
            height: 24,
        }),
    };
    let secrets = Secrets::new().secret("GITHUB_TOKEN", SECRET);

    fill_form(&dir, marked(Vec::new(), vec![comment]), &secrets).expect("fills in");

    let svg = fs::read_to_string(dir.join("marks.svg")).expect("the overlay");
    assert!(!svg.contains(SECRET), "{svg}");
    assert!(!svg.contains("<b>"), "{svg}");
    assert!(svg.contains("&amp;"), "{svg}");
    assert!(svg.contains("<rect"), "{svg}");
}

#[test]
fn removing_the_strokes_and_comments_removes_the_overlay() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let strokes = vec![stroke(Shape::Line, &[(1, 1), (5, 5)])];
    fill_form(&dir, marked(strokes, Vec::new()), &Secrets::new()).expect("fills in");
    assert!(dir.join("marks.svg").exists());

    fill_form(&dir, marked(Vec::new(), Vec::new()), &Secrets::new()).expect("fills in again");

    assert!(!dir.join("marks.svg").exists());
}

#[test]
fn a_comment_without_a_place_is_not_drawn() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let comment = Comment {
        element: item("milk"),
        text: "shown twice".to_owned(),
        place: None,
    };

    fill_form(&dir, marked(Vec::new(), vec![comment]), &Secrets::new()).expect("fills in");

    assert!(!dir.join("marks.svg").exists());
}

#[test]
fn the_prompt_and_the_summary_name_the_pictures_when_present() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    write_window(&dir, &white(16, 12)).expect("saves the picture");
    let strokes = vec![stroke(Shape::Line, &[(1, 1), (5, 5)])];
    fill_form(&dir, marked(strokes, Vec::new()), &Secrets::new()).expect("fills in");

    let prompt = copy_prompt(&dir).expect("makes the prompt");
    let summary = fs::read_to_string(dir.join("report.md")).expect("the summary");

    for text in [&prompt, &summary] {
        assert!(text.contains("window.png"), "{text}");
        assert!(text.contains("marks.svg"), "{text}");
    }
}

#[test]
fn a_recordings_video_is_named_with_what_it_holds_in_the_summary_and_the_prompt() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let mut video = Video::new(1);
    video.push(0, 8, 6, white(8, 6).as_raw()).expect("keeps the frame");
    video.write(&dir).expect("writes the video").expect("there is a video");
    fill_form(&dir, marked(Vec::new(), Vec::new()), &Secrets::new()).expect("fills in");

    let prompt = copy_prompt(&dir).expect("makes the prompt");
    let summary = fs::read_to_string(dir.join("report.md")).expect("the summary");

    for file in ["video.png", "video.jsonl"] {
        assert!(summary.contains(&format!("- `{file}`: ")), "{summary}");
        let folder = dir.canonicalize().expect("the folder exists");
        let listed = format!("- {}: ", folder.join(file).display());
        assert!(prompt.contains(&listed), "{prompt}");
    }
}

#[test]
fn a_folder_without_pictures_does_not_mention_them() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    fill_form(&dir, marked(Vec::new(), Vec::new()), &Secrets::new()).expect("fills in");

    let prompt = copy_prompt(&dir).expect("makes the prompt");
    let summary = fs::read_to_string(dir.join("report.md")).expect("the summary");

    for text in [&prompt, &summary] {
        assert!(!text.contains("window.png"), "{text}");
        assert!(!text.contains("marks.svg"), "{text}");
    }
}

#[test]
fn the_saved_picture_decodes_to_what_was_given() {
    let reports = tempfile::tempdir().expect("a temporary folder");
    let dir = saved(reports.path());
    let mut picture = white(8, 6);
    picture.put_pixel(1, 1, Rgba([1, 2, 3, 255]));

    write_window(&dir, &picture).expect("saves the picture");

    let back = image::open(dir.join("window.png")).expect("decodes").to_rgba8();
    assert_eq!(back, picture);
}
