use super::*;
use crate::comment_cards::{self, StartReattach};
use humata2_clients::comments::{Anchor, CommentId, Side};
use humata2_clients::model::DiffLine;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// DS-15: every comment card is at most 720 pt wide.
fn sent_card(look: Look, comment: &Comment) -> Div {
    card(look)
        .max_w(px(720.))
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .gap(px(4.))
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .child(row().gap(px(6.)).child(text("You").font_weight(FontWeight::SEMIBOLD)))
        .child(text(comment.body.clone()))
}

/// What the code rows need to draw, besides the data.
#[derive(Clone, Copy)]
pub(super) struct Metrics {
    pub(super) look: Look,
    pub(super) columns: usize,
    pub(super) gutter_width: f32,
    pub(super) cell_width: f32,
    pub(super) line_height: f32,
    /// The code view's visible width: on Scroll the rows are wider, but a
    /// comment stays inside what can be seen.
    pub(super) view_width: f32,
    /// A pull request review's inline code (frame 68): the numbers keep a
    /// 6 pt gap before the sign, and its cards keep frame 68's place.
    pub(super) review: bool,
    /// Two number columns, old then new (DS-19): full screen only.
    pub(super) old: bool,
}

impl Metrics {
    /// How far in from the row's left edge the code starts.
    fn code_start(&self) -> f32 {
        code_start_of(self.gutter_width, self.cell_width, self.old)
    }
}

/// Where the code starts in a row (DS-19, DS-20): the number column, or two,
/// then the sign's cell and one space after it, in a review as in the
/// Diff panel (the review mockup's 14 pt sign column).
pub(super) fn code_start_of(gutter_width: f32, cell_width: f32, old: bool) -> f32 {
    let one = (gutter_width - 8.).max(0.);
    let numbers = if old { one * 2. } else { one };
    numbers + cell_width * 2.
}

/// The side a line's comments are on and its number there: a removed line
/// is on the old side of the file, every other line on the new side.
/// Whether `session` reviews a pull request (PR-19: its hunks and files
/// offer Ask).
pub(super) fn reviewing(state: &AppState, session: &SessionId) -> bool {
    state
        .workspace
        .session(session)
        .is_some_and(|found| found.pull_request_review)
}

/// The lines `lines` covers, as Ask on a hunk or a whole file points at
/// them: the new side's first and last number where it has any, else the
/// old side's (a hunk that only removes).
pub(super) fn span(lines: &[DiffLine]) -> Option<(Side, u32, u32)> {
    [Side::New, Side::Old].into_iter().find_map(|side| {
        lines
            .iter()
            .filter(|line| is_code(line))
            .filter_map(|line| match (side, line.kind) {
                (Side::New, LineKind::Removed) | (Side::Old, LineKind::Added) => None,
                (Side::New, _) => line.new,
                (Side::Old, _) => line.old,
            })
            .fold(None, |held: Option<(u32, u32)>, n| {
                Some(held.map_or((n, n), |(first, last)| (first.min(n), last.max(n))))
            })
            .map(|(first, last)| (side, first, last))
    })
}

/// The lines of the hunk whose header is at `header`: up to the next header.
fn hunk_lines(lines: &[DiffLine], header: usize) -> &[DiffLine] {
    let rest = lines.get(header + 1..).unwrap_or_default();
    let end = rest
        .iter()
        .position(|line| line.kind == LineKind::Hunk)
        .unwrap_or(rest.len());
    &rest[..end]
}

/// PR-19: the Ask a review's file row or hunk header shows on hover. It picks
/// the file and opens the comment box over `anchor`; the box's Ask then
/// types the question into the review chat.
pub(super) fn ask_over(
    ui: &Rc<Ui>,
    look: Look,
    id: SharedString,
    group: SharedString,
    session: SessionId,
    anchor: Anchor,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label("Ask about this")
        .absolute()
        .top(px(2.))
        .right(px(4.))
        .px(px(6.))
        .rounded(px(4.))
        .bg(rgb(look.fill))
        .text_size(px(11.))
        .text_color(rgb(look.accent))
        .cursor_pointer()
        .opacity(0.)
        .group_hover(group, |style| style.opacity(1.))
        .focus_visible(|style| style.opacity(1.))
        .child("Ask")
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(
                AppAction::Review(ReviewAction::SelectFile {
                    repo: anchor.repo.clone(),
                    path: anchor.path.clone(),
                }),
                cx,
            );
            ui.send(
                AppAction::Comments(CommentsAction::OpenBox(session.clone(), anchor.clone())),
                cx,
            );
        })
        .test_support()
}

fn side_of(line: &DiffLine) -> (Side, Option<u32>) {
    if line.kind == LineKind::Removed {
        (Side::Old, line.old)
    } else {
        (Side::New, line.new)
    }
}

/// Whether `line` is on `side` and numbered `number` there. A context line
/// has both numbers, so an old-side comment can sit on it too.
fn at(line: &DiffLine, side: Side, number: u32) -> bool {
    match side {
        Side::Old => line.kind != LineKind::Added && line.old == Some(number),
        Side::New => line.kind != LineKind::Removed && line.new == Some(number),
    }
}

/// Whether `line` is on `side` and numbered within `first..=last` there.
fn within_span(line: &DiffLine, side: Side, first: u32, last: u32) -> bool {
    let number = match side {
        Side::Old if line.kind != LineKind::Added => line.old,
        Side::New if line.kind != LineKind::Removed => line.new,
        _ => None,
    };
    number.is_some_and(|n| (first..=last).contains(&n))
}

/// The index of the line a comment or box ending at `last` on `side` goes
/// under: that line, or the last code line when it is not in the hunks shown.
fn under(lines: &[DiffLine], side: Side, last: u32) -> Option<usize> {
    lines
        .iter()
        .position(|line| is_code(line) && at(line, side, last))
        .or_else(|| lines.iter().rposition(is_code))
}

/// The index of the line numbered `last` on `side`, when it is shown.
fn exactly_on(lines: &[DiffLine], side: Side, last: u32) -> Option<usize> {
    lines
        .iter()
        .position(|line| is_code(line) && at(line, side, last))
}

/// Everything a click on a line number needs. The file's lines are looked up
/// when they're needed, never copied into the target.
struct Target {
    ui: Rc<Ui>,
    session: SessionId,
    repo: String,
    path: String,
    base: String,
    reattach: Rc<RefCell<Option<CommentId>>>,
}

/// The lines of `repo`'s `path` in `session`'s diff, as the store holds them.
pub(super) fn file_lines<'a>(state: &'a AppState, session: &SessionId, repo: &str, path: &str) -> Option<&'a [DiffLine]> {
    state
        .workspace
        .diffs
        .iter()
        .filter(|diff| &diff.session == session && diff.repo == repo)
        .flat_map(|diff| &diff.files)
        .find(|file| file.path == path)
        .map(|file| file.lines.as_slice())
}

/// Active gutter drag state for selecting a multi-line comment range in the
/// diff view. GPUI runs on a single UI thread, so a thread-local cell keeps
/// drag state across per-frame `CodeRows` rebuilds.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DiffDrag {
    session: SessionId,
    repo: String,
    path: String,
    side: Side,
    start: u32,
    last: u32,
}

/// Default step size for in-place hunk gap expansion.
pub(crate) const GAP_EXPAND_STEP: u32 = 20;

/// Hidden context gap before a diff hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HunkGap {
    pub(crate) gap_index: usize,
    pub(crate) prev_new_end: u32,
    pub(crate) next_new_start: u32,
    pub(crate) hidden_lines: u32,
}

type GapKey = (SessionId, String, String, usize);
type FileViewKey = (SessionId, String, String);

/// Version shown in read-only full-file view mode (`Open file ↗`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileVersion {
    Head,
    Base,
}

/// Per-file read-only view mode state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileViewState {
    pub(crate) open: bool,
    pub(crate) version: FileVersion,
}

impl Default for FileViewState {
    fn default() -> Self {
        Self {
            open: false,
            version: FileVersion::Head,
        }
    }
}

/// Kind of change marker on the read-only file scrollbar rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MarkerKind {
    Added,
    Removed,
    Modified,
}

/// Proportional position (`0.0..=1.0`) of a change marker on the scrollbar rail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollbarMarker {
    pub(crate) fraction: f32,
    pub(crate) kind: MarkerKind,
}

thread_local! {
    static DIFF_DRAG: RefCell<Option<DiffDrag>> = const { RefCell::new(None) };
    static EXPANDED_GAPS: RefCell<HashMap<GapKey, u32>> = RefCell::new(HashMap::new());
    static FILE_VIEW_MODES: RefCell<HashMap<FileViewKey, FileViewState>> = RefCell::new(HashMap::new());
}

pub(crate) fn expand_gap_step(current_expanded: u32, total_hidden: u32, step: u32) -> u32 {
    current_expanded.saturating_add(step).min(total_hidden)
}

pub(crate) fn expand_gap_all(total_hidden: u32) -> u32 {
    total_hidden
}

pub(crate) fn remaining_hidden_lines(total_hidden: u32, expanded: u32) -> u32 {
    total_hidden.saturating_sub(expanded)
}

pub(crate) fn gap_label(remaining: u32, is_top: bool) -> String {
    let noun = if remaining == 1 { "line" } else { "lines" };
    if is_top {
        format!("{remaining} {noun} hidden above")
    } else {
        format!("{remaining} hidden {noun}")
    }
}

/// Computes the hidden context gap immediately before the hunk header at `hunk_index`.
pub(crate) fn compute_hunk_gap(lines: &[DiffLine], hunk_index: usize, gap_index: usize) -> Option<HunkGap> {
    if lines.get(hunk_index)?.kind != LineKind::Hunk {
        return None;
    }
    let next_new_start = lines
        .get(hunk_index + 1..)?
        .iter()
        .take_while(|l| l.kind != LineKind::Hunk)
        .filter(|l| is_code(l))
        .find_map(|l| l.new.or(l.old))?;
    let prev_new_end = lines
        .get(..hunk_index)?
        .iter()
        .rev()
        .filter(|l| is_code(l))
        .find_map(|l| l.new.or(l.old))
        .unwrap_or(0);
    if next_new_start <= prev_new_end.saturating_add(1) {
        return None;
    }
    let hidden_lines = next_new_start - prev_new_end - 1;
    Some(HunkGap {
        gap_index,
        prev_new_end,
        next_new_start,
        hidden_lines,
    })
}

fn gap_expanded_count(session: &SessionId, repo: &str, path: &str, gap_index: usize) -> u32 {
    EXPANDED_GAPS.with(|cell| {
        cell.borrow()
            .get(&(session.clone(), repo.to_owned(), path.to_owned(), gap_index))
            .copied()
            .unwrap_or(0)
    })
}

fn set_gap_expanded_count(session: SessionId, repo: String, path: String, gap_index: usize, count: u32) {
    EXPANDED_GAPS.with(|cell| {
        cell.borrow_mut().insert((session, repo, path, gap_index), count);
    });
}

/// Returns the current read-only file view state for `(session, repo, path)`.
pub(super) fn file_view_state(session: &SessionId, repo: &str, path: &str) -> FileViewState {
    FILE_VIEW_MODES.with(|cell| {
        cell.borrow()
            .get(&(session.clone(), repo.to_owned(), path.to_owned()))
            .copied()
            .unwrap_or_default()
    })
}

/// Opens or closes read-only file view mode for `(session, repo, path)`.
pub(super) fn set_file_view_open(session: SessionId, repo: String, path: String, open: bool) {
    FILE_VIEW_MODES.with(|cell| {
        let mut map = cell.borrow_mut();
        let entry = map.entry((session, repo, path)).or_default();
        entry.open = open;
    });
}

/// Switches between `Head` and `Base` in read-only file view mode.
pub(super) fn set_file_view_version(
    session: SessionId,
    repo: String,
    path: String,
    version: FileVersion,
) {
    FILE_VIEW_MODES.with(|cell| {
        let mut map = cell.borrow_mut();
        let entry = map.entry((session, repo, path)).or_default();
        entry.version = version;
    });
}

/// Extracts read-only file lines `(line_number, text, kind)` for `Head` or `Base`.
pub(crate) fn readonly_file_lines(lines: &[DiffLine], version: FileVersion) -> Vec<(u32, &str, LineKind)> {
    lines
        .iter()
        .filter(|line| is_code(line))
        .filter_map(|line| match version {
            FileVersion::Head => {
                if line.kind == LineKind::Removed {
                    None
                } else {
                    Some((line.new.unwrap_or(0), line.text.as_str(), line.kind))
                }
            }
            FileVersion::Base => {
                if line.kind == LineKind::Added {
                    None
                } else {
                    Some((line.old.unwrap_or(0), line.text.as_str(), line.kind))
                }
            }
        })
        .collect()
}

/// Computes proportional scrollbar marker positions for Added (green), Removed (red),
/// and Modified (amber) lines in `lines`.
pub(crate) fn compute_scrollbar_markers(lines: &[DiffLine]) -> Vec<ScrollbarMarker> {
    let code_indices: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| is_code(l))
        .map(|(ix, _)| ix)
        .collect();
    let total = code_indices.len();
    if total == 0 {
        return Vec::new();
    }
    let denom = total as f32;
    let mut markers = Vec::new();
    for (pos, &ix) in code_indices.iter().enumerate() {
        let line = &lines[ix];
        let kind = match line.kind {
            LineKind::Added => {
                if paired(lines, ix).is_some() {
                    MarkerKind::Modified
                } else {
                    MarkerKind::Added
                }
            }
            LineKind::Removed => {
                if paired(lines, ix).is_some() {
                    MarkerKind::Modified
                } else {
                    MarkerKind::Removed
                }
            }
            _ => continue,
        };
        markers.push(ScrollbarMarker {
            fraction: (pos as f32 / denom).clamp(0.0, 1.0),
            kind,
        });
    }
    markers
}

/// Renders the read-only full-file view with a slim vertical scrollbar marker rail.
pub(super) fn readonly_file_view(
    file: &humata2_clients::model::FileDiff,
    version: FileVersion,
    look: Look,
    font_size: f32,
    line_height: f32,
) -> AnyElement {
    let entries = readonly_file_lines(&file.lines, version);
    let markers = compute_scrollbar_markers(&file.lines);
    let rows: Vec<AnyElement> = entries
        .into_iter()
        .enumerate()
        .map(|(ix, (num, content, kind))| {
            let bg = tint(look, kind);
            row()
                .id(SharedString::from(format!("readonly-line-{ix}")))
                .w_full()
                .min_h(px(line_height))
                .when_some(bg, |r, c| r.bg(rgb(c)))
                .child(
                    text(if num > 0 { num.to_string() } else { String::new() })
                        .w(px(44.))
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .pr(px(8.))
                        .text_color(rgb(look.term_dim)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_nowrap()
                        .text_color(rgb(look.term_text))
                        .child(content.to_owned()),
                )
                .into_any_element()
        })
        .collect();

    let rail = div()
        .id("diff-file-marker-rail")
        .test_support()
        .relative()
        .w(px(8.))
        .h_full()
        .flex_shrink_0()
        .bg(rgb(look.panel))
        .border_l_1()
        .border_color(rgb(look.line))
        .children(markers.into_iter().enumerate().map(|(ix, marker)| {
            let color = match marker.kind {
                MarkerKind::Added => GREEN,
                MarkerKind::Removed => RED,
                MarkerKind::Modified => 0xd98200,
            };
            div()
                .id(SharedString::from(format!("diff-file-marker-{ix}")))
                .absolute()
                .top(relative(marker.fraction))
                .left(px(1.))
                .w(px(6.))
                .h(px(3.))
                .rounded(px(1.))
                .bg(rgb(color))
        }));

    row()
        .id("diff-readonly-file-view")
        .test_support()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .bg(rgb(look.term))
        .child(
            col()
                .id("diff-readonly-file-scroll")
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_y_scroll()
                .py(px(6.))
                .font_family("Menlo")
                .text_size(px(font_size))
                .line_height(px(line_height))
                .children(rows),
        )
        .child(rail)
        .into_any_element()
}

fn clear_diff_drag() {
    DIFF_DRAG.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

fn line_number_for_side(side: Side, old_line: Option<u32>, new_line: Option<u32>) -> Option<u32> {
    match side {
        Side::Old => old_line,
        Side::New => new_line,
    }
}

/// Computes `(side, first, last)` for clicking or Shift+clicking a diff row
/// when `editing` may hold an open box's anchor on `(repo, path)`.
fn resolve_diff_click(
    editing: Option<&Anchor>,
    repo: &str,
    path: &str,
    default_side: Side,
    old_line: Option<u32>,
    new_line: Option<u32>,
    extend: bool,
) -> Option<(Side, u32, u32)> {
    if extend
        && let Some(anchor) = editing.filter(|a| a.repo == repo && a.path == path)
        && let Some(n) = line_number_for_side(anchor.side, old_line, new_line)
    {
        let (first, last) = Anchor::extend_range(anchor.first, anchor.last, n);
        return Some((anchor.side, first, last));
    }
    let n = line_number_for_side(default_side, old_line, new_line)?;
    Some((default_side, n, n))
}

impl Target {
    fn anchor(&self, lines: &[DiffLine], side: Side, first: u32, last: u32) -> Anchor {
        let (first, last) = Anchor::normalized_range(first, last);
        let quoted = lines
            .iter()
            .filter(|line| is_code(line))
            .filter(|line| within_span(line, side, first, last))
            .map(|line| line.text.clone())
            .collect();
        Anchor {
            repo: self.repo.clone(),
            path: self.path.clone(),
            side,
            first,
            last,
            quoted,
            base: Some(self.base.clone()),
            detached: false,
            outdated: false,
        }
    }

    /// The anchor over `first..=last`, quoting the file's lines as the store
    /// holds them now.
    fn anchor_now(&self, side: Side, first: u32, last: u32) -> Anchor {
        self.ui.read(|state| {
            let lines = file_lines(state, &self.session, &self.repo, &self.path).unwrap_or_default();
            self.anchor(lines, side, first, last)
        })
    }

    /// A number was clicked: it takes a comment being re-attached, or opens
    /// the box on its line, or with `extend` stretches the open box to it
    /// when that box is on the same file and side (including context lines on
    /// `Side::Old`).
    fn click_line(
        &self,
        side: Side,
        old_line: Option<u32>,
        new_line: Option<u32>,
        extend: bool,
        cx: &mut App,
    ) {
        let pending = self.reattach.borrow_mut().take();
        if let Some(id) = pending {
            clear_diff_drag();
            let Some(line) = line_number_for_side(side, old_line, new_line) else {
                return;
            };
            self.ui.send(
                AppAction::Comments(CommentsAction::Reattach(
                    self.session.clone(),
                    id,
                    self.anchor_now(side, line, line),
                )),
                cx,
            );
            return;
        }
        let resolved = self.ui.read(|state| {
            let editing = state
                .comments
                .sessions
                .get(&self.session)
                .and_then(|held| held.editing.as_ref())
                .map(|b| &b.anchor);
            resolve_diff_click(editing, &self.repo, &self.path, side, old_line, new_line, extend)
        });
        let Some((resolved_side, first, last)) = resolved else {
            return;
        };
        if !extend {
            DIFF_DRAG.with(|cell| {
                *cell.borrow_mut() = Some(DiffDrag {
                    session: self.session.clone(),
                    repo: self.repo.clone(),
                    path: self.path.clone(),
                    side: resolved_side,
                    start: first,
                    last,
                });
            });
        } else {
            clear_diff_drag();
        }
        self.ui.send(
            AppAction::Comments(CommentsAction::OpenBox(
                self.session.clone(),
                self.anchor_now(resolved_side, first, last),
            )),
            cx,
        );
    }

    /// Extends an active gutter drag to the hovered line if it is on the same
    /// file and has a line number on the drag's side.
    fn drag_over(&self, old_line: Option<u32>, new_line: Option<u32>, cx: &mut App) {
        let update = DIFF_DRAG.with(|cell| {
            let mut slot = cell.borrow_mut();
            let drag = slot.as_mut()?;
            if drag.session != self.session || drag.repo != self.repo || drag.path != self.path {
                return None;
            }
            let n = line_number_for_side(drag.side, old_line, new_line)?;
            if n == drag.last {
                return None;
            }
            drag.last = n;
            let (first, last) = Anchor::normalized_range(drag.start, n);
            Some((drag.side, first, last))
        });
        if let Some((side, first, last)) = update {
            self.ui.send(
                AppAction::Comments(CommentsAction::OpenBox(
                    self.session.clone(),
                    self.anchor_now(side, first, last),
                )),
                cx,
            );
        }
    }
}

/// Files with more lines than this draw them in a virtual list (DV-5): only
/// the lines in view and a little past it are built each frame. Shorter
/// files draw every line, so each stays queryable and the frames match the
/// approved goldens.
pub(super) const VIRTUAL_FROM: usize = 1000;

/// Pull request review files with more lines than this draw in their own
/// bounded-height virtual list (DV-5 step 5, DS-29); shorter files keep
/// frame 68's inline blocks.
pub(super) const REVIEW_VIRTUAL_FROM: usize = 1500;

/// Everything one file's code rows need besides its lines, gathered once
/// per frame and owned, so a virtual list can build any line later in the
/// frame: the comments by line, the open box, the click target and the
/// agent's focus.
pub(super) struct CodeRows {
    pub(super) ui: Rc<Ui>,
    metrics: Metrics,
    wrap: bool,
    /// How far Scroll mode has moved the code column left, in pixels.
    shift: f32,
    agent: String,
    session: Option<SessionId>,
    reviewing: bool,
    editing: Option<humata2_review::comments::CommentBox>,
    offers: comment_cards::BoxOffers,
    box_at: Option<usize>,
    cards: HashMap<usize, Vec<DaemonComment>>,
    /// Every comment on the file is done: the line index of the fold, how
    /// many comments, its key and whether they are shown anyway.
    fold: Option<(usize, usize, String, bool)>,
    unfolded: comment_cards::Unfolded,
    sent: HashMap<usize, Vec<Comment>>,
    reattaching: Option<CommentId>,
    start: StartReattach,
    input: Entity<gpui_kit::component::input::TextareaState>,
    target: Option<Rc<Target>>,
    lit_color: u32,
    /// Subtle highlight for lines covered by a saved multi-line comment.
    comment_lit_color: u32,
    /// Multi-line spans `(side, first, last)` of visible non-detached comments.
    comment_spans: Vec<(Side, u32, u32)>,
    /// The agent's focus, and the index of the first line it is on.
    focus: Option<(Side, u32)>,
    pub(super) focus_index: Option<usize>,
    /// What a click on a line focuses, outside a review: the code view.
    code_focus: Option<FocusHandle>,
    /// Linked To-do context for Add to steps / Spin off To-do comment hooks.
    todo_ctx: comment_cards::TodoHookContext,
    /// Cached per-line syntax highlight spans for `file.lines`.
    syntax: std::sync::Arc<humata2_highlight::LineHighlights>,
}

impl CodeRows {
    /// The rows of `file`, with the line the agent focuses marked
    /// (`review-focus-<n>`, the accent border and a bold number).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        ui: &Rc<Ui>,
        state: &AppState,
        documents: &Documents,
        diff: &RepoDiff,
        file: &humata2_clients::model::FileDiff,
        metrics: Metrics,
        focus: Option<(Side, u32)>,
        shift: f32,
    ) -> Self {
        let look = metrics.look;
        let session = state
            .session
            .open
            .as_ref()
            .and_then(|id| state.workspace.session(id));
        let agent = session.map_or("the agent", |session| session.agent.command()).to_owned();
        let held = session.and_then(|session| state.comments.sessions.get(&session.id));
        let editing = held
            .and_then(|held| held.editing.as_ref())
            .filter(|b| b.anchor.repo == diff.repo && b.anchor.path == file.path);
        let box_comment = editing.and_then(|b| b.comment.clone());
        let cell = documents.reattach();
        let start: StartReattach = Rc::new(move |id, cx| {
            *cell.borrow_mut() = Some(id.clone());
            cx.refresh_windows();
        });
        let target = session.map(|session| {
            Rc::new(Target {
                ui: ui.clone(),
                session: session.id.clone(),
                repo: diff.repo.clone(),
                path: file.path.clone(),
                base: diff.base.clone(),
                reattach: documents.reattach(),
            })
        });

        // Where each comment, legacy card and the box goes, by line index.
        let mut cards: HashMap<usize, Vec<DaemonComment>> = HashMap::new();
        if let Some(session) = session {
            for comment in state
                .comments
                .on_file(&session.id, &diff.repo, &file.path)
                .into_values()
                .flatten()
            {
                if Some(&comment.id) == box_comment.as_ref() {
                    continue;
                }
                // A review's drafts for GitHub are listed in Submit review and
                // counted in the footer, so one whose line is in no hunk shown
                // waits for its hunk rather than sitting under another line
                // (frame 68).
                let index = if session.pull_request_review {
                    exactly_on(&file.lines, comment.anchor.side, comment.anchor.last)
                } else {
                    under(&file.lines, comment.anchor.side, comment.anchor.last)
                };
                if let Some(index) = index {
                    cards.entry(index).or_default().push(comment.clone());
                }
            }
        }
        let fold_key = format!("{}/{}", diff.repo, file.path);
        let unfolded = documents.unfolded();
        // Done comments fold only on a file the person ticked (4-3); an
        // unticked file keeps its done cards open (4-2).
        let ticked = humata2_review::looks::is_looked(&state.review, &diff.session, &diff.repo, &file.path, file);
        let fold = comment_cards::all_done(cards.values().flatten()).filter(|_| ticked).and_then(|count| {
            let at = cards.keys().min().copied()?;
            Some((at, count, fold_key.clone(), unfolded.borrow().contains(&fold_key)))
        });
        if matches!(fold, Some((_, _, _, false))) {
            cards.clear();
        }
        let mut sent: HashMap<usize, Vec<Comment>> = HashMap::new();
        for comment in diff
            .comments
            .iter()
            .filter(|comment| on_path(comment, diff, &file.path))
        {
            if let Some(index) = under(&file.lines, Side::New, comment.line) {
                sent.entry(index).or_default().push(comment.clone());
            }
        }
        let box_at = editing.and_then(|b| under(&file.lines, b.anchor.side, b.anchor.last));
        let offers = match (editing, session) {
            (Some(b), Some(session)) => {
                let offers = comment_cards::offers(state, &session.id, &b.anchor.repo);
                // A draft for GitHub that is being edited can't be drafted again.
                if held.is_some_and(|held| {
                    held.comments.iter().any(|c| {
                        Some(&c.id) == b.comment.as_ref()
                            && matches!(c.destination, humata2_clients::comments::Destination::PullRequest { .. })
                    })
                }) {
                    offers.without_github()
                } else {
                    offers
                }
            }
            _ => comment_cards::BoxOffers::default(),
        };
        let focus_index = focus.and_then(|(side, n)| {
            file.lines
                .iter()
                .position(|line| is_code(line) && at(line, side, n))
        });
        let todo_ctx = session
            .map(|session| comment_cards::todo_context(state, &session.id))
            .unwrap_or_default();
        let dark = ((look.term >> 8) & 0xff) < 0x80;
        let first_code = file
            .lines
            .iter()
            .find(|line| is_code(line))
            .map(|line| line.text.as_str());
        let language = humata2_highlight::Language::detect(Some(&file.path), first_code);
        let code_lines: Vec<&str> = file
            .lines
            .iter()
            .map(|line| if is_code(line) { line.text.as_str() } else { "" })
            .collect();
        let syntax = humata2_highlight::highlight_lines_cached(language, &code_lines, dark);
        let comment_spans: Vec<(Side, u32, u32)> = cards
            .values()
            .flatten()
            .filter(|comment| !comment.anchor.detached && comment.anchor.is_multi_line())
            .map(|comment| {
                let (first, last) = comment.anchor.line_range();
                (comment.anchor.side, first, last)
            })
            .collect();
        let comment_lit_color = blend(look.term, look.accent, if dark { 0.12 } else { 0.06 });
        Self {
            ui: ui.clone(),
            metrics,
            wrap: state.review.wrap,
            shift,
            agent,
            reviewing: session.is_some_and(|session| reviewing(state, &session.id)),
            session: session.map(|session| session.id.clone()),
            editing: editing.cloned(),
            offers,
            box_at,
            cards,
            fold,
            unfolded,
            sent,
            reattaching: documents.reattach().borrow().clone(),
            start,
            input: documents.input(),
            target,
            lit_color: if look.term == 0xffffff { 0xeaf2ff } else { 0x1c2a3f },
            comment_lit_color,
            comment_spans,
            focus,
            focus_index,
            code_focus: (!metrics.review).then(|| documents.diff_code.clone()),
            todo_ctx,
            syntax,
        }
    }

    /// One item: the line at `index` with the cards under it, as one block.
    pub(super) fn item(&self, lines: &[DiffLine], index: usize) -> AnyElement {
        let (line, cards) = self.line(lines, index);
        if cards.is_empty() {
            line
        } else {
            col().child(line).children(cards).into_any_element()
        }
    }

    /// A card kept inside what can be seen of the code view. Outside a
    /// review it starts at the code column, not under the numbers (DS-15);
    /// the card's own 10 pt margin is part of that inset. A review keeps
    /// frame 68's cards.
    fn fit(&self, card: AnyElement) -> AnyElement {
        let inset = if self.metrics.review {
            0.
        } else {
            (self.metrics.code_start() - 10.).max(0.)
        };
        div()
            .max_w(px(self.metrics.view_width))
            .pl(px(inset))
            .child(card)
            .into_any_element()
    }

    /// The line at `index` of `lines` (a code line or a hunk header) and the
    /// cards under it: legacy sent comments, the comments and the open box.
    pub(super) fn line(&self, lines: &[DiffLine], index: usize) -> (AnyElement, Vec<AnyElement>) {
        let Metrics {
            look,
            columns,
            gutter_width,
            cell_width,
            line_height,
            review,
            old,
            ..
        } = self.metrics;
        let Some(line) = lines.get(index) else {
            return (div().into_any_element(), Vec::new());
        };
        let sign = match line.kind {
            LineKind::Added => "+",
            LineKind::Removed => "−",
            _ => " ",
        };
        let bg = tint(look, line.kind);
        if line.kind == LineKind::Hunk {
            // DS-22: a 24 pt band in the panel's colour with only the
            // section text, or a thin line when there is none. The `@@`
            // numbers are never drawn: the gutter has them.
            let section = hunk_section(&line.text);
            let header = if section.is_empty() {
                div().w_full().h(px(1.)).flex_shrink_0().bg(rgb(look.line))
            } else {
                row()
                    .w_full()
                    .h(px(24.))
                    .flex_shrink_0()
                    .items_center()
                    .px(px(10.))
                    .overflow_hidden()
                    .bg(rgb(look.panel))
                    .text_color(rgb(look.term_dim))
                    .child(
                        text(section.to_owned())
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis(),
                    )
            };
            // PR-19: a review's hunk header offers Ask about the hunk.
            let ask = self
                .target
                .as_ref()
                .filter(|_| self.reviewing)
                .and_then(|target| {
                    // Only this hunk's lines: quoting from the whole file
                    // made each header scan every line, which froze a
                    // 40,000-line file.
                    let hunk = hunk_lines(lines, index);
                    let (side, first, last) = span(hunk)?;
                    let group = SharedString::from(format!("change-hunk-group-{index}"));
                    Some((
                        group.clone(),
                        ask_over(
                            &self.ui,
                            look,
                            SharedString::from(format!("change-ask-hunk-{index}")),
                            group,
                            target.session.clone(),
                            target.anchor(hunk, side, first, last),
                        ),
                    ))
                });
            let header = match ask {
                Some((group, ask)) => div()
                    .id(SharedString::from(format!("change-hunk-{index}")))
                    .group(group)
                    .relative()
                    .child(header)
                    .child(ask)
                    .test_support()
                    .into_any_element(),
                None => header
                    .id(SharedString::from(format!("change-hunk-{index}")))
                    .test_support()
                    .into_any_element(),
            };
            let gap_index = lines[..index]
                .iter()
                .filter(|l| l.kind == LineKind::Hunk)
                .count();
            if let (Some(gap), Some(target)) = (compute_hunk_gap(lines, index, gap_index), self.target.as_ref()) {
                let expanded = gap_expanded_count(&target.session, &target.repo, &target.path, gap.gap_index);
                let remaining = remaining_hidden_lines(gap.hidden_lines, expanded);
                let mut stack = col().w_full();
                if remaining > 0 {
                    let s20 = target.session.clone();
                    let r20 = target.repo.clone();
                    let p20 = target.path.clone();
                    let sall = target.session.clone();
                    let rall = target.repo.clone();
                    let pall = target.path.clone();
                    let next_step = expand_gap_step(expanded, gap.hidden_lines, GAP_EXPAND_STEP);
                    let total_hidden = gap.hidden_lines;
                    let gix = gap.gap_index;
                    stack = stack.child(
                        row()
                            .id(SharedString::from(format!("diff-hunk-gap-{gix}")))
                            .test_support()
                            .w_full()
                            .h(px(24.))
                            .px(px(10.))
                            .gap(px(10.))
                            .items_center()
                            .bg(rgb(look.panel))
                            .border_y_1()
                            .border_color(rgb(look.line))
                            .font_family(".SystemUIFont")
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(text(format!(
                                "\u{2195} {}",
                                gap_label(remaining, gap.prev_new_end == 0)
                            )))
                            .child(
                                text("Expand 20 lines")
                                    .id(SharedString::from(format!("diff-hunk-gap-expand-20-{gix}")))
                                    .role(Role::Button)
                                    .test_support()
                                    .text_color(rgb(look.accent))
                                    .cursor_pointer()
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        set_gap_expanded_count(s20.clone(), r20.clone(), p20.clone(), gix, next_step);
                                        cx.refresh_windows();
                                    }),
                            )
                            .child(
                                text("Expand all")
                                    .id(SharedString::from(format!("diff-hunk-gap-expand-all-{gix}")))
                                    .role(Role::Button)
                                    .test_support()
                                    .text_color(rgb(look.accent))
                                    .cursor_pointer()
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        set_gap_expanded_count(
                                            sall.clone(),
                                            rall.clone(),
                                            pall.clone(),
                                            gix,
                                            expand_gap_all(total_hidden),
                                        );
                                        cx.refresh_windows();
                                    }),
                            ),
                    );
                }
                if expanded > 0 {
                    let start_line = gap.next_new_start.saturating_sub(expanded);
                    for line_no in start_line..gap.next_new_start {
                        stack = stack.child(
                            row()
                                .id(SharedString::from(format!("diff-hunk-gap-line-{}-{line_no}", gap.gap_index)))
                                .w_full()
                                .h(px(line_height))
                                .bg(rgb(look.term))
                                .child(
                                    text(line_no.to_string())
                                        .w(px(self.metrics.code_start()))
                                        .flex_shrink_0()
                                        .flex()
                                        .justify_end()
                                        .pr(px(10.))
                                        .text_color(rgb(look.term_dim)),
                                )
                                .child(
                                    text(format!("// context line {line_no}"))
                                        .text_color(rgb(look.term_dim)),
                                ),
                        );
                    }
                }
                stack = stack.child(header);
                return (stack.into_any_element(), Vec::new());
            }
            return (header, Vec::new());
        }
        if line.kind == LineKind::Note {
            // DS-23: dim, no number, no click target; it starts at the code.
            let note = text(line.text.clone())
                .id(SharedString::from(format!("change-note-{index}")))
                .test_support()
                .pl(px(self.metrics.code_start()))
                .whitespace_nowrap()
                .text_color(rgb(look.term_dim));
            return (note.into_any_element(), Vec::new());
        }
        let (side, number) = side_of(line);
        let focused = self.focus_index == Some(index);
        let editing = self.editing.as_ref();
        let lit = editing.is_some_and(|b| {
            let (first, last) = b.anchor.line_range();
            within_span(line, b.anchor.side, first, last)
        });
        let comment_lit = self
            .comment_spans
            .iter()
            .any(|&(c_side, first, last)| within_span(line, c_side, first, last));
        let fragments = if self.wrap {
            code_wrap::wrapped(&line.text, columns)
        } else {
            vec![code_wrap::unwrapped(&line.text)]
        };
        // DS-19: the number that is the comment target keeps `change-gutter-<n>`;
        // the other column of a full-screen row gets `-old-` or `-new-`.
        let number_width = (gutter_width - 8.).max(0.);
        let old_n = line.old;
        let new_n = line.new;
        let column = |shown: Option<u32>, id: String, first: bool| -> AnyElement {
            let mut cell = text(shown.map_or_else(String::new, |n| n.to_string()))
                .w(px(number_width))
                .h(px(line_height))
                .flex()
                .justify_end()
                .pr(px(if review { 6. } else { 4. }))
                .text_color(rgb(look.term_dim))
                .flex_shrink_0()
                .id(SharedString::from(id))
                .when(first && (lit || comment_lit || focused), |gutter| {
                    gutter.border_l_2().border_color(rgb(look.accent))
                })
                .when(focused, |gutter| {
                    gutter.font_weight(FontWeight::BOLD).text_color(rgb(look.accent))
                });
            if let (Some(target), Some(n), Some(_)) = (self.target.clone(), number, shown) {
                let target_move = target.clone();
                cell = cell
                    .role(Role::Button)
                    .aria_label(format!("Comment on line {n}"))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        target.click_line(side, old_n, new_n, event.modifiers.shift, cx);
                    })
                    .on_mouse_move(move |event: &MouseMoveEvent, _, cx| {
                        if event.pressed_button == Some(MouseButton::Left) {
                            target_move.drag_over(old_n, new_n, cx);
                        } else {
                            clear_diff_drag();
                        }
                    })
                    .on_mouse_up(MouseButton::Left, move |_, _, _| {
                        clear_diff_drag();
                    });
            }
            cell.test_support().into_any_element()
        };
        let target_id = format!("change-gutter-{index}");
        let gutters: Vec<AnyElement> = if old {
            let (old_id, new_id) = if side == Side::Old {
                (target_id, format!("change-gutter-new-{index}"))
            } else {
                (format!("change-gutter-old-{index}"), target_id)
            };
            vec![column(line.old, old_id, true), column(line.new, new_id, false)]
        } else {
            vec![column(number, target_id, true)]
        };
        // DS-21: the span of this line that differs from its partner.
        let mark = paired(lines, index)
            .and_then(|other| {
                let other = &lines.get(other)?.text;
                let removed = line.kind == LineKind::Removed;
                let (before, after) = if removed { (&line.text, other) } else { (other, &line.text) };
                let (before_span, after_span) = changed_span(before, after)?;
                Some(if removed {
                    (before_span, crate::theme::RED)
                } else {
                    (after_span, crate::theme::GREEN)
                })
            })
            .filter(|(span, _)| !span.is_empty());
        let line_syntax = self.syntax.line(index);
        let code = col().children(fragments.into_iter().enumerate().map(|(part, fragment)| {
            let overlay = mark.as_ref().and_then(|(span, color)| {
                let range = display_range(&line.text, &fragment, self.wrap, span)?;
                let style = HighlightStyle {
                    background_color: Some(rgb(blend(look.term, *color, 0.28)).into()),
                    ..Default::default()
                };
                Some((range, style))
            });
            let syntax_spans: Vec<(Range<usize>, HighlightStyle)> = line_syntax
                .iter()
                .filter_map(|(span, style)| {
                    let mapped = display_range(&line.text, &fragment, self.wrap, span)?;
                    Some((mapped, *style))
                })
                .collect();
            let highlights = humata2_highlight::merge_overlay(syntax_spans, overlay);
            let cell = div()
                .id(SharedString::from(format!("change-code-{index}-{part}")))
                .h(px(line_height))
                .whitespace_nowrap()
                .text_color(rgb(look.term_text));
            let cell = if highlights.is_empty() {
                cell.child(fragment.display.clone())
            } else {
                cell.child(StyledText::new(fragment.display.clone()).with_highlights(highlights))
            };
            cell.test_text(line.text[fragment.source].to_owned()).test_support()
        }));
        // On Scroll the number and sign stay put and only the code column
        // moves sideways (DIFFVIEW-design › Long lines and the way back).
        let code = if self.wrap {
            code.flex_1().min_w_0()
        } else {
            col()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(code.relative().left(px(-self.shift)))
        };
        // DS-20: numbers, a sign cell, one space, then the code; added and
        // removed rows share it, tinted full bleed with no borders.
        let line_row = row()
            .id(SharedString::from(format!("change-line-{index}")))
            .items_start()
            .w_full()
            .min_h(px(line_height))
            .when_some(bg, |row, bg| row.bg(rgb(bg)))
            .when(comment_lit && !lit, |row| row.bg(rgb(self.comment_lit_color)))
            .when(lit, |row| row.bg(rgb(self.lit_color)))
            .children(gutters)
            .child(
                text(sign)
                    .w(px(cell_width * 2.))
                    .h(px(line_height))
                    .flex_shrink_0()
                    .text_color(rgb(look.term_text)),
            )
            .child(code)
            .when_some(self.code_focus.clone(), |row, handle| {
                row.on_mouse_down(MouseButton::Left, move |_, window, cx| window.focus(&handle, cx))
            })
            .test_support();
        let line_row = match self.focus.filter(|_| focused) {
            Some((_, n)) => div()
                .id(SharedString::from(format!("review-focus-{n}")))
                .test_support()
                .child(line_row)
                .into_any_element(),
            None => line_row.into_any_element(),
        };
        let mut cards = Vec::new();
        for comment in self.sent.get(&index).into_iter().flatten() {
            cards.push(self.fit(sent_card(look, comment).into_any_element()));
        }
        if let Some((at, count, key, shown)) = self.fold.as_ref()
            && *at == index
        {
            cards.push(self.fit(comment_cards::fold_line(look, key, *count, *shown, &self.unfolded).into_any_element()));
        }
        for comment in self.cards.get(&index).into_iter().flatten() {
            cards.push(self.fit(
                comment_cards::card_with_todo(
                    &self.ui,
                    look,
                    comment,
                    &self.agent,
                    self.reviewing,
                    self.reattaching.as_ref() == Some(&comment.id),
                    &self.start,
                    &self.todo_ctx,
                )
                .into_any_element(),
            ));
        }
        if let (Some(b), Some(session), true) = (editing, self.session.as_ref(), self.box_at == Some(index)) {
            cards.push(self.fit(
                comment_cards::open_box(
                    &self.ui,
                    look,
                    session,
                    &b.anchor,
                    &self.input,
                    &self.agent,
                    &b.text,
                    &self.offers,
                )
                .into_any_element(),
            ));
        }
        (line_row, cards)
    }
}

/// `over` laid on `under` at `alpha`, as one solid colour.
fn blend(under: u32, over: u32, alpha: f32) -> u32 {
    let channel = |shift: u32| {
        let below = ((under >> shift) & 0xff) as f32;
        let above = ((over >> shift) & 0xff) as f32;
        ((below + (above - below) * alpha).round().clamp(0., 255.)) as u32
    };
    (channel(16) << 16) | (channel(8) << 8) | channel(0)
}

/// The tint of an added or removed row (DS-20): the status green or red at
/// about 12% in light and 16% in dark, laid on the code's colour.
fn tint(look: Look, kind: LineKind) -> Option<u32> {
    let base = match kind {
        LineKind::Added => crate::theme::GREEN,
        LineKind::Removed => crate::theme::RED,
        _ => return None,
    };
    let dark = ((look.term >> 8) & 0xff) < 0x80;
    Some(blend(look.term, base, if dark { 0.16 } else { 0.12 }))
}

/// The section text of a hunk header, after its second `@@` ("impl Client {"),
/// or nothing when it has none.
fn hunk_section(header: &str) -> &str {
    header
        .strip_prefix("@@")
        .and_then(|rest| rest.split_once("@@"))
        .map_or("", |(_, section)| section.trim())
}

/// The lines around `index` that are of its kind: the run it is in.
fn run_of(lines: &[DiffLine], index: usize) -> Option<Range<usize>> {
    let kind = lines.get(index)?.kind;
    let start = lines
        .get(..index)?
        .iter()
        .rposition(|line| line.kind != kind)
        .map_or(0, |at| at + 1);
    let end = lines
        .get(index..)?
        .iter()
        .position(|line| line.kind != kind)
        .map_or(lines.len(), |at| index + at);
    Some(start..end)
}

/// The line `index` is paired with for the word highlight (DS-21): where a
/// run of k removed lines is followed directly by k added lines, the n-th of
/// one is the partner of the n-th of the other.
fn paired(lines: &[DiffLine], index: usize) -> Option<usize> {
    let kind = lines.get(index)?.kind;
    let this = run_of(lines, index)?;
    let (removed, added) = match kind {
        LineKind::Removed => {
            if lines.get(this.end)?.kind != LineKind::Added {
                return None;
            }
            let added = run_of(lines, this.end)?;
            (this.clone(), added)
        }
        LineKind::Added => {
            let before = this.start.checked_sub(1)?;
            if lines.get(before)?.kind != LineKind::Removed {
                return None;
            }
            (run_of(lines, before)?, this.clone())
        }
        _ => return None,
    };
    if removed.len() != added.len() {
        return None;
    }
    let offset = index - this.start;
    Some(if kind == LineKind::Removed { added.start + offset } else { removed.start + offset })
}

/// The middle of two paired lines that differs (DS-21), as byte ranges of
/// `old` and `new` on char boundaries: what is left once their common start
/// and common end are taken off. `None` when the lines are equal or when the
/// common parts are not more than half of the longer line, where the whole
/// line is simply changed.
fn changed_span(old: &str, new: &str) -> Option<(Range<usize>, Range<usize>)> {
    let (mut prefix_bytes, mut prefix_chars) = (0, 0);
    for (a, b) in old.chars().zip(new.chars()) {
        if a != b {
            break;
        }
        prefix_bytes += a.len_utf8();
        prefix_chars += 1;
    }
    let (old_rest, new_rest) = (old.get(prefix_bytes..)?, new.get(prefix_bytes..)?);
    let (mut suffix_bytes, mut suffix_chars) = (0, 0);
    for (a, b) in old_rest.chars().rev().zip(new_rest.chars().rev()) {
        if a != b {
            break;
        }
        suffix_bytes += a.len_utf8();
        suffix_chars += 1;
    }
    let old_middle = prefix_bytes..old.len() - suffix_bytes;
    let new_middle = prefix_bytes..new.len() - suffix_bytes;
    if old_middle.is_empty() && new_middle.is_empty() {
        return None;
    }
    let longer = old.chars().count().max(new.chars().count());
    if (prefix_chars + suffix_chars) * 2 <= longer {
        return None;
    }
    Some((old_middle, new_middle))
}

/// The bytes `text` takes once drawn from `column`, and the column after it:
/// a tab fills to the next four-column stop, as `code_wrap` draws it.
fn drawn(text: &str, column: usize) -> (usize, usize) {
    let (mut bytes, mut column) = (0, column);
    for grapheme in text.graphemes(true) {
        if grapheme == "\t" {
            let count = 4 - column % 4;
            bytes += count;
            column += count;
        } else {
            bytes += grapheme.len();
            column += grapheme.width();
        }
    }
    (bytes, column)
}

/// Where `marked`, a span of the line `text` in source bytes, lies in the
/// text `fragment` draws: tabs are spaces there, and a wrapped line's first
/// row draws its indentation as spaces of its own and each later row an
/// indent. `None` when the fragment shows none of the span.
fn display_range(
    text: &str,
    fragment: &code_wrap::Fragment,
    wrapped: bool,
    marked: &Range<usize>,
) -> Option<Range<usize>> {
    let source = &fragment.source;
    // From here on the fragment draws the source itself.
    let body = if wrapped && source.start == 0 {
        text.len() - text.trim_start_matches([' ', '\t']).len()
    } else {
        source.start
    };
    let from = marked.start.max(body);
    let to = marked.end.min(source.end);
    if from >= to {
        return None;
    }
    let (_, column) = drawn(text.get(..body)?, 0);
    let (body_len, _) = drawn(text.get(body..source.end)?, column);
    let indent = fragment.display.len().checked_sub(body_len)?;
    let (lead, column) = drawn(text.get(body..from)?, column);
    let (length, _) = drawn(text.get(from..to)?, column);
    Some(indent + lead..indent + lead + length)
}

/// The digits of a file's widest line number and its widest line in cells,
/// as drawn on Scroll (tabs to four-column stops).
pub(super) fn line_facts(lines: &[DiffLine]) -> LineFacts {
    let digits = lines
        .iter()
        .flat_map(|l| [l.old, l.new])
        .flatten()
        .max()
        .unwrap_or(1)
        .to_string()
        .len();
    let widest = lines
        .iter()
        .filter(|l| is_code(l))
        .map(|l| code_wrap::unwrapped(&l.text).display.width())
        .max()
        .unwrap_or(0);
    LineFacts { digits, widest }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;

    fn line(kind: LineKind, text: &str) -> DiffLine {
        DiffLine {
            kind,
            old: None,
            new: None,
            text: text.to_owned(),
        }
    }

    #[test]
    fn equal_lines_have_no_changed_span() {
        assert_eq!(changed_span("let x = 1;", "let x = 1;"), None);
    }

    #[test]
    fn one_changed_word_is_the_span_on_both_lines() {
        let old = "let total = price + tax;";
        let new = "let total = price + fee;";
        let (before, after) = changed_span(old, new).expect("a span");
        assert_eq!(&old[before], "tax");
        assert_eq!(&new[after], "fee");
    }

    #[test]
    fn an_insertion_leaves_an_empty_span_on_the_old_line() {
        let old = "foo(a)";
        let new = "foo(a, b)";
        let (before, after) = changed_span(old, new).expect("a span");
        assert!(before.is_empty());
        assert_eq!(&new[after], ", b");
    }

    #[test]
    fn lines_mostly_different_have_no_changed_span() {
        assert_eq!(changed_span("abcdefgh", "abxyzwvu"), None);
        assert_eq!(changed_span("fn a() {}", "struct B;"), None);
    }

    #[test]
    fn multibyte_spans_stay_on_char_boundaries() {
        let old = "naïve café ☕ ok";
        let new = "naïve café ☃ ok";
        let (before, after) = changed_span(old, new).expect("a span");
        assert!(old.is_char_boundary(before.start) && old.is_char_boundary(before.end));
        assert!(new.is_char_boundary(after.start) && new.is_char_boundary(after.end));
        assert_eq!(&old[before], "☕");
        assert_eq!(&new[after], "☃");
    }

    #[test]
    fn a_run_of_removed_lines_pairs_with_as_many_added_lines() {
        let lines = [
            line(LineKind::Context, "a"),
            line(LineKind::Removed, "one"),
            line(LineKind::Removed, "two"),
            line(LineKind::Added, "uno"),
            line(LineKind::Added, "dos"),
            line(LineKind::Context, "b"),
        ];
        assert_eq!(paired(&lines, 1), Some(3));
        assert_eq!(paired(&lines, 2), Some(4));
        assert_eq!(paired(&lines, 3), Some(1));
        assert_eq!(paired(&lines, 4), Some(2));
        assert_eq!(paired(&lines, 0), None);
        assert_eq!(paired(&lines, 5), None);
    }

    #[test]
    fn runs_of_different_lengths_or_alone_have_no_partner() {
        let lines = [
            line(LineKind::Removed, "one"),
            line(LineKind::Removed, "two"),
            line(LineKind::Added, "uno"),
            line(LineKind::Context, "b"),
            line(LineKind::Added, "tres"),
        ];
        for index in 0..lines.len() {
            assert_eq!(paired(&lines, index), None, "line {index}");
        }
    }

    #[test]
    fn a_hunk_shows_only_its_section_text() {
        assert_eq!(hunk_section("@@ -40,7 +40,12 @@ impl Client {"), "impl Client {");
        assert_eq!(hunk_section("@@ -1 +1 @@"), "");
    }

    #[test]
    fn a_marked_span_is_found_in_the_text_a_row_draws() {
        // A tab draws as spaces up to the next stop of four columns.
        let text = "\tlet x = 1;";
        let fragment = code_wrap::unwrapped(text);
        let marked = 5..6;
        let shown = display_range(text, &fragment, false, &marked).expect("a range");
        assert_eq!(&fragment.display[shown], "x");
    }

    #[test]
    fn a_marked_span_is_found_on_the_wrapped_row_that_holds_it() {
        let text = "    let total = price + tax;";
        let marked = text.find("tax").map(|at| at..at + 3).expect("tax");
        let found: Vec<String> = code_wrap::wrapped(text, 12)
            .iter()
            .filter_map(|fragment| {
                let shown = display_range(text, fragment, true, &marked)?;
                Some(fragment.display[shown].to_owned())
            })
            .collect();
        assert_eq!(found.concat(), "tax");
    }

    #[test]
    fn within_span_matches_every_line_in_range_on_the_right_side() {
        let context = DiffLine {
            kind: LineKind::Context,
            old: Some(10),
            new: Some(12),
            text: "ctx".into(),
        };
        let added = DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(13),
            text: "add".into(),
        };
        let removed = DiffLine {
            kind: LineKind::Removed,
            old: Some(11),
            new: None,
            text: "rem".into(),
        };
        // New-side range 12..=14 covers context (new=12) and added (new=13), not removed.
        assert!(within_span(&context, Side::New, 12, 14));
        assert!(within_span(&added, Side::New, 12, 14));
        assert!(!within_span(&removed, Side::New, 12, 14));

        // Old-side range 10..=11 covers context (old=10) and removed (old=11), not added.
        assert!(within_span(&context, Side::Old, 10, 11));
        assert!(within_span(&removed, Side::Old, 10, 11));
        assert!(!within_span(&added, Side::Old, 10, 11));
    }

    #[test]
    fn resolve_diff_click_extends_and_normalizes_reverse_order() {
        let anchor_18 = Anchor {
            repo: "api".into(),
            path: "src/lib.rs".into(),
            side: Side::New,
            first: 18,
            last: 18,
            quoted: vec!["line 18".into()],
            base: Some("origin/main".into()),
            detached: false,
            outdated: false,
        };

        // Shift+click line 10 while box is on line 18 -> normalized 10..=18.
        assert_eq!(
            resolve_diff_click(Some(&anchor_18), "api", "src/lib.rs", Side::New, Some(8), Some(10), true),
            Some((Side::New, 10, 18))
        );
        // Plain click without Shift starts a fresh single-line selection.
        assert_eq!(
            resolve_diff_click(Some(&anchor_18), "api", "src/lib.rs", Side::New, Some(8), Some(10), false),
            Some((Side::New, 10, 10))
        );
        // Shift+click on a different file starts a single-line selection there.
        assert_eq!(
            resolve_diff_click(Some(&anchor_18), "api", "src/other.rs", Side::New, Some(8), Some(10), true),
            Some((Side::New, 10, 10))
        );
        // Shift+click a context line while box is on Side::Old extends on Side::Old.
        let old_anchor = Anchor {
            side: Side::Old,
            first: 10,
            last: 10,
            ..anchor_18
        };
        assert_eq!(
            resolve_diff_click(Some(&old_anchor), "api", "src/lib.rs", Side::New, Some(14), Some(16), true),
            Some((Side::Old, 10, 14))
        );
    }

    #[test]
    fn hunk_gap_calculation_and_expand_transitions() {
        let lines = vec![
            DiffLine { kind: LineKind::Hunk, old: None, new: None, text: "@@ -58,3 +58,3 @@".into() },
            DiffLine { kind: LineKind::Context, old: Some(58), new: Some(58), text: "a".into() },
            DiffLine { kind: LineKind::Context, old: Some(59), new: Some(59), text: "b".into() },
            DiffLine { kind: LineKind::Hunk, old: None, new: None, text: "@@ -85,2 +85,2 @@".into() },
            DiffLine { kind: LineKind::Context, old: Some(85), new: Some(85), text: "c".into() },
        ];
        let top = compute_hunk_gap(&lines, 0, 0).expect("top gap");
        assert_eq!(top.hidden_lines, 57);
        assert_eq!(gap_label(top.hidden_lines, true), "57 lines hidden above");

        let mid = compute_hunk_gap(&lines, 3, 1).expect("mid gap");
        assert_eq!(mid.prev_new_end, 59);
        assert_eq!(mid.next_new_start, 85);
        assert_eq!(mid.hidden_lines, 25);
        assert_eq!(gap_label(mid.hidden_lines, false), "25 hidden lines");

        let after_20 = expand_gap_step(0, mid.hidden_lines, GAP_EXPAND_STEP);
        assert_eq!(after_20, 20);
        assert_eq!(remaining_hidden_lines(mid.hidden_lines, after_20), 5);

        let after_all = expand_gap_all(mid.hidden_lines);
        assert_eq!(after_all, 25);
        assert_eq!(remaining_hidden_lines(mid.hidden_lines, after_all), 0);
    }

    #[test]
    fn readonly_file_view_head_base_and_scrollbar_markers() {
        let lines = vec![
            DiffLine { kind: LineKind::Context, old: Some(1), new: Some(1), text: "ctx".into() },
            DiffLine { kind: LineKind::Removed, old: Some(2), new: None, text: "old".into() },
            DiffLine { kind: LineKind::Added, old: None, new: Some(2), text: "new".into() },
            DiffLine { kind: LineKind::Added, old: None, new: Some(3), text: "extra".into() },
            DiffLine { kind: LineKind::Context, old: Some(3), new: Some(4), text: "end".into() },
        ];
        let head = readonly_file_lines(&lines, FileVersion::Head);
        assert_eq!(head.len(), 4);
        assert_eq!(head[1], (2, "new", LineKind::Added));

        let base = readonly_file_lines(&lines, FileVersion::Base);
        assert_eq!(base.len(), 3);
        assert_eq!(base[1], (2, "old", LineKind::Removed));

        let markers = compute_scrollbar_markers(&lines);
        assert_eq!(markers.len(), 3);
        // Line 1 (Removed) and Line 2 (Added) are paired -> Modified (or Added if run lengths differ: 1 removed vs 2 added -> Removed + Added + Added).
        assert_eq!(markers[0].kind, MarkerKind::Removed);
        assert_eq!(markers[1].kind, MarkerKind::Added);
        assert_eq!(markers[2].kind, MarkerKind::Added);
        assert!(markers[0].fraction < markers[1].fraction);
        assert!(markers[1].fraction < markers[2].fraction);

        let sid = SessionId::new("se_fv");
        assert_eq!(file_view_state(&sid, "api", "src/lib.rs"), FileViewState { open: false, version: FileVersion::Head });
        set_file_view_open(sid.clone(), "api".into(), "src/lib.rs".into(), true);
        set_file_view_version(sid.clone(), "api".into(), "src/lib.rs".into(), FileVersion::Base);
        assert_eq!(file_view_state(&sid, "api", "src/lib.rs"), FileViewState { open: true, version: FileVersion::Base });
    }
}
