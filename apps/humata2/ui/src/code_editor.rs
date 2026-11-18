//! A changed code file's editor in a session tab (RV-17–RV-22): Inline, Side
//! by side or Full file, with unchanged stretches folded, the count of the
//! changes and ↑/↓ to step through them. The rows are the session crate's
//! `editor::rows`; this draws them and sends the choices back.
//!
//! GPUI's handles live in [`EditorView`], one per file in `Documents`: the
//! scroll position, and whether a step asked for the cursor's change to be
//! shown.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::comments::{Anchor, Comment, CommentId, Side};
use humata2_clients::model::{DiffLine, FileDiff, LineKind, Session, SessionId};
use humata2_review::comments::CommentsAction;
use humata2_session::SessionAction;
use humata2_session::editor::{self, CodeMode, Row, SideRow};

use crate::bridge::Ui;
use crate::comment_cards::{self, StartReattach};
use crate::document_tab::Documents;
use crate::theme::Look;
use crate::widgets::{col, row};

/// What GPUI keeps for one changed file's editor between frames.
#[derive(Clone)]
pub(crate) struct EditorView {
    /// The rows' scroll position.
    pub(crate) handle: ScrollHandle,
    /// Set by a step; the next frame that can scroll shows the cursor's change.
    pending: Rc<Cell<bool>>,
}

impl EditorView {
    pub(crate) fn new() -> Self {
        Self {
            handle: ScrollHandle::new(),
            pending: Rc::new(Cell::new(false)),
        }
    }
}

/// The diff of a file the session changed, if it has changes to show.
pub(crate) fn diff<'a>(
    state: &'a AppState,
    session: &SessionId,
    repo: &str,
    path: &str,
) -> Option<&'a FileDiff> {
    state
        .workspace
        .diffs
        .iter()
        .filter(|diff| diff.session == *session && diff.repo == repo)
        .flat_map(|diff| diff.files.iter())
        .find(|file| file.path == path && file.added + file.removed > 0)
}

/// The editor for a changed file whose text is loaded.
#[allow(clippy::too_many_arguments)]
pub(crate) fn body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    text: &str,
    diff: &FileDiff,
    documents: &Documents,
    look: Look,
) -> AnyElement {
    let view = documents.editor_view(repo, path);
    let code = state
        .session
        .views
        .get(&session.id)
        .map(|page| page.code(repo, path))
        .unwrap_or_default();
    let cursor = state.session.change(&session.id, repo, path);
    let rows = editor::rows(Some(diff), text, &code);
    let commenting = Rc::new(Commenting {
        ui: ui.clone(),
        session: session.id.clone(),
        repo: repo.to_owned(),
        path: path.to_owned(),
        reattach: documents.reattach(),
    });
    let side_rows = (code.mode == CodeMode::SideBySide).then(|| editor::side_by_side(&rows));
    let (lines, starts, slots): (Vec<Vec<AnyElement>>, Vec<usize>, Vec<Slot>) = match &side_rows {
        Some(side) => {
            let starts = editor::side_changes(side);
            let slots = side
                .iter()
                .map(|row| match row {
                    SideRow::Pair { old, new } => Slot::Lines(old.iter().chain(new.iter()).collect()),
                    SideRow::Fold { start, count } => Slot::Fold { start: *start, count: *count },
                })
                .collect();
            let lines = side
                .iter()
                .enumerate()
                .map(|(index, row)| match row {
                    SideRow::Pair { old, new } => {
                        vec![side_pair(index, old.as_ref(), new.as_ref(), &commenting, look)]
                    }
                    SideRow::Fold { start, count } => {
                        vec![fold(ui, repo, path, *start, *count, look)]
                    }
                })
                .collect();
            (lines, starts, slots)
        }
        None => {
            let starts = editor::changes(&rows);
            let slots = rows
                .iter()
                .map(|row| match row {
                    Row::Line(line) => Slot::Lines(vec![line]),
                    Row::Fold { start, count } => Slot::Fold { start: *start, count: *count },
                })
                .collect();
            let lines = rows
                .iter()
                .enumerate()
                .map(|(index, row)| match row {
                    Row::Line(line) => vec![line_row(index, line, &commenting, look)],
                    Row::Fold { start, count } => vec![fold(ui, repo, path, *start, *count, look)],
                })
                .collect();
            (lines, starts, slots)
        }
    };
    let under = comments_under(ui, state, session, repo, path, &slots, documents, look);
    let lines: Vec<AnyElement> = lines
        .into_iter()
        .zip(under)
        .flat_map(|(mut elements, cards)| {
            elements.extend(cards);
            elements
        })
        .collect();
    let sendable = state.comments.sendable(&session.id).len();
    let count = starts.len();
    let current = cursor.filter(|at| *at < count);
    if view.pending.get() {
        match current.and_then(|at| starts.get(at)) {
            Some(&index) => {
                if reveal(&view.handle, index) {
                    view.pending.set(false);
                }
            }
            None => view.pending.set(false),
        }
    }

    let mut header = row()
        .px(px(14.))
        .py(px(8.))
        .gap(px(10.))
        .items_center()
        .flex_shrink_0()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .child(format!("{repo}/{path}")),
        )
        .child(modes(ui, repo, path, code.mode, look));
    if code.mode == CodeMode::FullFile {
        header = header.child(show_changes(ui, repo, path, code.changes, look));
    }
    if count > 0 {
        header = header.child(counter(ui, &view, repo, path, current, count, look));
    }

    col()
        .id("session-file")
        .role(Role::Document)
        .aria_label(format!("{repo}/{path}"))
        .flex_1()
        .min_h_0()
        .bg(rgb(look.term))
        .text_color(rgb(look.text))
        .child(header)
        .child(
            col()
                .id("editor-rows")
                .role(Role::Code)
                .aria_label(format!("{repo}/{path}"))
                .test_support()
                .flex_1()
                .min_h_0()
                .py(px(4.))
                .font_family("Menlo")
                .text_size(px(12.))
                .line_height(px(18.))
                .track_scroll(&view.handle)
                .overflow_y_scroll()
                .children(lines),
        )
        .when(sendable > 0, |column| {
            column.child(drafts(ui, &session.id, sendable, look))
        })
        .test_support()
        .into_any_element()
}

/// Where the editor's line numbers send a comment: the session, the file and
/// the cell a detached comment waits in to be re-attached (shared with the
/// Diff panel's line numbers, so either view can take the click).
struct Commenting {
    ui: Rc<Ui>,
    session: SessionId,
    repo: String,
    path: String,
    reattach: Rc<RefCell<Option<CommentId>>>,
}

impl Commenting {
    /// A new-side line number was clicked. A detached comment waiting for a
    /// line takes it; otherwise the comment box opens on that line (RV-20).
    fn click(&self, number: u32, text: &str, cx: &mut App) {
        let anchor = Anchor {
            repo: self.repo.clone(),
            path: self.path.clone(),
            side: Side::New,
            first: number,
            last: number,
            quoted: vec![text.to_owned()],
            base: None,
            detached: false,
            outdated: false,
        };
        let pending = self.reattach.borrow_mut().take();
        let action = match pending {
            Some(id) => CommentsAction::Reattach(self.session.clone(), id, anchor),
            None => CommentsAction::OpenBox(self.session.clone(), anchor),
        };
        self.ui.send(AppAction::Comments(action), cx);
    }
}

/// One row of the editor as the comments see it: the lines it shows, or a
/// fold of unchanged lines.
enum Slot<'a> {
    Lines(Vec<&'a DiffLine>),
    Fold { start: u32, count: u32 },
}

/// Whether `line` is on `side` and numbered `number` there, as the Diff
/// panel decides it: a context line has both numbers.
fn at(line: &DiffLine, side: Side, number: u32) -> bool {
    match side {
        Side::Old => line.kind != LineKind::Added && line.old == Some(number),
        Side::New => line.kind != LineKind::Removed && line.new == Some(number),
    }
}

/// The row a comment or box ending at `number` on `side` goes under: the row
/// showing that line, else the fold hiding it, else the last row.
fn slot_of(slots: &[Slot], side: Side, number: u32) -> Option<usize> {
    slots
        .iter()
        .position(|slot| match slot {
            Slot::Lines(lines) => lines.iter().any(|line| at(line, side, number)),
            Slot::Fold { .. } => false,
        })
        .or_else(|| {
            slots.iter().position(|slot| match *slot {
                Slot::Fold { start, count } => {
                    side == Side::New && number >= start && number - start < count
                }
                Slot::Lines(_) => false,
            })
        })
        .or_else(|| slots.len().checked_sub(1))
}

/// The comment cards of each row, and the open box after the row it goes
/// under: the same cards and box as the Diff panel (RV-20).
#[allow(clippy::too_many_arguments)]
fn comments_under(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    slots: &[Slot],
    documents: &Documents,
    look: Look,
) -> Vec<Vec<AnyElement>> {
    let editing = state
        .comments
        .sessions
        .get(&session.id)
        .and_then(|held| held.editing.as_ref())
        .filter(|open| open.anchor.repo == repo && open.anchor.path == path);
    let box_comment = editing.and_then(|open| open.comment.clone());
    let box_at = editing.and_then(|open| slot_of(slots, open.anchor.side, open.anchor.last));
    let mut cards: Vec<Vec<&Comment>> = vec![Vec::new(); slots.len()];
    for comment in state
        .comments
        .on_file(&session.id, repo, path)
        .into_values()
        .flatten()
    {
        if Some(&comment.id) == box_comment.as_ref() {
            continue;
        }
        if let Some(index) = slot_of(slots, comment.anchor.side, comment.anchor.last) {
            cards[index].push(comment);
        }
    }
    let agent = session.agent.command();
    let reattach = documents.reattach();
    let reattaching = reattach.borrow().clone();
    let cell = reattach.clone();
    let start: StartReattach = Rc::new(move |id: &CommentId, cx: &mut App| {
        *cell.borrow_mut() = Some(id.clone());
        cx.refresh_windows();
    });
    let input = documents.input();
    cards
        .into_iter()
        .enumerate()
        .map(|(index, comments)| {
            let mut elements: Vec<AnyElement> = comments
                .into_iter()
                .map(|comment| {
                    comment_cards::card(
                        ui,
                        look,
                        comment,
                        agent,
                        reattaching.as_ref() == Some(&comment.id),
                        &start,
                    )
                    .into_any_element()
                })
                .collect();
            if let Some(open) = editing.filter(|_| box_at == Some(index)) {
                elements.push(
                    comment_cards::open_box(
                        ui,
                        look,
                        &session.id,
                        &open.anchor,
                        &input,
                        agent,
                        &open.text,
                        &comment_cards::offers(state, &session.id, &open.anchor.repo),
                    )
                    .into_any_element(),
                );
            }
            elements
        })
        .collect()
}

/// The drafts footer (RV-5): how many the Send would type, and Send them all.
fn drafts(ui: &Rc<Ui>, session: &SessionId, count: usize, look: Look) -> AnyElement {
    let noun = if count == 1 { "comment" } else { "comments" };
    let shown = format!("{count} draft {noun}");
    let send_label = format!("Send {count} {noun}");
    let ui = ui.clone();
    let session = session.clone();
    row()
        .id("editor-drafts")
        .items_center()
        .gap(px(8.))
        .p(px(10.))
        .border_t_1()
        .border_color(rgb(look.line))
        .flex_shrink_0()
        .text_size(px(12.))
        .text_color(rgb(look.secondary))
        .child(div().flex_1().child(shown.clone()).test_text(shown))
        .child(
            div()
                .id("editor-send-comments")
                .role(Role::Button)
                // The Diff panel shows its own "Send N comments"; a distinct
                // label keeps the two apart for screen readers.
                .aria_label(format!("{send_label} from the editor"))
                .px(px(11.))
                .py(px(3.))
                .rounded(px(6.))
                .bg(rgb(look.accent))
                .text_color(rgb(0xffffff))
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    ui.send(
                        AppAction::Comments(CommentsAction::SendAll(session.clone())),
                        cx,
                    );
                })
                .child(send_label.clone())
                .test_text(send_label)
                .test_support(),
        )
        .test_support()
        .into_any_element()
}

/// The editor's three modes, as a segmented control (like a Markdown tab's).
fn modes(ui: &Rc<Ui>, repo: &str, path: &str, current: CodeMode, look: Look) -> AnyElement {
    let segments = [
        (CodeMode::Inline, "Inline", "inline"),
        (CodeMode::SideBySide, "Side by side", "side-by-side"),
        (CodeMode::FullFile, "Full file", "full-file"),
    ];
    row()
        .id("editor-modes")
        .role(Role::RadioGroup)
        .aria_label("Show the changes as")
        .test_support()
        .p(px(2.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .flex_shrink_0()
        .children(segments.into_iter().map(|(mode, label, slug)| {
            let on = mode == current;
            let ui = ui.clone();
            let (repo, path) = (repo.to_owned(), path.to_owned());
            div()
                .id(SharedString::from(format!("editor-mode-{slug}")))
                .role(Role::RadioButton)
                .aria_label(label)
                .aria_selected(on)
                .px(px(12.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(11.5))
                .text_color(rgb(if on { look.text } else { look.secondary }))
                .cursor_pointer()
                .when(on, |segment| segment.bg(rgb(look.card)).shadow_sm())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    ui.send(
                        AppAction::Session(SessionAction::CodeMode {
                            repo: repo.clone(),
                            path: path.clone(),
                            mode,
                        }),
                        cx,
                    );
                })
                .child(label)
                .test_support()
        }))
        .into_any_element()
}

/// The Full file toggle for the changes (RV-22).
fn show_changes(ui: &Rc<Ui>, repo: &str, path: &str, shown: bool, look: Look) -> AnyElement {
    let ui = ui.clone();
    let (repo, path) = (repo.to_owned(), path.to_owned());
    row()
        .id("editor-show-changes")
        .role(Role::CheckBox)
        .aria_label("Show changes")
        .aria_selected(shown)
        .test_support()
        .gap(px(7.))
        .flex_shrink_0()
        .text_size(px(12.5))
        .text_color(rgb(look.text))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(
                AppAction::Session(SessionAction::ShowChanges {
                    repo: repo.clone(),
                    path: path.clone(),
                    shown: !shown,
                }),
                cx,
            );
        })
        .child(
            row()
                .size(px(14.))
                .flex_shrink_0()
                .rounded(px(4.))
                .justify_center()
                .when(shown, |box_| box_.bg(rgb(look.accent)))
                .when(!shown, |box_| {
                    box_.border_1()
                        .border_color(rgb(look.tertiary))
                        .bg(rgb(look.card))
                }),
        )
        .child(div().child("Show changes").test_text("Show changes".to_owned()))
        .into_any_element()
}

/// "N changes", or "i of N" while the cursor is on change i, with the
/// previous and next buttons beside it.
fn counter(
    ui: &Rc<Ui>,
    view: &EditorView,
    repo: &str,
    path: &str,
    current: Option<usize>,
    count: usize,
    look: Look,
) -> Div {
    let label = match current {
        Some(at) => format!("{} of {count}", at + 1),
        None if count == 1 => "1 change".to_owned(),
        None => format!("{count} changes"),
    };
    row()
        .gap(px(4.))
        .flex_shrink_0()
        .child(
            div()
                .id("editor-count")
                .px(px(4.))
                .text_color(rgb(look.secondary))
                .child(label.clone())
                .test_text(label)
                .test_support(),
        )
        .child(step(ui, view, repo, path, "editor-previous-change", "Previous change", "↑", false, count, look))
        .child(step(ui, view, repo, path, "editor-next-change", "Next change", "↓", true, count, look))
}

/// A previous or next button: it asks the editor to show the change it lands on.
#[allow(clippy::too_many_arguments)]
fn step(
    ui: &Rc<Ui>,
    view: &EditorView,
    repo: &str,
    path: &str,
    id: &str,
    label: &str,
    glyph: &str,
    forward: bool,
    count: usize,
    look: Look,
) -> AnyElement {
    let ui = ui.clone();
    let pending = view.pending.clone();
    let (repo, path) = (repo.to_owned(), path.to_owned());
    div()
        .id(SharedString::from(id.to_owned()))
        .role(Role::Button)
        .aria_label(label.to_owned())
        .cursor_pointer()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(5.))
        .text_color(rgb(look.secondary))
        .hover(|button| button.bg(rgb(look.fill)))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            pending.set(true);
            ui.send(
                AppAction::Session(SessionAction::StepChange {
                    repo: repo.clone(),
                    path: path.clone(),
                    forward,
                    count,
                }),
                cx,
            );
        })
        .child(glyph.to_owned())
        .test_support()
        .into_any_element()
}

/// Scrolls the rows so the child at `index` is inside the view. False while
/// GPUI has no bounds for it yet, so the step is asked again next frame.
fn reveal(handle: &ScrollHandle, index: usize) -> bool {
    let (Some(item), view) = (handle.bounds_for_item(index), handle.bounds()) else {
        return false;
    };
    let mut offset = handle.offset();
    if item.top() < view.top() {
        offset.y = offset.y + (view.top() - item.top());
    } else if item.bottom() > view.bottom() {
        offset.y = offset.y - (item.bottom() - view.bottom());
    }
    offset.y = offset.y.min(px(0.));
    handle.set_offset(offset);
    true
}

/// One fold of unchanged lines (RV-18); clicking it opens the stretch.
fn fold(ui: &Rc<Ui>, repo: &str, path: &str, start: u32, count: u32, look: Look) -> AnyElement {
    let noun = if count == 1 { "line" } else { "lines" };
    let label = format!("⋯ {count} unchanged {noun} — show");
    let ui = ui.clone();
    let (repo, path) = (repo.to_owned(), path.to_owned());
    row()
        .id(SharedString::from(format!("editor-fold-{start}")))
        .role(Role::Button)
        .aria_label(label.clone())
        .test_support()
        .px(px(12.))
        .py(px(2.))
        .cursor_pointer()
        .text_color(rgb(look.accent))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(
                AppAction::Session(SessionAction::Unfold {
                    repo: repo.clone(),
                    path: path.clone(),
                    start,
                }),
                cx,
            );
        })
        .child(label.clone())
        .test_text(label)
        .into_any_element()
}

/// A line of the inline and full-file rows: both numbers, the sign, the text.
fn line_row(index: usize, line: &DiffLine, commenting: &Rc<Commenting>, look: Look) -> AnyElement {
    let (sign, bg) = match line.kind {
        LineKind::Added => ("+", Some(look.add)),
        LineKind::Removed => ("−", Some(look.del)),
        _ => (" ", None),
    };
    row()
        .id(SharedString::from(format!("editor-line-{index}")))
        .items_start()
        .w_full()
        .min_h(px(18.))
        .when_some(bg, |row, bg| row.bg(rgb(bg)))
        .child(number(line.old, look))
        .child(new_number(line.new, line, commenting, look))
        .child(
            div()
                .w(px(16.))
                .flex_shrink_0()
                .text_color(rgb(look.term_text))
                .child(sign),
        )
        .child(
            div()
                .whitespace_nowrap()
                .text_color(rgb(look.term_text))
                .child(line.text.clone())
                .test_text(line.text.clone()),
        )
        .test_support()
        .into_any_element()
}

/// A pair of the side-by-side rows: the old line left, the new line right.
fn side_pair(
    index: usize,
    old: Option<&DiffLine>,
    new: Option<&DiffLine>,
    commenting: &Rc<Commenting>,
    look: Look,
) -> AnyElement {
    row()
        .id(SharedString::from(format!("editor-row-{index}")))
        .items_stretch()
        .w_full()
        .child(half(old, true, commenting, look))
        .child(div().w(px(1.)).flex_shrink_0().bg(rgb(look.line)))
        .child(half(new, false, commenting, look))
        .test_support()
        .into_any_element()
}

/// One side of a pair. An empty side is a muted cell standing in for hatching.
/// Only the new side's numbers take a comment.
fn half(line: Option<&DiffLine>, old_side: bool, commenting: &Rc<Commenting>, look: Look) -> Div {
    let Some(line) = line else {
        return div().flex_1().min_w_0().min_h(px(18.)).bg(rgb(look.fill));
    };
    let (shown, bg) = match (line.kind, old_side) {
        (LineKind::Removed, true) => (line.old, Some(look.del)),
        (LineKind::Added, false) => (line.new, Some(look.add)),
        (_, true) => (line.old, None),
        (_, false) => (line.new, None),
    };
    let gutter = if old_side {
        number(shown, look).into_any_element()
    } else {
        new_number(shown, line, commenting, look)
    };
    row()
        .flex_1()
        .min_w_0()
        .min_h(px(18.))
        .when_some(bg, |row, bg| row.bg(rgb(bg)))
        .child(gutter)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .text_color(rgb(look.term_text))
                .child(line.text.clone())
                .test_text(line.text.clone()),
        )
}

/// A line number column, blank when the line has no number on that side.
fn number(value: Option<u32>, look: Look) -> Div {
    div()
        .w(px(40.))
        .flex_shrink_0()
        .pr(px(8.))
        .text_right()
        .text_color(rgb(look.term_dim))
        .child(value.map(|n| n.to_string()).unwrap_or_default())
}

/// The new-side line number of a row, a button that comments on that line
/// (RV-20). A row with no new number is a plain blank cell.
fn new_number(value: Option<u32>, line: &DiffLine, commenting: &Rc<Commenting>, look: Look) -> AnyElement {
    let cell = number(value, look);
    let Some(n) = value else {
        return cell.into_any_element();
    };
    let commenting = commenting.clone();
    let text = line.text.clone();
    cell.id(SharedString::from(format!("editor-comment-{n}")))
        .role(Role::Button)
        .aria_label(format!("Comment on line {n}"))
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            commenting.click(n, &text, cx);
        })
        .test_support()
        .into_any_element()
}
