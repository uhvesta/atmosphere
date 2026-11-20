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

impl Target {
    fn anchor(&self, lines: &[DiffLine], side: Side, first: u32, last: u32) -> Anchor {
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
    /// when that box is on the same file and side.
    fn click(&self, side: Side, line: u32, extend: bool, cx: &mut App) {
        let pending = self.reattach.borrow_mut().take();
        if let Some(id) = pending {
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
        let range = extend
            .then(|| {
                self.ui.read(|state| {
                    state
                        .comments
                        .sessions
                        .get(&self.session)
                        .and_then(|held| held.editing.as_ref())
                        .filter(|b| {
                            b.anchor.repo == self.repo
                                && b.anchor.path == self.path
                                && b.anchor.side == side
                        })
                        .map(|b| (b.anchor.first.min(line), b.anchor.last.max(line)))
                })
            })
            .flatten()
            .unwrap_or((line, line));
        self.ui.send(
            AppAction::Comments(CommentsAction::OpenBox(
                self.session.clone(),
                self.anchor_now(side, range.0, range.1),
            )),
            cx,
        );
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
    /// The agent's focus, and the index of the first line it is on.
    focus: Option<(Side, u32)>,
    pub(super) focus_index: Option<usize>,
    /// What a click on a line focuses, outside a review: the code view.
    code_focus: Option<FocusHandle>,
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
            focus,
            focus_index,
            code_focus: (!metrics.review).then(|| documents.diff_code.clone()),
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
            b.anchor.side == side
                && number.is_some_and(|n| b.anchor.first <= n && n <= b.anchor.last)
        });
        let fragments = if self.wrap {
            code_wrap::wrapped(&line.text, columns)
        } else {
            vec![code_wrap::unwrapped(&line.text)]
        };
        // DS-19: the number that is the comment target keeps `change-gutter-<n>`;
        // the other column of a full-screen row gets `-old-` or `-new-`.
        let number_width = (gutter_width - 8.).max(0.);
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
                .when(first && (lit || focused), |gutter| {
                    gutter.border_l_2().border_color(rgb(look.accent))
                })
                .when(focused, |gutter| {
                    gutter.font_weight(FontWeight::BOLD).text_color(rgb(look.accent))
                });
            if let (Some(target), Some(n), Some(_)) = (self.target.clone(), number, shown) {
                cell = cell
                    .role(Role::Button)
                    .aria_label(format!("Comment on line {n}"))
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        target.click(side, n, event.modifiers.shift, cx);
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
        let code = col().children(fragments.into_iter().enumerate().map(|(part, fragment)| {
            let marked = mark.as_ref().and_then(|(span, color)| {
                Some((display_range(&line.text, &fragment, self.wrap, span)?, *color))
            });
            let cell = div()
                .id(SharedString::from(format!("change-code-{index}-{part}")))
                .h(px(line_height))
                .whitespace_nowrap()
                .text_color(rgb(look.term_text));
            let cell = match marked {
                Some((range, color)) => {
                    let style = HighlightStyle {
                        background_color: Some(rgb(blend(look.term, color, 0.28)).into()),
                        ..Default::default()
                    };
                    cell.child(
                        StyledText::new(fragment.display.clone()).with_highlights([(range, style)]),
                    )
                }
                None => cell.child(fragment.display.clone()),
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
                comment_cards::card(
                    &self.ui,
                    look,
                    comment,
                    &self.agent,
                    self.reviewing,
                    self.reattaching.as_ref() == Some(&comment.id),
                    &self.start,
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
}
