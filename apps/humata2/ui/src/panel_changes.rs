//! The Diff panel: files, hunks, wrapped lines and comment cards.
use super::*;
use crate::comment_cards::{self, StartReattach};
use crate::document_tab::Documents;
use humata2_clients::comments::{Anchor, CommentId, Side};
use humata2_clients::model::DiffLine;
use std::cell::RefCell;

/// Whether a comment the workspace carries is about `path` of `diff`'s
/// repository.
fn on_path(comment: &Comment, diff: &RepoDiff, path: &str) -> bool {
    comment.path == path
        && comment
            .context
            .as_ref()
            .is_none_or(|context| context.repo == diff.repo)
}

/// How many comments sit on `path` of `repo` for the shown session: the
/// diff's own sent ones, and the daemon's comments (drafts included;
/// archived and hidden ones not).
pub(super) fn file_comments(state: &AppState, diff: &RepoDiff, path: &str) -> usize {
    let sent = diff
        .comments
        .iter()
        .filter(|comment| on_path(comment, diff, path))
        .count();
    let held: usize = state.session.open.as_ref().map_or(0, |session| {
        state
            .comments
            .on_file(session, &diff.repo, path)
            .values()
            .map(Vec::len)
            .sum()
    });
    sent + held
}

pub(super) fn changes(
    ui: &Rc<Ui>,
    state: &AppState,
    full: bool,
    look: Look,
    width: f32,
    window: &Window,
    documents: &Documents,
) -> Div {
    let font_size = f32::from(state.settings.preferences.font_size) - 0.5;
    let line_height = font_size + 5.5;
    let run = TextRun {
        len: 1,
        font: Font {
            family: "Menlo".into(),
            ..Default::default()
        },
        color: rgb(look.term_text).into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let cell_width = f32::from(
        window
            .text_system()
            .shape_line("M".into(), px(font_size), &[run], None)
            .width,
    )
    .max(1.);

    let diffs: Vec<_> = state
        .workspace
        .diffs
        .iter()
        .filter(|diff| Some(&diff.session) == state.session.open.as_ref())
        .collect();
    let picked = selected(state, &diffs);
    let (files, added, removed) = diffs
        .iter()
        .flat_map(|diff| &diff.files)
        .fold((0_usize, 0_u64, 0_u64), |(n, a, r), f| {
            (n + 1, a + u64::from(f.added), r + u64::from(f.removed))
        });
    let branch = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
        .map_or("", |session| session.branch.as_str());
    let list = col()
        .p(px(8.))
        .gap(px(1.))
        .text_size(px(12.5))
        .when(full, |list| {
            list.w(px(300.))
                .h_full()
                .flex_shrink_0()
                .border_r_1()
                .border_color(rgb(look.line))
        })
        .child(
            row()
                .gap(px(6.))
                .px(px(4.))
                .pb(px(6.))
                .child(secondary(
                    look,
                    format!(
                        "{files} file{} · {} repo{}",
                        if files == 1 { "" } else { "s" },
                        diffs.len(),
                        if diffs.len() == 1 { "" } else { "s" }
                    ),
                ))
                .child(div().flex_1())
                .child(
                    text(format!("+{added}"))
                        .text_size(px(11.))
                        .text_color(rgb(GREEN)),
                )
                .child(
                    text(format!("−{removed}"))
                        .text_size(px(11.))
                        .text_color(rgb(RED)),
                ),
        )
        .children(diffs.iter().map(|diff| {
            col()
                .child(
                    row()
                        .gap(px(6.))
                        .h(px(26.))
                        .px(px(4.))
                        .child(icon(IconName::ChevronDown, look.tertiary, 12.))
                        .child(icon(IconName::FolderGit2, look.secondary, 12.))
                        .child(text(diff.repo.clone()).font_weight(FontWeight::SEMIBOLD))
                        .child(div().flex_1())
                        .child(secondary(look, branch.to_owned()).text_color(rgb(look.tertiary))),
                )
                .children(diff.files.iter().map(|file| {
                    let ui = ui.clone();
                    let repo = diff.repo.clone();
                    let path = file.path.clone();
                    let on = picked.is_some_and(|(d, f)| d.repo == repo && f.path == path);
                    let comments = file_comments(state, diff, &file.path);
                    row()
                        .id(SharedString::from(format!("change-file {repo}/{path}")))
                        .role(Role::ListBoxOption)
                        .aria_label(format!("{repo}/{path}"))
                        .aria_selected(on)
                        .test_support()
                        .gap(px(6.))
                        .h(px(25.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .cursor_pointer()
                        .when(on, |row| row.bg(rgb(look.fill)))
                        .child(icon(IconName::FileCode, look.secondary, 12.))
                        .child(
                            text(path.clone())
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap(),
                        )
                        .when(comments > 0, |row| {
                            row.child(
                                widgets::row()
                                    .gap(px(3.))
                                    .flex_shrink_0()
                                    .child(icon(IconName::MessageSquare, look.accent, 11.))
                                    .child(
                                        text(comments.to_string())
                                            .text_size(px(11.))
                                            .text_color(rgb(look.accent)),
                                    ),
                            )
                        })
                        .child(
                            text(format!("+{}", file.added))
                                .text_size(px(11.))
                                .text_color(rgb(GREEN)),
                        )
                        .child(
                            text(format!("−{}", file.removed))
                                .text_size(px(11.))
                                .text_color(rgb(RED)),
                        )
                        .on_click(move |_, _, cx| {
                            ui.send(
                                AppAction::Review(ReviewAction::SelectFile {
                                    repo: repo.clone(),
                                    path: path.clone(),
                                }),
                                cx,
                            )
                        })
                }))
        }));
    let session_id = state.session.open.clone();
    let sendable = session_id
        .as_ref()
        .map_or(0, |session| state.comments.sendable(session).len());
    let view = col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .when_some(picked, |view, (diff, file)| {
            let digits = file
                .lines
                .iter()
                .flat_map(|l| [l.old, l.new])
                .flatten()
                .max()
                .unwrap_or(1)
                .to_string()
                .len();
            let gutter_width = 34_f32.max((digits as f32 + 1.) * cell_width);
            let code_width =
                width - if full { 300. } else { 0. } - gutter_width - cell_width - 2. + 8.;
            let columns = ((code_width / cell_width).floor().max(4.)) as usize;
            let scrolling_width = file
                .lines
                .iter()
                .filter(|l| l.kind != LineKind::Hunk)
                .map(|l| code_wrap::unwrapped(&l.text).display.width())
                .max()
                .unwrap_or(0) as f32
                * cell_width
                + gutter_width
                + cell_width;
            let name = format!("{}/{}", diff.repo, file.path);
            let rows = code_rows(
                ui,
                state,
                documents,
                diff,
                file,
                Metrics {
                    look,
                    columns,
                    gutter_width,
                    cell_width,
                    line_height,
                },
            );
            view.child(
                row()
                    .h(px(30.))
                    .px(px(10.))
                    .gap(px(6.))
                    .flex_shrink_0()
                    .border_y_1()
                    .border_color(rgb(look.line))
                    .text_size(px(12.))
                    .child(icon(IconName::FileCode, look.secondary, 12.))
                    .child(text(name.clone()).font_weight(FontWeight::MEDIUM))
                    .child(div().flex_1())
                    .child(
                        // No editor client exists yet: drawn, but dimmed and inert.
                        secondary(look, "Open in editor")
                            .id("change-open-editor")
                            .opacity(0.5),
                    ),
            )
            .child(
                col()
                    .id(SharedString::from(format!("change-code {name}")))
                    .role(Role::Code)
                    .aria_label(name)
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(!state.review.wrap, |x| x.overflow_x_scroll())
                    .when(state.review.wrap, |x| x.overflow_x_hidden())
                    .child(
                        col()
                            .py(px(6.))
                            .font_family("Menlo")
                            .text_size(px(font_size))
                            .line_height(px(line_height))
                            .bg(rgb(look.term))
                            .when(!state.review.wrap, |body| body.min_w(px(scrolling_width)))
                            .children(rows)
                    )
            )
        })
        .when(picked.is_none(), |view| {
            view.child(secondary(look, "No changes yet").p(px(12.)))
        });
    let base = diffs.first().map_or("", |diff| diff.base.as_str());
    col()
        .size_full()
        .child(header(
            ui,
            Panel::Changes,
            full,
            Some(
                row().gap(px(6.)).child(secondary(look, "against")).child(
                    row()
                        .gap(px(6.))
                        .px(px(9.))
                        .h(px(24.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .bg(rgb(look.fill))
                        .child(text(base.to_owned()))
                        .child(icon(IconName::ChevronsUpDown, look.secondary, 12.)),
                ),
            ),
            look,
        ))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .when(!full, |body| body.flex_col())
                .child(list)
                .child(view),
        )
        .child(
            row()
                .gap(px(8.))
                .p(px(10.))
                .border_t_1()
                .border_color(rgb(look.line))
                .flex_shrink_0()
                .child(secondary(
                    look,
                    format!(
                        "{sendable} draft comment{}",
                        if sendable == 1 { "" } else { "s" }
                    ),
                ))
                .child(div().flex_1())
                .when_some(
                    session_id.clone().filter(|_| sendable > 0),
                    |footer, session| {
                        footer.child(action(
                            ui,
                            "send-comments",
                            format!(
                                "Send {sendable} comment{}",
                                if sendable == 1 { "" } else { "s" }
                            ),
                            AppAction::Comments(CommentsAction::SendAll(session)),
                            look,
                            true,
                        ))
                    },
                ),
        )
}

/// A comment the workspace carries, already sent.
fn sent_card(look: Look, comment: &Comment) -> Div {
    card(look)
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
struct Metrics {
    look: Look,
    columns: usize,
    gutter_width: f32,
    cell_width: f32,
    line_height: f32,
}

/// The side a line's comments are on and its number there: a removed line
/// is on the old side of the file, every other line on the new side.
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

/// The index of the line a comment or box ending at `last` on `side` goes
/// under: that line, or the last code line when it is not in the hunks shown.
fn under(lines: &[DiffLine], side: Side, last: u32) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.kind != LineKind::Hunk && at(line, side, last))
        .or_else(|| lines.iter().rposition(|line| line.kind != LineKind::Hunk))
}

/// Everything a click on a line number needs.
struct Target {
    ui: Rc<Ui>,
    session: SessionId,
    repo: String,
    path: String,
    base: String,
    lines: Vec<DiffLine>,
    reattach: Rc<RefCell<Option<CommentId>>>,
}

impl Target {
    fn anchor(&self, side: Side, first: u32, last: u32) -> Anchor {
        let quoted = self
            .lines
            .iter()
            .filter(|line| line.kind != LineKind::Hunk)
            .filter(|line| (first..=last).any(|n| at(line, side, n)))
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
        }
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
                    self.anchor(side, line, line),
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
                self.anchor(side, range.0, range.1),
            )),
            cx,
        );
    }
}

/// The code lines of one file, each followed by the comments, legacy sent
/// ones and the open box that are about it.
fn code_rows(
    ui: &Rc<Ui>,
    state: &AppState,
    documents: &Documents,
    diff: &RepoDiff,
    file: &humata2_clients::model::FileDiff,
    metrics: Metrics,
) -> Vec<AnyElement> {
    let Metrics {
        look,
        columns,
        gutter_width,
        cell_width,
        line_height,
    } = metrics;
    let session = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id));
    let agent = session.map_or("the agent", |session| session.agent.command());
    let held = session.and_then(|session| state.comments.sessions.get(&session.id));
    let editing = held
        .and_then(|held| held.editing.as_ref())
        .filter(|b| b.anchor.repo == diff.repo && b.anchor.path == file.path);
    let box_comment = editing.and_then(|b| b.comment.clone());
    let reattaching = documents.reattach().borrow().clone();
    let lit_color = if look.term == 0xffffff { 0xeaf2ff } else { 0x1c2a3f };
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
            lines: file.lines.clone(),
            reattach: documents.reattach(),
        })
    });

    // Where each comment, legacy card and the box goes, by line index.
    let mut cards: Vec<Vec<&DaemonComment>> = vec![Vec::new(); file.lines.len()];
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
            if let Some(index) = under(&file.lines, comment.anchor.side, comment.anchor.last) {
                cards[index].push(comment);
            }
        }
    }
    let mut sent: Vec<Vec<&Comment>> = vec![Vec::new(); file.lines.len()];
    for comment in diff
        .comments
        .iter()
        .filter(|comment| on_path(comment, diff, &file.path))
    {
        if let Some(index) = under(&file.lines, Side::New, comment.line) {
            sent[index].push(comment);
        }
    }
    let box_at = editing.and_then(|b| under(&file.lines, b.anchor.side, b.anchor.last));

    let mut rows = Vec::new();
    for (index, line) in file.lines.iter().enumerate() {
        let (sign, bg) = match line.kind {
            LineKind::Added => ("+", Some(look.add)),
            LineKind::Removed => ("−", Some(look.del)),
            _ => (" ", None),
        };
        if line.kind == LineKind::Hunk {
            rows.push(
                text(line.text.clone())
                    .px(px(10.))
                    .whitespace_nowrap()
                    .text_color(rgb(look.term_dim))
                    .into_any_element(),
            );
            continue;
        }
        let (side, number) = side_of(line);
        let lit = editing.is_some_and(|b| {
            b.anchor.side == side
                && number.is_some_and(|n| b.anchor.first <= n && n <= b.anchor.last)
        });
        let fragments = if state.review.wrap {
            code_wrap::wrapped(&line.text, columns)
        } else {
            vec![code_wrap::unwrapped(&line.text)]
        };
        let mut gutter = text(number.map_or_else(String::new, |n| n.to_string()))
            .w(px((gutter_width - 8.).max(0.)))
            .h(px(line_height))
            .flex()
            .justify_end()
            .pr(px(4.))
            .text_color(rgb(look.term_dim))
            .flex_shrink_0()
            .id(SharedString::from(format!("change-gutter-{index}")))
            .when(lit, |gutter| gutter.border_l_2().border_color(rgb(look.accent)));
        if let (Some(target), Some(n)) = (target.clone(), number) {
            gutter = gutter
                .role(Role::Button)
                .aria_label(format!("Comment on line {n}"))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    target.click(side, n, event.modifiers.shift, cx);
                });
        }
        rows.push(
            row()
                .id(SharedString::from(format!("change-line-{index}")))
                .items_start()
                .w_full()
                .min_h(px(line_height))
                .when_some(bg, |row, bg| row.bg(rgb(bg)))
                .when(lit, |row| row.bg(rgb(lit_color)))
                .child(gutter.test_support())
                .child(
                    text(sign)
                        .w(px(cell_width))
                        .h(px(line_height))
                        .flex_shrink_0()
                        .text_color(rgb(look.term_text)),
                )
                .child(col().flex_1().min_w_0().children(
                    fragments.into_iter().enumerate().map(|(part, fragment)| {
                        div()
                            .id(SharedString::from(format!("change-code-{index}-{part}")))
                            .h(px(line_height))
                            .whitespace_nowrap()
                            .text_color(rgb(look.term_text))
                            .child(fragment.display)
                            .test_text(line.text[fragment.source].to_owned())
                            .test_support()
                    }),
                ))
                .test_support()
                .into_any_element(),
        );
        for comment in &sent[index] {
            rows.push(sent_card(look, comment).into_any_element());
        }
        for comment in &cards[index] {
            rows.push(
                comment_cards::card(
                    ui,
                    look,
                    comment,
                    agent,
                    reattaching.as_ref() == Some(&comment.id),
                    &start,
                )
                .into_any_element(),
            );
        }
        if let (Some(b), Some(session), true) = (editing, session, box_at == Some(index)) {
            rows.push(
                comment_cards::open_box(ui, look, &session.id, &b.anchor, &documents.input(), agent)
                    .into_any_element(),
            );
        }
    }
    rows
}
