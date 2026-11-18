//! The gallery's pixels as a headless Metal render draws them, plain and
//! annotated, and a streamed gallery against a one-shot render. macOS only.
//! These run in their own test binary: gpui-component resolves its mono font
//! once per process, and a font-less test context first would fix it to the
//! system UI font (D747).
//!
//! Record with `bazel run //apps/humata2/markdown/headless:pixels_record`
//! and read every file it writes in `tests/__snapshots__/pixels/`.

#![cfg(target_os = "macos")]

use gpui_kit::prelude::*;
use gpui_kit::Context;
use gpui_kit::test::TestWindowExt as _;
use humata2_markdown::MarkdownView;
use humata2_markdown_headless::{GALLERY, Pane, annotate, gallery_options};
use humata2_snapshot::{layout, snapshots};

const WIDTH: f32 = 900.;
const HEIGHT: f32 = 1400.;

/// Opens the gallery in a headless Metal context and returns what it drew.
fn render(source: &str, prepare: impl FnOnce(&mut MarkdownView, &mut Context<MarkdownView>)) -> (humata2_snapshot::RgbaImage, humata2_snapshot::layout::Layout) {
    use humata2_markdown_headless::window_options;

    let mut cx = humata2_snapshot::headless_context();
    cx.update(|cx| cx.bind_keys(humata2_markdown::key_bindings()));
    let source = source.to_owned();
    let (window, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(window_options(WIDTH, HEIGHT), cx, |_, cx| {
                cx.new(|cx| Pane::new(&source, gallery_options(), cx))
            })
        })
        .expect("the headless window opens");
    cx.run_until_parked();
    let view = cx.update(|cx| pane.read(cx).view.clone());
    cx.update(|cx| view.update(cx, prepare));
    for _ in 0..2 {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| window.render_frame(cx))
            .expect("the window is open");
    }
    cx.run_until_parked();
    let (image, layout) = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            (window.render_to_image(), layout::capture(window))
        })
        .expect("the window is open");
    (image.expect("Metal draws the window off screen"), layout)
}

/// SM-3
#[::std::prelude::v1::test]
fn the_gallery_draws_as_recorded() {
    let (image, layout) = render(GALLERY, |_, _| {});
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    snapshots.assert_image("gallery", &image, humata2_snapshot::Tolerance::default());
    snapshots.assert_layout("gallery", &layout);
}

/// SM-3
#[::std::prelude::v1::test]
fn a_streamed_gallery_draws_the_same_pixels_as_a_one_shot_render() {
    let (whole, _) = render(GALLERY, |_, _| {});
    let (streamed, _) = render("", |view, cx| {
        let mut at = 0;
        while at < GALLERY.len() {
            let mut end = (at + 31).min(GALLERY.len());
            while !GALLERY.is_char_boundary(end) {
                end += 1;
            }
            view.append(&GALLERY[at..end], cx);
            at = end;
        }
    });
    assert_eq!(streamed.dimensions(), whole.dimensions());
    assert!(streamed.as_raw() == whole.as_raw(), "the streamed gallery's pixels differ");
}

/// SM-5, SM-8
#[::std::prelude::v1::test]
fn the_annotated_gallery_draws_as_recorded() {
    let (image, layout) = render(GALLERY, annotate);
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    snapshots.assert_image("gallery-annotated", &image, humata2_snapshot::Tolerance::default());
    snapshots.assert_layout("gallery-annotated", &layout);
}
