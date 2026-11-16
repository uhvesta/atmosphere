//! Drags in the rendered page that run from a paragraph into the heading
//! after it map to source, so the selection toolbar (Copy as Markdown, Copy
//! as text) shows. Headless; no window is shown.
//!
//! Each drag runs twice: from the paragraph's first word, and from the
//! middle of the page, which is the blank space beside the paragraph's last
//! line. As in a browser, the second selects from that point on, through
//! the heading (gpui-base is patched for it; see markdown/patches).

use std::path::PathBuf;

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::gpui;
use gpui_kit::{AnyWindowHandle, Entity, Role, TestAppContext};
use humata2_flow::{Flow, GpuiDriver, SpecFlow, Target, artifact_root, run};
use humata2_markdown::{AnchorTarget, MarkdownOptions, SourceSelection};
use humata2_markdown_headless::{GALLERY, Pane, gallery_options, init, open};
use humata2_snapshot::snapshots;

const WIDTH: f32 = 900.;
const HEIGHT: f32 = 1400.;

fn run_flow(cx: &mut TestAppContext, window: AnyWindowHandle, flow: &Flow) {
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    let default = option_env!("CARGO_TARGET_TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let out = artifact_root(&default);
    if let Err(error) = run(flow, &mut GpuiDriver::new(cx, window), &snapshots, &out) {
        panic!("{error}");
    }
}

/// Drags in a flow of its own: tests run at once, and each flow's name
/// names its artifact and snapshots.
fn drag(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    name: &'static str,
    from: Target,
    to: Target,
) {
    let flow = Flow::new(SpecFlow::new("26.20", name)).drag(from, to);
    run_flow(cx, window, &flow);
}

/// Drags from the paragraph in `source` into the heading after it: from
/// its first word, then again from the middle of its row. Both selections
/// map to source and end with the heading, and the drag is over.
fn paragraph_then_heading(cx: &mut TestAppContext, name: &'static str, source: &str) {
    init(cx);
    let (window, pane) = open(cx, source, MarkdownOptions::new("Doc"), WIDTH, 400.);
    let heading = || Target::new(Role::Heading, "Heading 2, level 2");
    drag(cx, window, name, Target::new(Role::Paragraph, "Paragraph 1").at_start(), heading());
    let selection = assert_spans(cx, &pane, source, "## Next");
    assert_eq!(selection.target, AnchorTarget::Text);
    assert_eq!(selection.bytes.start, 0, "{selection:?}");

    drag(cx, window, name, Target::new(Role::Paragraph, "Paragraph 1"), heading());
    assert_spans(cx, &pane, source, "Next");
}

/// Checks the selection maps to the source it quotes, ends with `ends`, and
/// that no drag is left in progress.
fn assert_spans(
    cx: &mut TestAppContext,
    pane: &Entity<Pane>,
    source: &str,
    ends: &str,
) -> SourceSelection {
    let (selection, selecting) = pane.read_with(cx, |pane, cx| {
        let view = pane.view.read(cx);
        (view.selection(cx), view.is_selecting(cx))
    });
    let selection = selection.expect("the selection maps to source");
    assert!(!selecting, "the drag ended");
    assert_eq!(selection.source, source[selection.bytes.clone()]);
    assert!(selection.source.trim_end().ends_with(ends), "{selection:?}");
    selection
}

/// The gallery's paragraph 2 into heading 3, the passage flow 26.20 used
/// to comment on.
#[gpui_kit::test]
fn the_gallery_paragraph_into_the_next_heading_maps(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    drag(
        cx,
        window,
        "Select from the gallery's paragraph into a heading",
        Target::new(Role::Paragraph, "Paragraph 2").at_start(),
        Target::new(Role::Heading, "Heading 3, level 2"),
    );
    let selection = assert_spans(cx, &pane, GALLERY, "## Lists");
    // The drag starts at the left of the paragraph's second line.
    assert!(selection.quote.starts_with("to the spec."), "{selection:?}");

    // The middle of the paragraph is past the end of its second line.
    drag(
        cx,
        window,
        "Select from the gallery's paragraph into a heading",
        Target::new(Role::Paragraph, "Paragraph 2"),
        Target::new(Role::Heading, "Heading 3, level 2"),
    );
    let selection = assert_spans(cx, &pane, GALLERY, "Lists");
    assert!(!selection.quote.contains("to the spec."), "{selection:?}");
}

/// A click selects nothing, and must not leave the text counting as
/// selecting, which would keep the selection toolbar away.
#[gpui_kit::test]
fn a_click_beside_a_line_selects_nothing_and_ends(cx: &mut TestAppContext) {
    init(cx);
    let source = "A plain paragraph of words.\n\n## Next\n";
    let (window, pane) = open(cx, source, MarkdownOptions::new("Doc"), WIDTH, 400.);
    let flow = Flow::new(SpecFlow::new("26.20", "Click beside a line"))
        .click(Role::Paragraph, "Paragraph 1");
    run_flow(cx, window, &flow);
    let (selection, selecting) = pane.read_with(cx, |pane, cx| {
        let view = pane.view.read(cx);
        (view.selection(cx), view.is_selecting(cx))
    });
    assert_eq!(selection, None);
    assert!(!selecting, "the click ended");
}

#[gpui_kit::test]
fn a_plain_paragraph_into_a_heading_maps(cx: &mut TestAppContext) {
    paragraph_then_heading(
        cx,
        "Select from a plain paragraph into a heading",
        "A plain paragraph of words.\n\n## Next\n",
    );
}

#[gpui_kit::test]
fn a_paragraph_ending_in_a_link_into_a_heading_maps(cx: &mut TestAppContext) {
    paragraph_then_heading(
        cx,
        "Select from a paragraph ending in a link into a heading",
        "Read the [link to the spec](https://example.com/spec).\n\n## Next\n",
    );
}

#[gpui_kit::test]
fn a_paragraph_with_code_into_a_heading_maps(cx: &mut TestAppContext) {
    paragraph_then_heading(
        cx,
        "Select from a paragraph with code into a heading",
        "Some `inline code` in a paragraph.\n\n## Next\n",
    );
}

#[gpui_kit::test]
fn a_paragraph_over_two_lines_into_a_heading_maps(cx: &mut TestAppContext) {
    paragraph_then_heading(
        cx,
        "Select from a paragraph over two lines into a heading",
        "The first line of the paragraph,\nand the second one.\n\n## Next\n",
    );
}
