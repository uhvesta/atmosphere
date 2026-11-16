//! What the Markdown view draws, checked through the window it draws in:
//! streaming against a one-shot render, diagrams that break and recover, a
//! shared diagram cache, comments that go stale, diff marks, images and long
//! documents. Everything runs headless; no window is shown.

// `#[gpui_kit::test]` expands to paths under `gpui`.
use gpui_kit::gpui;
use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, Role, TestAppContext};
use humata2_markdown::{
    Anchor, AnchorStatus, Comment, CommentMarks, Document, MarkdownEvent, MarkdownOptions, MarkdownView, MermaidCache,
};
use humata2_markdown_headless::{GALLERY, Pane, gallery_options, init, open};
use humata2_snapshot::{layout, view};

const WIDTH: f32 = 900.;
const HEIGHT: f32 = 1400.;

fn frame(cx: &mut TestAppContext, window: AnyWindowHandle) {
    cx.run_until_parked();
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.render_frame(cx);
    })
    .expect("the window is open");
    cx.run_until_parked();
}

fn markdown(cx: &mut TestAppContext, pane: &Entity<Pane>) -> Entity<MarkdownView> {
    pane.read_with(cx, |pane, _| pane.view.clone())
}

fn update(cx: &mut TestAppContext, pane: &Entity<Pane>, f: impl FnOnce(&mut MarkdownView, &mut gpui_kit::Context<MarkdownView>)) {
    let view = markdown(cx, pane);
    view.update(cx, f);
}

fn role(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) -> Option<Role> {
    let id = id.into();
    cx.update_window(window, |_, window, _| window.try_find(id).and_then(|found| found.role()))
        .expect("the window is open")
}

fn present(cx: &mut TestAppContext, window: AnyWindowHandle, id: impl Into<gpui_kit::ElementId>) -> bool {
    let id = id.into();
    cx.update_window(window, |_, window, _| window.try_find(id).is_some())
        .expect("the window is open")
}

fn captured(cx: &mut TestAppContext, window: AnyWindowHandle) -> (String, String) {
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let tree = view::capture(window).unwrap_or_else(|error| panic!("{error}"));
        (tree, layout::capture(window).text().to_owned())
    })
    .expect("the window is open")
}

/// The gallery split into pieces of `size` bytes, on character boundaries.
fn chunks(source: &str, size: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut at = 0;
    while at < source.len() {
        let mut end = (at + size).min(source.len());
        while !source.is_char_boundary(end) {
            end += 1;
        }
        pieces.push(&source[at..end]);
        at = end;
    }
    pieces
}

/// SM-3
#[gpui_kit::test]
fn a_streamed_document_ends_with_the_same_view_and_layout_as_a_one_shot_render(cx: &mut TestAppContext) {
    init(cx);
    let (whole, _) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    frame(cx, whole);
    let expected = captured(cx, whole);

    let (streamed, pane) = open(cx, "", gallery_options(), WIDTH, HEIGHT);
    for piece in chunks(GALLERY, 23) {
        update(cx, &pane, |view, cx| view.append(piece, cx));
        frame(cx, streamed);
    }
    frame(cx, streamed);
    let actual = captured(cx, streamed);
    assert_eq!(actual.0, expected.0, "the streamed view tree differs");
    assert_eq!(actual.1, expected.1, "the streamed layout differs");
    let stats = markdown(cx, &pane).read_with(cx, |view, _| view.stats());
    assert!(!stats.parse.full, "the last append reparsed only the tail: {stats:?}");
}

const BROKEN: &str = "# Diagram\n\n```mermaid\nflowchart LR\n  subgraph one\n  A --> B\n```\n";
const FIXED: &str = "# Diagram\n\n```mermaid\nflowchart LR\n  subgraph one\n  A --> B\n  end\n```\n";

fn diagram_id(source: &str) -> (&'static str, usize) {
    let start = source.find("```mermaid").expect("the source has a diagram");
    ("md-diagram", start)
}

/// SM-3
#[gpui_kit::test]
fn a_broken_diagram_shows_an_error_and_draws_once_it_is_fixed(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, BROKEN, MarkdownOptions::new("Diagram"), WIDTH, 600.);
    frame(cx, window);
    assert_eq!(role(cx, window, diagram_id(BROKEN)), Some(Role::Alert));

    update(cx, &pane, |view, cx| view.set_source(FIXED, cx));
    frame(cx, window);
    assert_eq!(role(cx, window, diagram_id(FIXED)), Some(Role::Image));
}

/// SM-3
#[gpui_kit::test]
fn a_diagram_still_arriving_waits_for_its_closing_fence_before_drawing(cx: &mut TestAppContext) {
    init(cx);
    let options = MarkdownOptions::new("Diagram");
    let cache = options.diagrams.clone();
    let (open_fence, rest) = FIXED.split_at(FIXED.len() - "```\n".len());
    let (window, pane) = open(cx, open_fence, options, WIDTH, 600.);
    frame(cx, window);
    assert_eq!(role(cx, window, diagram_id(FIXED)), Some(Role::Status));
    assert_eq!(cache.stats().renders, 0, "nothing is drawn before the fence closes");

    update(cx, &pane, |view, cx| view.append(rest, cx));
    frame(cx, window);
    assert_eq!(role(cx, window, diagram_id(FIXED)), Some(Role::Image));
    assert_eq!(cache.stats().renders, 1);
}

/// SM-3
#[gpui_kit::test]
fn views_sharing_a_cache_draw_a_diagram_once(cx: &mut TestAppContext) {
    init(cx);
    let cache = MermaidCache::default();
    let mut first = MarkdownOptions::new("First");
    first.diagrams = cache.clone();
    let mut second = MarkdownOptions::new("Second");
    second.diagrams = cache.clone();
    let (one, pane) = open(cx, FIXED, first, WIDTH, 600.);
    frame(cx, one);
    let (two, _) = open(cx, FIXED, second, WIDTH, 600.);
    frame(cx, two);
    assert_eq!(role(cx, two, diagram_id(FIXED)), Some(Role::Image));

    // Text streaming in after the diagram redraws the view, not the diagram.
    update(cx, &pane, |view, cx| view.append("\nMore text.\n", cx));
    frame(cx, one);
    let stats = cache.stats();
    assert_eq!(stats.renders, 1, "{stats:?}");
    assert!(stats.hits >= 2, "{stats:?}");
}

fn comment(document: &Document, id: &str, passage: &str) -> Comment {
    let start = document.source().find(passage).expect("the passage is in the document");
    let selection = document
        .selection(start..start + passage.len(), passage)
        .expect("the range is in the source");
    Comment {
        id: id.into(),
        anchor: Anchor::new(document, &selection),
        body: format!("About {passage}"),
    }
}

/// The first paragraph keeps its surroundings when it is reworded; the last
/// one is deleted outright.
const PLAN: &str = "# Plan\n\nShip the parser first.\n\n## Later\n\nThen the view.\n";

/// SM-7, SM-8
#[gpui_kit::test]
fn comments_are_marked_outdated_or_gone_when_their_passage_changes(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, PLAN, MarkdownOptions::new("Plan"), WIDTH, 600.);
    update(cx, &pane, |view, cx| {
        let comments = vec![
            comment(view.document(), "c1", "Ship the parser first."),
            comment(view.document(), "c2", "Then the view."),
        ];
        view.set_comments(comments, cx);
    });
    frame(cx, window);
    assert_eq!(role(cx, window, "md-comment-c1"), Some(Role::Comment));
    assert!(!present(cx, window, "md-gone-comments"));

    let edited = "# Plan\n\nShip the view first.\n\n## Later\n";
    update(cx, &pane, |view, cx| view.set_source(edited, cx));
    frame(cx, window);
    let statuses = markdown(cx, &pane).read_with(cx, |view, _| view.comment_statuses().to_vec());
    assert!(matches!(statuses[0], AnchorStatus::Outdated { .. }), "{statuses:?}");
    assert_eq!(statuses[1], AnchorStatus::Gone);
    // The outdated one stays in the margin; the gone one waits in the notice.
    assert_eq!(role(cx, window, "md-comment-c1"), Some(Role::Comment));
    assert!(!present(cx, window, "md-comment-c2"));
    assert_eq!(role(cx, window, "md-gone-comments"), Some(Role::Status));

    cx.update_window(window, |_, window, cx| window.click("md-drop-c2", cx))
        .expect("the window is open");
    frame(cx, window);
    let events = pane.read_with(cx, |pane, _| pane.events.clone());
    assert!(events.contains(&MarkdownEvent::DropRequested { id: "c2".into() }));
    let ids: Vec<String> = markdown(cx, &pane).read_with(cx, |view, _| {
        view.comments().iter().map(|comment| comment.id.clone()).collect()
    });
    assert_eq!(ids, vec!["c1".to_owned()]);
    assert!(!present(cx, window, "md-gone-comments"));
}

/// SM-8
#[gpui_kit::test]
fn counts_light_each_commented_block_once_and_draw_no_margin_notes(cx: &mut TestAppContext) {
    init(cx);
    let mut options = MarkdownOptions::new("Plan");
    options.marks = CommentMarks::Counts;
    let (window, pane) = open(cx, PLAN, options, WIDTH, 600.);
    update(cx, &pane, |view, cx| {
        let comments = vec![
            comment(view.document(), "c1", "Ship the parser first."),
            comment(view.document(), "c2", "Ship the parser first."),
            comment(view.document(), "c3", "Then the view."),
        ];
        view.set_comments(comments, cx);
    });
    frame(cx, window);
    // Blocks: 0 heading, 1 first paragraph, 2 heading, 3 last paragraph.
    assert!(present(cx, window, ("md-comment-lit", 1usize)));
    assert!(present(cx, window, ("md-comment-lit", 3usize)));
    assert!(!present(cx, window, ("md-comment-lit", 0usize)));
    assert!(!present(cx, window, ("md-comment-lit", 2usize)));
    // One count per block, reported through the block's first comment; the
    // second comment of a block has no mark of its own and no note text.
    assert_eq!(role(cx, window, "md-comment-c1"), Some(Role::Comment));
    assert_eq!(role(cx, window, "md-comment-c3"), Some(Role::Comment));
    assert!(!present(cx, window, "md-comment-c2"));
    let tree = captured(cx, window).0;
    assert!(!tree.contains("About Ship the parser first."), "{tree}");
    assert!(!tree.contains("About Then the view."), "{tree}");

    cx.update_window(window, |_, window, cx| window.click("md-comment-c1", cx))
        .expect("the window is open");
    frame(cx, window);
    let events = pane.read_with(cx, |pane, _| pane.events.clone());
    assert!(events.contains(&MarkdownEvent::CommentActivated { id: "c1".into() }), "{events:?}");
}

/// SM-7
#[gpui_kit::test]
fn re_attaching_a_gone_comment_reports_the_comment(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, PLAN, MarkdownOptions::new("Plan"), WIDTH, 600.);
    update(cx, &pane, |view, cx| {
        let comments = vec![comment(view.document(), "c1", "Then the view.")];
        view.set_comments(comments, cx);
        view.set_source("Something else entirely.\n", cx);
    });
    frame(cx, window);
    cx.update_window(window, |_, window, cx| window.click("md-reattach-c1", cx))
        .expect("the window is open");
    frame(cx, window);
    let events = pane.read_with(cx, |pane, _| pane.events.clone());
    assert!(
        events
            .iter()
            .any(|event| matches!(event, MarkdownEvent::ReattachRequested { id, .. } if id == "c1")),
        "{events:?}"
    );
}

/// SM-5
#[gpui_kit::test]
fn a_diff_base_marks_changed_inserted_and_deleted_blocks(cx: &mut TestAppContext) {
    init(cx);
    let old = "# Plan\n\nKeep this.\n\nChange this line.\n\nDrop this.\n";
    let new = "# Plan\n\nKeep this.\n\nChange this sentence.\n\nA new paragraph.\n";
    let (window, pane) = open(cx, new, MarkdownOptions::new("Plan"), WIDTH, 600.);
    update(cx, &pane, |view, cx| view.set_diff_base(Some(old), cx));
    frame(cx, window);
    assert!(!present(cx, window, ("md-change", 0usize)));
    assert!(!present(cx, window, ("md-change", 1usize)));
    assert_eq!(role(cx, window, ("md-change", 2usize)), Some(Role::Mark));
    assert_eq!(role(cx, window, ("md-change", 3usize)), Some(Role::Mark));
    assert_eq!(role(cx, window, ("md-deleted", 0usize)), Some(Role::Mark));

    update(cx, &pane, |view, cx| view.set_diff_base(None, cx));
    frame(cx, window);
    assert!(!present(cx, window, ("md-change", 2usize)));
    assert!(!present(cx, window, ("md-deleted", 0usize)));
}

/// SM-3
#[gpui_kit::test]
fn an_image_is_loaded_once_however_often_the_view_redraws(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, HEIGHT);
    frame(cx, window);
    let before = markdown(cx, &pane).read_with(cx, |view, _| view.stats().images);
    assert!(before.loads >= 1, "{before:?}");
    update(cx, &pane, |view, cx| view.append("\nMore text.\n", cx));
    frame(cx, window);
    let after = markdown(cx, &pane).read_with(cx, |view, _| view.stats().images);
    assert_eq!(after.loads, before.loads, "redrawing loaded an image again");
    assert!(after.hits > before.hits, "{after:?}");
}

/// SM-4
#[gpui_kit::test]
fn the_outline_lists_the_headings_and_revealing_one_scrolls_to_it(cx: &mut TestAppContext) {
    init(cx);
    let (window, pane) = open(cx, GALLERY, gallery_options(), WIDTH, 400.);
    frame(cx, window);
    let outline = markdown(cx, &pane).read_with(cx, |view, _| view.outline());
    let levels: Vec<u8> = outline.iter().map(|entry| entry.level).collect();
    assert_eq!(levels, vec![1, 2, 2, 2, 2, 2, 2]);
    let last = outline.last().expect("the gallery has headings").block;
    assert!(!present(cx, window, ("md-block", last)));

    update(cx, &pane, |view, cx| view.reveal_block(last, cx));
    frame(cx, window);
    assert!(present(cx, window, ("md-block", last)));
    assert!(!present(cx, window, ("md-block", 0usize)));
}

/// SM-3
#[gpui_kit::test]
fn a_ten_thousand_line_document_draws_only_what_is_on_screen(cx: &mut TestAppContext) {
    init(cx);
    let source: String = (1..=2500)
        .map(|n| format!("## Section {n}\n\nLine one of {n}\nline two\n\n"))
        .collect();
    assert!(source.lines().count() >= 10_000);
    let (window, pane) = open(cx, &source, MarkdownOptions::new("Long"), WIDTH, 800.);
    frame(cx, window);
    let blocks = markdown(cx, &pane).read_with(cx, |view, _| view.document().blocks().len());
    assert_eq!(blocks, 5000);
    let stats = markdown(cx, &pane).read_with(cx, |view, _| view.stats());
    assert!(stats.blocks_aligned, "{stats:?}");
    assert!(stats.labelled_blocks > 0 && stats.labelled_blocks < 60, "{stats:?}");

    update(cx, &pane, |view, cx| view.reveal_block(4998, cx));
    frame(cx, window);
    assert!(present(cx, window, ("md-block", 4998usize)));
    let stats = markdown(cx, &pane).read_with(cx, |view, _| view.stats());
    assert!(stats.labelled_blocks < 60, "{stats:?}");
}
