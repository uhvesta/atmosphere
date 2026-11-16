//! The Diff panel: files, hunks, wrapped lines and comment cards.
use super::*;

/// How many comments sit on `path` of `repo` for the shown session: the
/// diff's own, this session's unsent drafts and its queued messages.
pub(super) fn file_comments(state: &AppState, diff: &RepoDiff, path: &str) -> usize {
    let here = |comment: &&Comment| {
        comment.path == path
            && comment
                .context
                .as_ref()
                .is_none_or(|context| context.repo == diff.repo)
    };
    let queued = state.review.queued().iter().filter(here).count();
    let pending = state
        .session
        .view()
        .into_iter()
        .flat_map(|view| &view.queue)
        .flat_map(|message| match message {
            humata2_session::QueuedMessage::Comments { comments } => comments.as_slice(),
            humata2_session::QueuedMessage::Text(_) => &[],
        })
        .filter(here)
        .count();
    diff.comments.iter().filter(here).count() + queued + pending
}

pub(super) fn changes(
    ui: &Rc<Ui>,
    state: &AppState,
    full: bool,
    look: Look,
    width: f32,
    window: &Window,
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
    let queued = state.review.queued();
    let pending: Vec<_> = state
        .session
        .view()
        .into_iter()
        .flat_map(|view| &view.queue)
        .flat_map(|message| match message {
            humata2_session::QueuedMessage::Comments { comments } => comments.as_slice(),
            humata2_session::QueuedMessage::Text(_) => &[],
        })
        .collect();
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
                            .children(file.lines.iter().enumerate().map(|(index, line)| {
                                let (sign, bg) = match line.kind {
                                    LineKind::Added => ("+", Some(look.add)),
                                    LineKind::Removed => ("−", Some(look.del)),
                                    _ => (" ", None),
                                };
                                if line.kind == LineKind::Hunk {
                                    return text(line.text.clone())
                                        .px(px(10.))
                                        .whitespace_nowrap()
                                        .text_color(rgb(look.term_dim))
                                        .into_any_element();
                                }
                                let number = if line.kind == LineKind::Removed {
                                    line.old
                                } else {
                                    line.new
                                };
                                let fragments = if state.review.wrap {
                                    code_wrap::wrapped(&line.text, columns)
                                } else {
                                    vec![code_wrap::unwrapped(&line.text)]
                                };
                                row()
                                    .id(SharedString::from(format!("change-line-{index}")))
                                    .items_start()
                                    .w_full()
                                    .min_h(px(line_height))
                                    .when_some(bg, |row, bg| row.bg(rgb(bg)))
                                    .child(
                                        // The number, a 4 pt gap, then the
                                        // sign ("41 −"), code where it was.
                                        text(number.map_or_else(String::new, |n| n.to_string()))
                                            .w(px((gutter_width - 8.).max(0.)))
                                            .h(px(line_height))
                                            .flex()
                                            .justify_end()
                                            .pr(px(4.))
                                            .text_color(rgb(look.term_dim))
                                            .flex_shrink_0()
                                            .id(SharedString::from(format!(
                                                "change-gutter-{index}"
                                            )))
                                            .test_support(),
                                    )
                                    .child(
                                        text(sign)
                                            .w(px(cell_width))
                                            .h(px(line_height))
                                            .flex_shrink_0()
                                            .text_color(rgb(look.term_text)),
                                    )
                                    .child(col().flex_1().min_w_0().children(
                                        fragments.into_iter().enumerate().map(
                                            |(part, fragment)| {
                                                div()
                                                    .id(SharedString::from(format!(
                                                        "change-code-{index}-{part}"
                                                    )))
                                                    .h(px(line_height))
                                                    .whitespace_nowrap()
                                                    .text_color(rgb(look.term_text))
                                                    .child(fragment.display)
                                                    .test_text(
                                                        line.text[fragment.source].to_owned(),
                                                    )
                                                    .test_support()
                                            },
                                        ),
                                    ))
                                    .test_support()
                                    .into_any_element()
                            })),
                    )
                    .children(
                        diff.comments
                            .iter()
                            .filter(|comment| {
                                comment.path == file.path
                                    && comment
                                        .context
                                        .as_ref()
                                        .is_none_or(|context| context.repo == diff.repo)
                            })
                            .map(|comment| comment_card(ui, state, look, comment, false, None))
                            .chain(
                                queued
                                    .iter()
                                    .enumerate()
                                    .filter(|(_, comment)| {
                                        comment.path == file.path
                                            && comment
                                                .context
                                                .as_ref()
                                                .is_none_or(|context| context.repo == diff.repo)
                                    })
                                    .map(|(index, comment)| {
                                        comment_card(ui, state, look, comment, false, Some(index))
                                    }),
                            )
                            .chain(
                                pending
                                    .iter()
                                    .filter(|comment| {
                                        comment.path == file.path
                                            && comment
                                                .context
                                                .as_ref()
                                                .is_none_or(|context| context.repo == diff.repo)
                                    })
                                    .map(|comment| {
                                        comment_card(ui, state, look, comment, true, None)
                                    }),
                            ),
                    ),
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
                        "{} draft comment{}",
                        queued.len(),
                        if queued.len() == 1 { "" } else { "s" }
                    ),
                ))
                .child(div().flex_1())
                .when(!queued.is_empty(), |footer| {
                    footer.child(action(
                        ui,
                        "send-comments",
                        format!(
                            "Send {} comment{}",
                            queued.len(),
                            if queued.len() == 1 { "" } else { "s" }
                        ),
                        AppAction::Review(ReviewAction::Send),
                        look,
                        true,
                    ))
                }),
        )
}

pub(super) fn comment_card(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    comment: &Comment,
    queued: bool,
    draft: Option<usize>,
) -> Div {
    card(look)
        .mx(px(10.))
        .my(px(6.))
        .p(px(9.))
        .gap(px(4.))
        .font_family(".SystemUIFont")
        .text_size(px(12.))
        .when(!comment.sent, |card| card.border_color(rgb(look.accent)))
        .child(
            row()
                .gap(px(6.))
                .child(text("You").font_weight(FontWeight::SEMIBOLD))
                .when(!comment.sent, |row| {
                    row.child(
                        text(if queued { "Waiting to send" } else { "Draft" })
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(rgb(look.line))
                            .text_size(px(11.))
                            .text_color(rgb(look.accent)),
                    )
                })
                .child(div().flex_1())
                .children(draft.map(|index| {
                    widgets::ghost(
                        ui,
                        SharedString::from(format!("send-comment-{index}")),
                        "Send",
                        AppAction::Review(ReviewAction::SendDraft(index)),
                    )
                    .h(px(20.))
                    .px(px(6.))
                    .text_size(px(11.))
                    .text_color(rgb(look.accent))
                    .disabled(state.review.sending)
                })),
        )
        .child(text(comment.body.clone()))
}
