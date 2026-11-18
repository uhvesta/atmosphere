//! A stand-in for the pane the Markdown view is embedded in, for the GUI
//! tests: it answers `CommentRequested` with a comment anchored to the
//! selection, the way the app does, and shows the last event in a status line
//! so snapshots and flows can see what the view reported.

use std::path::{Path, PathBuf};

use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, Bounds, Context, Entity, Point, Role, Subscription, TestAppContext, Window,
    WindowBounds, WindowOptions, div, px, size,
};
use humata2_markdown::{Anchor, Comment, MarkdownEvent, MarkdownOptions, MarkdownView, key_bindings};

/// The gallery: every kind of block the view draws, two images (one there,
/// one missing) and a diagram.
pub const GALLERY: &str = include_str!("../fixtures/gallery.md");

/// Where the fixtures are, from the test's working directory (the runfiles).
const FIXTURES: &str = "apps/humata2/markdown/headless/fixtures";

/// The folder the gallery's images are in: the real folder, not the
/// runfiles' links to it, because the view refuses an image whose link leads
/// out of its base folder.
#[must_use]
pub fn fixtures() -> PathBuf {
    Path::new(FIXTURES)
        .join("gallery.md")
        .canonicalize()
        .ok()
        .and_then(|gallery| gallery.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from(FIXTURES))
}

/// A Markdown view with a status line.
pub struct Pane {
    pub view: Entity<MarkdownView>,
    /// Every event the view sent, in order.
    pub events: Vec<MarkdownEvent>,
    next_id: u64,
    _events: Subscription,
}

impl Pane {
    pub fn new(source: &str, options: MarkdownOptions, cx: &mut Context<Self>) -> Self {
        let view = cx.new(|cx| MarkdownView::new(source, options, cx));
        let events = cx.subscribe(&view, |pane: &mut Self, view, event: &MarkdownEvent, cx| {
            pane.events.push(event.clone());
            match event {
                MarkdownEvent::CommentRequested(selection) => {
                    pane.next_id += 1;
                    let id = format!("c{}", pane.next_id);
                    view.update(cx, |view, cx| {
                        let anchor = Anchor::new(view.document(), selection);
                        let first = selection.quote.lines().next().unwrap_or_default().trim();
                        let mut comments = view.comments().to_vec();
                        comments.push(Comment {
                            id,
                            anchor,
                            body: format!("About: {first}"),
                        });
                        view.set_comments(comments, cx);
                    });
                }
                MarkdownEvent::DropRequested { id } => {
                    view.update(cx, |view, cx| {
                        let comments = view.comments().iter().filter(|c| c.id != *id).cloned().collect();
                        view.set_comments(comments, cx);
                    });
                }
                _ => {}
            }
            cx.notify();
        });
        Self {
            view,
            events: Vec::new(),
            next_id: 0,
            _events: events,
        }
    }
}

/// What the status line says about an event.
#[must_use]
pub fn describe(event: Option<&MarkdownEvent>) -> String {
    match event {
        None => "No events yet".to_owned(),
        Some(MarkdownEvent::LinkActivated { url }) => format!("Open {url}"),
        Some(MarkdownEvent::CommentRequested(selection)) => format!(
            "Comment requested on lines {}-{}: {:?}",
            selection.lines.start, selection.lines.end, selection.target
        ),
        Some(MarkdownEvent::CommentActivated { id }) => format!("Comment {id} activated"),
        Some(MarkdownEvent::ReattachRequested { id, .. }) => format!("Re-attach {id}"),
        Some(MarkdownEvent::DropRequested { id }) => format!("Drop {id}"),
        Some(MarkdownEvent::ModeChanged(mode)) => format!("Mode: {mode:?}"),
    }
}

impl Render for Pane {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let status = describe(self.events.last());
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h(px(0.)).child(self.view.clone()))
            .child(
                div()
                    .id("pane-status")
                    .test_support()
                    .role(Role::Status)
                    .aria_label("Last event")
                    .h(px(22.))
                    .px(px(8.))
                    .text_size(px(12.))
                    .child(status.clone())
                    .test_text(status),
            )
    }
}

/// Window options for a test window that is never shown.
#[must_use]
pub fn window_options(width: f32, height: f32) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(width), px(height)),
        })),
        focus: false,
        show: false,
        ..Default::default()
    }
}

/// The gallery's options: its images resolve in [`fixtures`].
#[must_use]
pub fn gallery_options() -> MarkdownOptions {
    let mut options = MarkdownOptions::new("Gallery");
    options.base_dir = Some(fixtures());
    options
}

/// Sets up a test app once: gpui-kit and the view's key bindings.
pub fn init(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    cx.update(|cx| cx.bind_keys(key_bindings()));
}

/// Opens a pane on `source` in a test window. Call [`init`] first.
pub fn open(
    cx: &mut TestAppContext,
    source: &str,
    options: MarkdownOptions,
    width: f32,
    height: f32,
) -> (AnyWindowHandle, Entity<Pane>) {
    let source = source.to_owned();
    let (window, pane) = cx
        .update(|cx| {
            gpui_kit::open_window(window_options(width, height), cx, |_, cx| {
                cx.new(|cx| Pane::new(&source, options, cx))
            })
        })
        .expect("the test window opens");
    cx.run_until_parked();
    (window, pane)
}

/// An older gallery: the third list item and the last paragraph's wording
/// came later, and a closing paragraph went away.
#[must_use]
pub fn older_gallery() -> String {
    format!(
        "{}\nA closing paragraph that went away.\n",
        GALLERY.replace("- Third item\n", "").replace("The end.", "The finish.")
    )
}

/// Comments on a passage, the table and the diagram, one whose passage is
/// gone, and a diff against [`older_gallery`].
pub fn annotate(view: &mut MarkdownView, cx: &mut Context<MarkdownView>) {
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
    let old = humata2_markdown::Document::new("A passage that is long gone from the gallery.\n");
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
