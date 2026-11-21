//! Smart Markdown rendering, Diff | Split | Rendered switching, and rendered
//! block commenting for the full-screen Diff view.

use std::rc::Rc;

use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::comments::Comment;
use humata2_clients::model::{FileDiff, LineKind, RepoDiff};
use humata2_review::{DiffMarkdownMode, ReviewAction};

use crate::bridge::Ui;
use crate::chrome::secondary;
use crate::comment_cards::{self, StartReattach};
use crate::document_tab::Documents;
use crate::theme::Look;
use crate::widgets::{col, row};

/// Whether `path` is a Markdown file for the full-screen Diff renderer
/// (`.md`, `.markdown`, or `.mdx`, any case).
pub(crate) fn is_diff_markdown(path: &str) -> bool {
    humata2_review::is_markdown_path(path)
}

/// Reconstructs the new-side Markdown text from a `FileDiff` when the full
/// file text hasn't been loaded yet, along with a 0-based -> 1-based real
/// new-side line number map so block comments anchor to exact file lines.
pub(crate) fn reconstruct_new_side(file: &FileDiff) -> (String, Vec<u32>) {
    let mut out: Vec<&str> = Vec::new();
    let mut map: Vec<u32> = Vec::new();
    let mut prev_new: Option<u32> = None;

    for line in &file.lines {
        if matches!(line.kind, LineKind::Context | LineKind::Added) {
            let real_line = line.new.unwrap_or((out.len() + 1) as u32);
            if let Some(prev) = prev_new
                && real_line > prev + 1
                && !out.last().is_some_and(|text| text.is_empty())
            {
                // Keep non-contiguous hunks separated as distinct Markdown blocks.
                out.push("");
                map.push(prev);
            }
            out.push(line.text.as_str());
            map.push(real_line);
            prev_new = Some(real_line);
        }
    }

    // Deleted Markdown files have no new-side lines; fall back to removed lines
    // so the user can still inspect the rendered document.
    if out.is_empty() {
        let mut prev_old: Option<u32> = None;
        for line in &file.lines {
            if line.kind == LineKind::Removed {
                let real_line = line.old.unwrap_or((out.len() + 1) as u32);
                if let Some(prev) = prev_old
                    && real_line > prev + 1
                    && !out.last().is_some_and(|text| text.is_empty())
                {
                    out.push("");
                    map.push(prev);
                }
                out.push(line.text.as_str());
                map.push(real_line);
                prev_old = Some(real_line);
            }
        }
    }

    (out.join("\n"), map)
}

/// Resolves the Markdown source to render from optional loaded full-file texts
/// (session tab text first, then review-loaded text), falling back to
/// reconstructed new-side diff lines.
fn resolve_markdown_source(
    session_text: Option<&str>,
    review_text: Option<&str>,
    file: &FileDiff,
) -> (String, Option<Vec<u32>>) {
    if let Some(text) = session_text {
        return (text.to_owned(), None);
    }
    if let Some(text) = review_text {
        return (text.to_owned(), None);
    }
    let (reconstructed, map) = reconstruct_new_side(file);
    (reconstructed, Some(map))
}

/// Returns the Markdown source to render for `(diff, file)` in the full-screen
/// Diff view, plus an optional 0-based -> 1-based line map when falling back
/// to reconstructed diff lines.
///
/// Prefers the loaded full file text (from session tabs or the review panel's
/// own read) so the entire document and its Mermaid diagrams render cleanly,
/// falling back gracefully to reconstructed new-side diff lines while loading.
pub(crate) fn diff_markdown_source(
    state: &AppState,
    diff: &RepoDiff,
    file: &FileDiff,
) -> (String, Option<Vec<u32>>) {
    let key = (diff.session.clone(), diff.repo.clone(), file.path.clone());
    let session_text = state
        .session
        .file_contents
        .get(&key)
        .and_then(|res| res.as_deref().ok());
    let review_text = state
        .review
        .diff_file_text(&diff.session, &diff.repo, &file.path);
    resolve_markdown_source(session_text, review_text, file)
}

/// Maps a 1-based line range in the rendered `MarkdownView` back to 1-based
/// real new-side line numbers in the file.
pub(crate) fn map_rendered_lines(start: usize, end: usize, line_map: Option<&[u32]>) -> (u32, u32) {
    let start_u32 = u32::try_from(start.max(1)).unwrap_or(u32::MAX);
    let end_u32 = u32::try_from(end.max(start).max(1)).unwrap_or(u32::MAX);
    let Some(map) = line_map else {
        return (start_u32, end_u32);
    };
    if map.is_empty() {
        return (start_u32, end_u32);
    }
    let first_idx = start.saturating_sub(1).min(map.len() - 1);
    let last_idx = end.saturating_sub(1).min(map.len() - 1);
    let first = map[first_idx];
    let last = map[last_idx].max(first);
    (first, last)
}

/// Maps a 1-based real new-side line range to 1-based line numbers in the
/// rendered `MarkdownView` so comment highlights land on the right block.
pub(crate) fn map_real_to_rendered_lines(
    first: u32,
    last: u32,
    line_map: Option<&[u32]>,
) -> Option<(u32, u32)> {
    let Some(map) = line_map else {
        return Some((first.max(1), last.max(first).max(1)));
    };
    let mut rendered_first: Option<u32> = None;
    let mut rendered_last: Option<u32> = None;
    for (idx, &real) in map.iter().enumerate() {
        if real >= first && real <= last {
            let line = u32::try_from(idx + 1).ok()?;
            rendered_first.get_or_insert(line);
            rendered_last = Some(line);
        }
    }
    Some((rendered_first?, rendered_last?))
}

/// Compact `Diff | Split | Rendered` segmented switcher for the full-screen
/// Diff file header bar when viewing a Markdown file.
pub(super) fn mode_switch(ui: &Rc<Ui>, look: Look, current: DiffMarkdownMode) -> AnyElement {
    let segments = [
        (DiffMarkdownMode::Diff, "Diff", "diff"),
        (DiffMarkdownMode::Split, "Split", "split"),
        (DiffMarkdownMode::Rendered, "Rendered", "rendered"),
    ];
    row()
        .id("diff-md-modes")
        .role(Role::RadioGroup)
        .aria_label("Markdown view mode")
        .test_support()
        .p(px(2.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .flex_shrink_0()
        .children(segments.into_iter().map(|(mode, label, slug)| {
            let on = mode == current;
            let ui = ui.clone();
            div()
                .id(SharedString::from(format!("diff-md-mode-{slug}")))
                .role(Role::RadioButton)
                .aria_label(label)
                .aria_selected(on)
                .px(px(9.))
                .py(px(1.5))
                .rounded(px(5.))
                .text_size(px(11.5))
                .text_color(rgb(if on { look.text } else { look.secondary }))
                .cursor_pointer()
                .when(on, |segment| segment.bg(rgb(look.card)).shadow_sm())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    ui.send(AppAction::Review(ReviewAction::SetMarkdownMode(mode)), cx);
                })
                .child(label)
                .test_support()
        }))
        .into_any_element()
}

/// Whether `file`'s diff lines include new-side line `line`.
fn diff_has_new_line(file: &FileDiff, line: u32) -> bool {
    file.lines
        .iter()
        .any(|each| matches!(each.kind, LineKind::Context | LineKind::Added) && each.new == Some(line))
}

/// The smart rendered Markdown pane for full-screen Diff (`Split` or
/// `Rendered` mode), including Mermaid diagrams + zoom and direct block
/// commenting wired into the review draft flow.
#[allow(clippy::too_many_arguments)]
pub(super) fn rendered_pane(
    ui: &Rc<Ui>,
    state: &AppState,
    diff: &RepoDiff,
    file: &FileDiff,
    documents: &Documents,
    look: Look,
    split: bool,
) -> AnyElement {
    let page = documents.diff_markdown_view(&diff.session, &diff.repo, &file.path);
    let body = match page {
        Some(view) => div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .when(!split, |inner| inner.max_w(px(760.)))
                    .px(px(28.))
                    .h_full()
                    .child(view),
            )
            .into_any_element(),
        None => col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .items_center()
            .justify_center()
            .p(px(12.))
            .child(secondary(look, "Rendering Markdown…").text_size(px(12.)))
            .into_any_element(),
    };

    let session_info = state.workspace.session(&diff.session);
    let agent = session_info.map_or("agent", |session| session.agent.command());
    let is_pr_review = session_info.is_some_and(|session| session.pull_request_review);

    let editing = state
        .comments
        .sessions
        .get(&diff.session)
        .and_then(|held| held.editing.as_ref())
        .filter(|b| b.anchor.repo == diff.repo && b.anchor.path == file.path);

    // In Rendered mode the diff pane isn't visible, so always show the open
    // comment composer in a docked tray. In Split mode the left diff pane shows
    // comments anchored to lines in the diff; only show the tray when the
    // rendered block's line lies outside the diff hunks.
    let show_open_box = editing.is_some_and(|b| !split || !diff_has_new_line(file, b.anchor.last));
    let editing_comment_id = editing.and_then(|b| b.comment.clone());

    let mut tray_items: Vec<AnyElement> = Vec::new();
    if !split {
        let by_line = state.comments.on_file(&diff.session, &diff.repo, &file.path);
        let reattaching = documents.reattach().borrow().clone();
        let cell = documents.reattach();
        let start: StartReattach = Rc::new(move |id, cx| {
            *cell.borrow_mut() = Some(id.clone());
            cx.refresh_windows();
        });
        let todo_ctx = comment_cards::todo_context(state, &diff.session);
        let existing: Vec<&Comment> = by_line
            .into_values()
            .flatten()
            .filter(|comment| Some(&comment.id) != editing_comment_id.as_ref())
            .collect();
        for comment in existing {
            tray_items.push(
                comment_cards::card_with_todo(
                    ui,
                    look,
                    comment,
                    agent,
                    is_pr_review,
                    reattaching.as_ref() == Some(&comment.id),
                    &start,
                    &todo_ctx,
                )
                .into_any_element(),
            );
        }
    }
    if show_open_box
        && let Some(b) = editing
    {
        tray_items.push(
            comment_cards::open_box(
                ui,
                look,
                &diff.session,
                &b.anchor,
                &documents.input(),
                agent,
                &b.text,
                &comment_cards::offers(state, &diff.session, &b.anchor.repo),
            )
            .into_any_element(),
        );
    }

    let tray = (!tray_items.is_empty()).then(|| {
        col()
            .id("diff-md-comments-tray")
            .test_support()
            .flex_shrink_0()
            .max_h(px(280.))
            .overflow_y_scroll()
            .border_t_1()
            .border_color(rgb(look.line))
            .bg(rgb(look.panel))
            .p(px(8.))
            .gap(px(6.))
            .children(tray_items)
    });

    col()
        .id("diff-md-rendered")
        .role(Role::Document)
        .aria_label(format!("{} rendered", file.path))
        .test_support()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .bg(rgb(look.content))
        .child(body)
        .children(tray)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    // The glob above brings GPUI's `test` macro; `#[test]` is the standard one.
    use ::core::prelude::v1::test;
    use humata2_clients::model::{DiffLine, FileDiff, LineKind, RepoDiff, SessionId};

    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> DiffLine {
        DiffLine {
            kind,
            old,
            new,
            text: text.to_owned(),
        }
    }

    #[test]
    fn recognizes_md_markdown_and_mdx_only() {
        for path in [
            "README.md",
            "docs/spec.MD",
            "notes/arch.markdown",
            "site/page.mdx",
            "site/PAGE.MDX",
        ] {
            assert!(is_diff_markdown(path), "expected markdown: {path}");
        }
        for path in ["src/main.rs", "README.md.bak", "notes.txt", "markdown", ""] {
            assert!(!is_diff_markdown(path), "expected non-markdown: {path}");
        }
    }

    #[test]
    fn reconstructs_new_side_with_line_map_and_hunk_separation() {
        let file = FileDiff {
            path: "docs/arch.md".to_owned(),
            lines: vec![
                line(LineKind::Hunk, None, None, "@@ -1,2 +1,2 @@"),
                line(LineKind::Removed, Some(1), None, "# Old Title"),
                line(LineKind::Added, None, Some(1), "# New Title"),
                line(LineKind::Context, Some(2), Some(2), "Intro line"),
                line(LineKind::Hunk, None, None, "@@ -20,1 +20,4 @@"),
                line(LineKind::Added, None, Some(20), "```mermaid"),
                line(LineKind::Added, None, Some(21), "graph TD"),
                line(LineKind::Added, None, Some(22), "  A --> B"),
                line(LineKind::Added, None, Some(23), "```"),
            ],
            ..FileDiff::default()
        };

        let (text, map) = reconstruct_new_side(&file);
        assert_eq!(
            text,
            "# New Title\nIntro line\n\n```mermaid\ngraph TD\n  A --> B\n```"
        );
        assert_eq!(map, vec![1, 2, 2, 20, 21, 22, 23]);

        // Mermaid block occupies reconstructed lines 4..=7 -> real lines 20..=23.
        assert_eq!(map_rendered_lines(4, 7, Some(&map)), (20, 23));
        // Inverse mapping maps real lines 20..=23 back to reconstructed lines 4..=7.
        assert_eq!(map_real_to_rendered_lines(20, 23, Some(&map)), Some((4, 7)));
    }

    #[test]
    fn full_file_source_takes_precedence_over_diff_reconstruction() {
        let file = FileDiff {
            path: "README.md".to_owned(),
            lines: vec![line(LineKind::Added, None, Some(5), "Only hunk line")],
            ..FileDiff::default()
        };

        // Fallback to diff new-side when nothing is loaded.
        let (src, map) = resolve_markdown_source(None, None, &file);
        assert_eq!(src, "Only hunk line");
        assert_eq!(map, Some(vec![5]));

        // Review-loaded full file upgrades to full text with identity line map.
        let (src, map) = resolve_markdown_source(None, Some("# Full\n\nBody"), &file);
        assert_eq!(src, "# Full\n\nBody");
        assert_eq!(map, None);

        // Session tab content takes precedence when loaded.
        let (src, map) = resolve_markdown_source(Some("# Tab Full"), Some("# Full\n\nBody"), &file);
        assert_eq!(src, "# Tab Full");
        assert_eq!(map, None);
    }

    #[test]
    fn deleted_markdown_file_falls_back_to_removed_lines() {
        let file = FileDiff {
            path: "docs/old.mdx".to_owned(),
            lines: vec![
                line(LineKind::Hunk, None, None, "@@ -1,2 +0,0 @@"),
                line(LineKind::Removed, Some(1), None, "# Removed Doc"),
                line(LineKind::Removed, Some(2), None, "Goodbye"),
            ],
            ..FileDiff::default()
        };
        let (src, map) = reconstruct_new_side(&file);
        assert_eq!(src, "# Removed Doc\nGoodbye");
        assert_eq!(map, vec![1, 2]);
    }
}
