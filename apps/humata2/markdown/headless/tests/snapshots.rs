//! Snapshots of the gallery: its view tree (roles, labels and text), its
//! layout, and on macOS the pixels a headless Metal render draws. Once
//! plain, once in Source mode, and once annotated with comments (one on a
//! passage, one on the table, one on the diagram, one whose passage is gone)
//! over a diff against an older gallery. Everything runs headless; no window
//! is shown.
//!
//! Record with `bazel run //apps/humata2/markdown/headless:snapshots_record`
//! and read every file it writes in `tests/__snapshots__/snapshots/`.

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::gpui;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Context, TestAppContext};
use humata2_markdown::{Anchor, Comment, Document, MarkdownView, ViewMode};
use humata2_markdown_headless::{GALLERY, Pane, gallery_options, init, open};
use humata2_snapshot::{layout, snapshots, view};

const WIDTH: f32 = 900.;
const HEIGHT: f32 = 1400.;

/// An older gallery: the third list item and the last paragraph's wording
/// came later, and a closing paragraph went away.
fn older_gallery() -> String {
    format!(
        "{}\nA closing paragraph that went away.\n",
        GALLERY.replace("- Third item\n", "").replace("The end.", "The finish.")
    )
}

/// Comments on a passage, the table and the diagram, one whose passage is
/// gone, and a diff against [`older_gallery`].
fn annotate(view: &mut MarkdownView, cx: &mut Context<MarkdownView>) {
    let document = view.document().clone();
    let passage = "**strong**, *emphasis*";
    let start = GALLERY.find(passage).expect("the gallery has the passage");
    let on = |id: &str, selection: humata2_markdown::SourceSelection, body: &str| Comment {
        id: id.into(),
        anchor: Anchor::new(&document, &selection),
        body: body.into(),
    };
    let text = document
        .selection(start..start + passage.len(), "strong, emphasis")
        .expect("the passage is in the gallery");
    let table = document.block_selection(10).expect("block 11 is the table");
    let diagram = document.block_selection(15).expect("block 16 is the diagram");
    let old = Document::new("A passage that is long gone from the gallery.\n");
    let gone = old.selection(2..9, "passage").expect("the old text has it");
    let comments = vec![
        on("c1", text, "Say which styles the parser keeps."),
        on("c2", table, "Add a row for diagrams."),
        on("c3", diagram, "Label the arrow."),
        Comment {
            id: "c4".into(),
            anchor: Anchor::new(&old, &gone),
            body: "This passage was removed.".into(),
        },
    ];
    view.set_comments(comments, cx);
    view.set_diff_base(Some(&older_gallery()), cx);
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) -> (String, humata2_snapshot::layout::Layout) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let tree = view::capture(window).unwrap_or_else(|error| panic!("{error}"));
        (tree, layout::capture(window))
    })
    .expect("the window is open")
}

fn check(name: &str, tree: &str, layout: &humata2_snapshot::layout::Layout) {
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    snapshots.assert_text(name, tree);
    snapshots.assert_layout(name, layout);
}

/// SM-3
#[gpui_kit::test]
fn the_gallery_draws_every_kind_of_block(cx: &mut TestAppContext) {
    init(cx);
    let (window, _) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    let (tree, layout) = draw(cx, window);
    check("gallery", &tree, &layout);
}

/// SM-4
#[gpui_kit::test]
fn the_gallery_in_source_mode_shows_its_markdown(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    let view = pane.read_with(cx, |pane, _| pane.view.clone());
    view.update(cx, |view, cx| view.set_mode(ViewMode::Source, cx));
    let (tree, layout) = draw(cx, window);
    check("gallery-source", &tree, &layout);
}

/// SM-5, SM-7, SM-8
#[gpui_kit::test]
fn the_annotated_gallery_shows_comments_in_the_margin_and_changes_in_the_gutter(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    let view = pane.read_with(cx, |pane, _| pane.view.clone());
    view.update(cx, annotate);
    let (tree, layout) = draw(cx, window);
    check("gallery-annotated", &tree, &layout);
}

/// Opens the gallery in a headless Metal context and returns what it drew.
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
#[::std::prelude::v1::test]
fn the_gallery_draws_as_recorded() {
    let (image, layout) = render(GALLERY, |_, _| {});
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    snapshots.assert_image("gallery", &image, humata2_snapshot::Tolerance::default());
    snapshots.assert_layout("gallery", &layout);
}

/// SM-3
#[cfg(target_os = "macos")]
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
#[cfg(target_os = "macos")]
#[::std::prelude::v1::test]
fn the_annotated_gallery_draws_as_recorded() {
    let (image, layout) = render(GALLERY, annotate);
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    snapshots.assert_image("gallery-annotated", &image, humata2_snapshot::Tolerance::default());
    snapshots.assert_layout("gallery-annotated", &layout);
}
