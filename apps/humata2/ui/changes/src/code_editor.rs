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
pub struct EditorView {
    /// The rows' scroll position.
    pub handle: ScrollHandle,
    /// Set by a step; the next frame that can scroll shows the cursor's change.
    pending: Rc<Cell<bool>>,
    /// The last terminal file-link jump generation that was scrolled into view.
    scrolled_jump: Rc<Cell<Option<u64>>>,
}

impl EditorView {
    pub fn new() -> Self {
        Self {
            handle: ScrollHandle::new(),
            pending: Rc::new(Cell::new(false)),
            scrolled_jump: Rc::new(Cell::new(None)),
        }
    }
}

/// The diff of a file the session changed, if it has changes to show.
pub fn diff<'a>(
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
pub fn body(
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
    let view = documents.editor_view(&session.id, repo, path);
    let code = state
        .session
        .views
        .get(&session.id)
        .map(|page| page.code(repo, path))
        .unwrap_or_default();
    let cursor = state.session.change(&session.id, repo, path);
    let rows = editor::rows(Some(diff), text, &code);
    let file_lines: Rc<Vec<String>> = Rc::new(text.lines().map(str::to_owned).collect());
    let commenting = Rc::new(Commenting {
        ui: ui.clone(),
        session: session.id.clone(),
        repo: repo.to_owned(),
        path: path.to_owned(),
        lines: file_lines,
        reattach: documents.reattach(),
    });
    let highlights = LineHighlights::for_file(state, &session.id, repo, path, look);
    let dark = ((look.term >> 8) & 0xff) < 0x80;
    let language = humata2_highlight::Language::detect(Some(path), Some(text));
    let side_rows = (code.mode == CodeMode::SideBySide).then(|| editor::side_by_side(&rows));
    let (lines, starts, slots): (Vec<Vec<AnyElement>>, Vec<usize>, Vec<Slot>) = match &side_rows {
        Some(side) => {
            let old_texts: Vec<&str> = side
                .iter()
                .map(|row| match row {
                    SideRow::Pair { old: Some(line), .. } => line.text.as_str(),
                    _ => "",
                })
                .collect();
            let new_texts: Vec<&str> = side
                .iter()
                .map(|row| match row {
                    SideRow::Pair { new: Some(line), .. } => line.text.as_str(),
                    _ => "",
                })
                .collect();
            let old_syntax = humata2_highlight::highlight_lines_cached(language, &old_texts, dark);
            let new_syntax = humata2_highlight::highlight_lines_cached(language, &new_texts, dark);
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
                        let old_spans = old_syntax.line(index);
                        let new_spans = new_syntax.line(index);
                        vec![side_pair(
                            index,
                            old.as_ref(),
                            old_spans,
                            new.as_ref(),
                            new_spans,
                            &commenting,
                            &highlights,
                            look,
                        )]
                    }
                    SideRow::Fold { start, count } => {
                        vec![fold(ui, repo, path, *start, *count, look)]
                    }
                })
                .collect();
            (lines, starts, slots)
        }
        None => {
            let line_texts: Vec<&str> = rows
                .iter()
                .map(|row| match row {
                    Row::Line(line) => line.text.as_str(),
                    Row::Fold { .. } => "",
                })
                .collect();
            let syntax = humata2_highlight::highlight_lines_cached(language, &line_texts, dark);
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
                    Row::Line(line) => {
                        let spans = syntax.line(index);
                        vec![line_row(index, line, spans, &commenting, &highlights, look)]
                    }
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
    let jump = state.session.file_jump(&session.id, repo, path);
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
    } else if let Some(jump) = jump
        && view.scrolled_jump.get() != Some(jump.generation)
        && let Some(index) = slot_of(&slots, Side::New, jump.first)
        && reveal(&view.handle, index)
    {
        view.scrolled_jump.set(Some(jump.generation));
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

    let clear_jump_click = jump.map(|_| {
        let ui = ui.clone();
        let session = session.id.clone();
        let repo = repo.to_owned();
        let path = path.to_owned();
        move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            ui.send(
                AppAction::Session(SessionAction::ClearFileJump {
                    session: session.clone(),
                    repo: repo.clone(),
                    path: path.clone(),
                    generation: None,
                }),
                cx,
            );
        }
    });
    let clear_jump_scroll = jump.map(|_| {
        let ui = ui.clone();
        let session = session.id.clone();
        let repo = repo.to_owned();
        let path = path.to_owned();
        move |_: &ScrollEvent, _: &mut Window, cx: &mut App| {
            ui.send(
                AppAction::Session(SessionAction::ClearFileJump {
                    session: session.clone(),
                    repo: repo.clone(),
                    path: path.clone(),
                    generation: None,
                }),
                cx,
            );
        }
    });

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
                .when_some(clear_jump_click, |rows, handler| {
                    rows.on_mouse_down(MouseButton::Left, handler)
                })
                .when_some(clear_jump_scroll, |rows, handler| {
                    rows.on_scroll(handler)
                })
                .children(lines),
        )
        .when(sendable > 0, |column| {
            column.child(drafts(ui, &session.id, sendable, look))
        })
        .test_support()
        .into_any_element()
}

/// A normal (unchanged) code file in a session tab, with syntax highlighting
/// and full single-line & multi-line commenting (click, Shift+click, or drag
/// across line numbers).
#[allow(clippy::too_many_arguments)]
pub fn plain_body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    text: &str,
    documents: &Documents,
    look: Look,
) -> AnyElement {
    let view = documents.editor_view(&session.id, repo, path);
    let jump = state.session.file_jump(&session.id, repo, path);
    let display_path = if repo.is_empty() {
        path.to_owned()
    } else {
        format!("{repo}/{path}")
    };
    let file_lines: Rc<Vec<String>> = Rc::new(text.lines().map(str::to_owned).collect());
    let commenting = Rc::new(Commenting {
        ui: ui.clone(),
        session: session.id.clone(),
        repo: repo.to_owned(),
        path: path.to_owned(),
        lines: file_lines.clone(),
        reattach: documents.reattach(),
    });
    let highlights = LineHighlights::for_file(state, &session.id, repo, path, look);
    let dark = ((look.term >> 8) & 0xff) < 0x80;
    let language = humata2_highlight::Language::detect(Some(path), Some(text));
    let line_refs: Vec<&str> = file_lines.iter().map(String::as_str).collect();
    let syntax = humata2_highlight::highlight_lines_cached(language, &line_refs, dark);
    let diff_lines: Vec<DiffLine> = file_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let n = u32::try_from(index + 1).unwrap_or(u32::MAX);
            DiffLine {
                kind: LineKind::Context,
                old: Some(n),
                new: Some(n),
                text: line.clone(),
            }
        })
        .collect();
    let slots: Vec<Slot> = diff_lines.iter().map(|line| Slot::Lines(vec![line])).collect();
    let under = comments_under(ui, state, session, repo, path, &slots, documents, look);
    let lines: Vec<AnyElement> = diff_lines
        .iter()
        .enumerate()
        .zip(under)
        .flat_map(|((index, line), cards)| {
            let spans = syntax.line(index);
            let mut elements = vec![plain_line_row(index, line, spans, &commenting, &highlights, look)];
            elements.extend(cards);
            elements
        })
        .collect();
    let sendable = state.comments.sendable(&session.id).len();
    if let Some(jump) = jump
        && view.scrolled_jump.get() != Some(jump.generation)
        && let Some(index) = slot_of(&slots, Side::New, jump.first)
        && reveal(&view.handle, index)
    {
        view.scrolled_jump.set(Some(jump.generation));
    }
    let clear_jump_click = jump.map(|_| {
        let ui = ui.clone();
        let session = session.id.clone();
        let repo = repo.to_owned();
        let path = path.to_owned();
        move |_: &MouseDownEvent, _: &mut Window, cx: &mut App| {
            ui.send(
                AppAction::Session(SessionAction::ClearFileJump {
                    session: session.clone(),
                    repo: repo.clone(),
                    path: path.clone(),
                    generation: None,
                }),
                cx,
            );
        }
    });
    let clear_jump_scroll = jump.map(|_| {
        let ui = ui.clone();
        let session = session.id.clone();
        let repo = repo.to_owned();
        let path = path.to_owned();
        move |_: &ScrollEvent, _: &mut Window, cx: &mut App| {
            ui.send(
                AppAction::Session(SessionAction::ClearFileJump {
                    session: session.clone(),
                    repo: repo.clone(),
                    path: path.clone(),
                    generation: None,
                }),
                cx,
            );
        }
    });

    col()
        .id("session-file")
        .role(Role::Document)
        .aria_label(display_path.clone())
        .flex_1()
        .min_h_0()
        .bg(rgb(look.term))
        .text_color(rgb(look.text))
        .child(
            div()
                .px(px(14.))
                .py(px(8.))
                .flex_shrink_0()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(display_path.clone()),
        )
        .child(
            col()
                .id("editor-rows")
                .role(Role::Code)
                .aria_label(display_path)
                .test_support()
                .flex_1()
                .min_h_0()
                .py(px(4.))
                .font_family("Menlo")
                .text_size(px(12.))
                .line_height(px(18.))
                .track_scroll(&view.handle)
                .overflow_y_scroll()
                .when_some(clear_jump_click, |rows, handler| {
                    rows.on_mouse_down(MouseButton::Left, handler)
                })
                .when_some(clear_jump_scroll, |rows, handler| {
                    rows.on_scroll(handler)
                })
                .children(lines),
        )
        .when(sendable > 0, |column| {
            column.child(drafts(ui, &session.id, sendable, look))
        })
        .test_support()
        .into_any_element()
}

/// Active gutter drag state for selecting a multi-line comment range in the
/// code editor / plain file view.
#[derive(Clone, Debug, PartialEq, Eq)]
struct EditorDrag {
    session: SessionId,
    repo: String,
    path: String,
    start: u32,
    last: u32,
}

thread_local! {
    static EDITOR_DRAG: RefCell<Option<EditorDrag>> = const { RefCell::new(None) };
}

fn clear_editor_drag() {
    EDITOR_DRAG.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Quotes 1-based inclusive `first..=last` from `lines`, normalizing order.
fn quote_lines(lines: &[String], first: u32, last: u32) -> Vec<String> {
    let (lo, hi) = Anchor::normalized_range(first, last);
    if lo == 0 {
        return Vec::new();
    }
    let start = (lo as usize).saturating_sub(1).min(lines.len());
    let end = (hi as usize).min(lines.len());
    lines[start..end].to_vec()
}

/// Highlight state for lines in the code editor: active open box vs saved
/// multi-line comments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineHighlight {
    Active,
    Comment,
}

struct LineHighlights {
    editing: Option<(Side, u32, u32)>,
    jump: Option<(u32, u32)>,
    comments: Vec<(Side, u32, u32)>,
    lit_color: u32,
    comment_lit_color: u32,
}

impl LineHighlights {
    fn for_file(
        state: &AppState,
        session: &SessionId,
        repo: &str,
        path: &str,
        look: Look,
    ) -> Self {
        let editing = state
            .comments
            .sessions
            .get(session)
            .and_then(|held| held.editing.as_ref())
            .filter(|open| open.anchor.repo == repo && open.anchor.path == path);
        let box_comment = editing.and_then(|open| open.comment.clone());
        let editing_span = editing.map(|open| {
            let (first, last) = open.anchor.line_range();
            (open.anchor.side, first, last)
        });
        let jump = state
            .session
            .file_jump(session, repo, path)
            .map(|jump| (jump.first, jump.last));
        let comments = state
            .comments
            .on_file(session, repo, path)
            .into_values()
            .flatten()
            .filter(|comment| {
                Some(&comment.id) != box_comment.as_ref()
                    && !comment.anchor.detached
                    && comment.anchor.is_multi_line()
            })
            .map(|comment| {
                let (first, last) = comment.anchor.line_range();
                (comment.anchor.side, first, last)
            })
            .collect();
        let dark = ((look.term >> 8) & 0xff) < 0x80;
        let lit_color = if look.term == 0xffffff { 0xeaf2ff } else { 0x1c2a3f };
        let comment_lit_color = blend_color(look.term, look.accent, if dark { 0.12 } else { 0.06 });
        Self {
            editing: editing_span,
            jump,
            comments,
            lit_color,
            comment_lit_color,
        }
    }

    fn for_line(&self, line: &DiffLine, side_filter: Option<Side>) -> Option<LineHighlight> {
        let matches_span = |side: Side, first: u32, last: u32| -> bool {
            if side_filter.is_some_and(|wanted| wanted != side) {
                return false;
            }
            let number = match side {
                Side::Old if line.kind != LineKind::Added => line.old,
                Side::New if line.kind != LineKind::Removed => line.new,
                _ => None,
            };
            number.is_some_and(|n| first > 0 && (first..=last).contains(&n))
        };
        if let Some((side, first, last)) = self.editing
            && matches_span(side, first, last)
        {
            return Some(LineHighlight::Active);
        }
        if let Some((first, last)) = self.jump
            && matches_span(Side::New, first, last)
        {
            return Some(LineHighlight::Active);
        }
        if self
            .comments
            .iter()
            .any(|&(side, first, last)| matches_span(side, first, last))
        {
            return Some(LineHighlight::Comment);
        }
        None
    }

    fn bg(&self, highlight: Option<LineHighlight>) -> Option<u32> {
        match highlight {
            Some(LineHighlight::Active) => Some(self.lit_color),
            Some(LineHighlight::Comment) => Some(self.comment_lit_color),
            None => None,
        }
    }
}

fn blend_color(under: u32, over: u32, alpha: f32) -> u32 {
    let channel = |shift: u32| {
        let below = ((under >> shift) & 0xff) as f32;
        let above = ((over >> shift) & 0xff) as f32;
        ((below + (above - below) * alpha).round().clamp(0., 255.)) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

/// Where the editor's line numbers send a comment: the session, the file, its
/// lines for quoting, and the cell a detached comment waits in to be
/// re-attached (shared with the Diff panel's line numbers, so either view can
/// take the click).
struct Commenting {
    ui: Rc<Ui>,
    session: SessionId,
    repo: String,
    path: String,
    lines: Rc<Vec<String>>,
    reattach: Rc<RefCell<Option<CommentId>>>,
}

impl Commenting {
    fn anchor(&self, first: u32, last: u32) -> Anchor {
        let (first, last) = Anchor::normalized_range(first, last);
        Anchor {
            repo: self.repo.clone(),
            path: self.path.clone(),
            side: Side::New,
            first,
            last,
            quoted: quote_lines(&self.lines, first, last),
            base: None,
            detached: false,
            outdated: false,
        }
    }

    /// A new-side line number was clicked. A detached comment waiting for a
    /// line takes it; otherwise the comment box opens on that line, or with
    /// `extend` (Shift+click) stretches the open box on this file to `number`.
    fn click(&self, number: u32, extend: bool, cx: &mut App) {
        let pending = self.reattach.borrow_mut().take();
        if let Some(id) = pending {
            clear_editor_drag();
            self.ui.send(
                AppAction::Comments(CommentsAction::Reattach(
                    self.session.clone(),
                    id,
                    self.anchor(number, number),
                )),
                cx,
            );
            return;
        }
        let (first, last) = if extend {
            clear_editor_drag();
            self.ui.read(|state| {
                state
                    .comments
                    .extend_box_range(&self.session, &self.repo, &self.path, Side::New, number)
            })
        } else {
            EDITOR_DRAG.with(|cell| {
                *cell.borrow_mut() = Some(EditorDrag {
                    session: self.session.clone(),
                    repo: self.repo.clone(),
                    path: self.path.clone(),
                    start: number,
                    last: number,
                });
            });
            (number, number)
        };
        self.ui.send(
            AppAction::Comments(CommentsAction::OpenBox(
                self.session.clone(),
                self.anchor(first, last),
            )),
            cx,
        );
    }

    /// Extends an active gutter drag to `number`.
    fn drag_over(&self, number: u32, cx: &mut App) {
        let update = EDITOR_DRAG.with(|cell| {
            let mut slot = cell.borrow_mut();
            let drag = slot.as_mut()?;
            if drag.session != self.session || drag.repo != self.repo || drag.path != self.path {
                return None;
            }
            if number == drag.last {
                return None;
            }
            drag.last = number;
            Some(Anchor::normalized_range(drag.start, number))
        });
        if let Some((first, last)) = update {
            self.ui.send(
                AppAction::Comments(CommentsAction::OpenBox(
                    self.session.clone(),
                    self.anchor(first, last),
                )),
                cx,
            );
        }
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
                        session.pull_request_review,
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

/// A line of a normal (unchanged) code file: one clickable line number and text.
fn plain_line_row(
    index: usize,
    line: &DiffLine,
    syntax: &[(std::ops::Range<usize>, HighlightStyle)],
    commenting: &Rc<Commenting>,
    highlights: &LineHighlights,
    look: Look,
) -> AnyElement {
    let hl = highlights.for_line(line, Some(Side::New));
    let bg = highlights.bg(hl);
    let text_cell = div()
        .whitespace_nowrap()
        .text_color(rgb(look.term_text));
    let text_cell = if syntax.is_empty() {
        text_cell.child(line.text.clone())
    } else {
        text_cell.child(StyledText::new(line.text.clone()).with_highlights(syntax.to_vec()))
    };
    row()
        .id(SharedString::from(format!("editor-line-{index}")))
        .items_start()
        .w_full()
        .min_h(px(18.))
        .when_some(bg, |row, bg| row.bg(rgb(bg)))
        .child(new_number(line.new, commenting, hl.is_some(), look))
        .child(div().w(px(8.)).flex_shrink_0())
        .child(text_cell.test_text(line.text.clone()))
        .test_support()
        .into_any_element()
}

/// A line of the inline and full-file rows: both numbers, the sign, the text.
fn line_row(
    index: usize,
    line: &DiffLine,
    syntax: &[(std::ops::Range<usize>, HighlightStyle)],
    commenting: &Rc<Commenting>,
    highlights: &LineHighlights,
    look: Look,
) -> AnyElement {
    let (sign, diff_bg) = match line.kind {
        LineKind::Added => ("+", Some(look.add)),
        LineKind::Removed => ("−", Some(look.del)),
        _ => (" ", None),
    };
    let hl = highlights.for_line(line, None);
    let bg = highlights.bg(hl).or(diff_bg);
    let text_cell = div()
        .whitespace_nowrap()
        .text_color(rgb(look.term_text));
    let text_cell = if syntax.is_empty() {
        text_cell.child(line.text.clone())
    } else {
        text_cell.child(StyledText::new(line.text.clone()).with_highlights(syntax.to_vec()))
    };
    row()
        .id(SharedString::from(format!("editor-line-{index}")))
        .items_start()
        .w_full()
        .min_h(px(18.))
        .when_some(bg, |row, bg| row.bg(rgb(bg)))
        .child(number(line.old, hl.is_some(), look))
        .child(new_number(line.new, commenting, false, look))
        .child(
            div()
                .w(px(16.))
                .flex_shrink_0()
                .text_color(rgb(look.term_text))
                .child(sign),
        )
        .child(text_cell.test_text(line.text.clone()))
        .test_support()
        .into_any_element()
}

/// A pair of the side-by-side rows: the old line left, the new line right.
#[allow(clippy::too_many_arguments)]
fn side_pair(
    index: usize,
    old: Option<&DiffLine>,
    old_syntax: &[(std::ops::Range<usize>, HighlightStyle)],
    new: Option<&DiffLine>,
    new_syntax: &[(std::ops::Range<usize>, HighlightStyle)],
    commenting: &Rc<Commenting>,
    highlights: &LineHighlights,
    look: Look,
) -> AnyElement {
    row()
        .id(SharedString::from(format!("editor-row-{index}")))
        .items_stretch()
        .w_full()
        .child(half(old, old_syntax, true, commenting, highlights, look))
        .child(div().w(px(1.)).flex_shrink_0().bg(rgb(look.line)))
        .child(half(new, new_syntax, false, commenting, highlights, look))
        .test_support()
        .into_any_element()
}

/// One side of a pair. An empty side is a muted cell standing in for hatching.
/// Only the new side's numbers take a comment.
fn half(
    line: Option<&DiffLine>,
    syntax: &[(std::ops::Range<usize>, HighlightStyle)],
    old_side: bool,
    commenting: &Rc<Commenting>,
    highlights: &LineHighlights,
    look: Look,
) -> Div {
    let Some(line) = line else {
        return div().flex_1().min_w_0().min_h(px(18.)).bg(rgb(look.fill));
    };
    let (shown, diff_bg) = match (line.kind, old_side) {
        (LineKind::Removed, true) => (line.old, Some(look.del)),
        (LineKind::Added, false) => (line.new, Some(look.add)),
        (_, true) => (line.old, None),
        (_, false) => (line.new, None),
    };
    let side = if old_side { Side::Old } else { Side::New };
    let hl = highlights.for_line(line, Some(side));
    let bg = highlights.bg(hl).or(diff_bg);
    let gutter = if old_side {
        number(shown, hl.is_some(), look).into_any_element()
    } else {
        new_number(shown, commenting, hl.is_some(), look)
    };
    let text_cell = div()
        .flex_1()
        .min_w_0()
        .whitespace_nowrap()
        .text_color(rgb(look.term_text));
    let text_cell = if syntax.is_empty() {
        text_cell.child(line.text.clone())
    } else {
        text_cell.child(StyledText::new(line.text.clone()).with_highlights(syntax.to_vec()))
    };
    row()
        .flex_1()
        .min_w_0()
        .min_h(px(18.))
        .when_some(bg, |row, bg| row.bg(rgb(bg)))
        .child(gutter)
        .child(text_cell.test_text(line.text.clone()))
}

/// A line number column, blank when the line has no number on that side.
fn number(value: Option<u32>, marked: bool, look: Look) -> Div {
    div()
        .w(px(40.))
        .flex_shrink_0()
        .pr(px(8.))
        .text_right()
        .text_color(rgb(look.term_dim))
        .when(marked, |cell| cell.border_l_2().border_color(rgb(look.accent)))
        .child(value.map(|n| n.to_string()).unwrap_or_default())
}

/// The new-side line number of a row, a button that comments on that line
/// (RV-20). Supports click, Shift+click, and click-and-drag across line
/// numbers for multi-line comments. A row with no new number is a plain blank
/// cell.
fn new_number(
    value: Option<u32>,
    commenting: &Rc<Commenting>,
    marked: bool,
    look: Look,
) -> AnyElement {
    let cell = number(value, marked, look);
    let Some(n) = value else {
        return cell.into_any_element();
    };
    let commenting_down = commenting.clone();
    let commenting_move = commenting.clone();
    cell.id(SharedString::from(format!("editor-comment-{n}")))
        .role(Role::Button)
        .aria_label(format!("Comment on line {n}"))
        .cursor_pointer()
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
            cx.stop_propagation();
            commenting_down.click(n, event.modifiers.shift, cx);
        })
        .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
            if event.pressed_button == Some(MouseButton::Left) {
                commenting_move.drag_over(n, cx);
            } else {
                clear_editor_drag();
            }
        })
        .on_mouse_up(MouseButton::Left, move |_, _, _| {
            clear_editor_drag();
        })
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;

    #[test]
    fn quote_lines_extracts_normalized_multi_line_passages() {
        let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        assert_eq!(quote_lines(&lines, 10, 10), vec!["line 10"]);
        assert_eq!(quote_lines(&lines, 10, 12), vec!["line 10", "line 11", "line 12"]);
        // Reverse order (e.g. line 18 Shift+click line 15) normalizes cleanly.
        assert_eq!(
            quote_lines(&lines, 18, 15),
            vec!["line 15", "line 16", "line 17", "line 18"]
        );
        // Backward-compatible single-line with last == 0.
        assert_eq!(quote_lines(&lines, 7, 0), vec!["line 7"]);
        assert!(quote_lines(&lines, 0, 0).is_empty());
    }

    #[test]
    fn line_highlights_mark_active_selection_and_saved_multi_line_comments() {
        let highlights = LineHighlights {
            editing: Some((Side::New, 10, 14)),
            jump: Some((30, 32)),
            comments: vec![(Side::New, 20, 24)],
            lit_color: 0xeaf2ff,
            comment_lit_color: 0xf4f8ff,
        };
        let line = |n: u32| DiffLine {
            kind: LineKind::Context,
            old: Some(n),
            new: Some(n),
            text: format!("code {n}"),
        };

        assert_eq!(highlights.for_line(&line(9), Some(Side::New)), None);
        for n in 10..=14 {
            assert_eq!(
                highlights.for_line(&line(n), Some(Side::New)),
                Some(LineHighlight::Active),
                "line {n}"
            );
        }
        assert_eq!(highlights.for_line(&line(15), Some(Side::New)), None);
        for n in 20..=24 {
            assert_eq!(
                highlights.for_line(&line(n), Some(Side::New)),
                Some(LineHighlight::Comment),
                "line {n}"
            );
        }
        assert_eq!(highlights.for_line(&line(29), Some(Side::New)), None);
        for n in 30..=32 {
            assert_eq!(
                highlights.for_line(&line(n), Some(Side::New)),
                Some(LineHighlight::Active),
                "jump line {n}"
            );
        }
        assert_eq!(highlights.for_line(&line(33), Some(Side::New)), None);
        // Old side filter does not match New-side highlights.
        assert_eq!(highlights.for_line(&line(12), Some(Side::Old)), None);
        assert_eq!(highlights.for_line(&line(22), Some(Side::Old)), None);
        assert_eq!(highlights.for_line(&line(31), Some(Side::Old)), None);
    }
}
