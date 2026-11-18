//! The Diff panel: files, hunks, wrapped lines and comment cards.
use super::*;
use crate::comment_cards::{self, StartReattach};
use crate::document_tab::{Documents, LineFacts};
use humata2_clients::comments::{Anchor, CommentId, Side};
use humata2_clients::model::{DiffBase, DiffLine, RepoBase};
use humata2_review::order::{OrderFile, bottom_up};
use humata2_review::{PlanRow, PlanRowNote, PushState, RepoPicker};
use gpui_kit::component::button::ButtonCustomVariant;
use std::cell::RefCell;
use std::collections::HashMap;

/// A quiet text action of the Diff panel: Push & PR, Add repo… and a lone
/// base chevron. Its colour stays as drawn on hover and press, where a ghost
/// button's turns to the theme's text colour; a pale fill shows the hover and
/// the kit's ring shows keyboard focus (D739).
#[derive(IntoElement)]
struct Quiet {
    button: Button,
    color: u32,
    fill: u32,
}

impl RenderOnce for Quiet {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let fill = Hsla::from(rgb(self.fill));
        self.button.custom(
            ButtonCustomVariant::new(cx)
                .foreground(rgb(self.color).into())
                .hover(fill)
                .active(fill),
        )
    }
}

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

/// One changed file of `diff`: its row, which picks it when clicked.
fn file_row(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    diff: &RepoDiff,
    file: &humata2_clients::model::FileDiff,
    on: bool,
    short: bool,
) -> impl IntoElement {
    let ui = ui.clone();
    let repo = diff.repo.clone();
    let path = file.path.clone();
    let comments = file_comments(state, diff, &file.path);
    let group = SharedString::from(format!("change-file-group {repo}/{path}"));
    // PR-19: a review's file row offers Ask about the whole file on hover.
    let ask = reviewing(state, &diff.session)
        .then(|| span(&file.lines))
        .flatten()
        .map(|(side, first, last)| {
            let anchor = Anchor {
                repo: repo.clone(),
                path: path.clone(),
                side,
                first,
                last,
                // A whole file is named, not quoted.
                quoted: Vec::new(),
                base: Some(diff.base.clone()),
                detached: false,
                outdated: false,
            };
            ask_over(
                &ui,
                look,
                SharedString::from(format!("change-ask-file {repo}/{path}")),
                group.clone(),
                diff.session.clone(),
                anchor,
            )
        });
    row()
        .id(SharedString::from(format!("change-file {repo}/{path}")))
        .group(group.clone())
        .relative()
        .role(Role::ListBoxOption)
        .aria_label(format!("{repo}/{path}"))
        .aria_selected(on)
        .test_support()
        .gap(px(6.))
        .h(px(if short { 24. } else { 25. }))
        // One level in: the file icon sits under the repo's
        // folder icon and its name under the repo name
        // (repo row: 4 inset + 12 chevron + 6 gap = 22).
        // A review's inline row (frame 68) sits flush under its repo row,
        // its drafts drawn on their lines rather than counted.
        .pl(px(if short { 8. } else { 22. }))
        .pr(px(8.))
        .rounded(px(5.))
        .cursor_pointer()
        .when(on, |row| row.bg(rgb(look.fill)))
        .child(icon(IconName::FileCode, look.secondary, 12.))
        .child(
            text(if short {
                path.rsplit('/').next().unwrap_or(path.as_str()).to_owned()
            } else {
                path.clone()
            })
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap(),
        )
        .when(comments > 0 && !short, |row| {
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
        .children(ask)
        .on_click(move |_, _, cx| {
            ui.send(
                AppAction::Review(ReviewAction::SelectFile {
                    repo: repo.clone(),
                    path: path.clone(),
                }),
                cx,
            )
        })
}

/// A plan note as it reads: "L12–14 · text", or "L12 · text" on one line, or
/// just the text for a note on the whole file.
fn plan_note_label(note: &PlanRowNote) -> String {
    match note.lines {
        Some((first, last)) if first == last => format!("L{first} · {}", note.text),
        Some((first, last)) => format!("L{first}–{last} · {}", note.text),
        None => note.text.clone(),
    }
}

/// A plan's rows (RV-32): a header and note for each group, then its files in
/// the plan's order with the agent's notes under them.
fn plan_list(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    diffs: &[&RepoDiff],
    picked: Option<(&RepoDiff, &humata2_clients::model::FileDiff)>,
    plan: Vec<PlanRow>,
) -> Vec<AnyElement> {
    let mut groups = 0_usize;
    let mut outdated = 0_usize;
    let mut out = Vec::with_capacity(diffs.len() + plan.len());
    out.extend(
        diffs
            .iter()
            .enumerate()
            .map(|(index, diff)| plan_repo_line(ui, state, look, documents, index, diff).into_any_element()),
    );
    for entry in plan {
        match entry {
            PlanRow::Group { title, note } => {
                let n = groups;
                groups += 1;
                let mut header = col()
                    .px(px(4.))
                    .pt(px(8.))
                    .pb(px(2.))
                    .gap(px(2.))
                    .child(
                        text(title)
                            .id(SharedString::from(format!("plan-group-{n}")))
                            .test_support()
                            .font_weight(FontWeight::SEMIBOLD),
                    );
                if !note.is_empty() {
                    header = header.child(
                        secondary(look, note)
                            .id(SharedString::from(format!("plan-note-{n}")))
                            .test_support(),
                    );
                }
                out.push(header.into_any_element());
            }
            PlanRow::File { repo, path, notes } => {
                let Some(diff) = diffs.iter().find(|diff| diff.repo == repo) else {
                    continue;
                };
                let Some(file) = diff.files.iter().find(|file| file.path == path) else {
                    continue;
                };
                let on = picked.is_some_and(|(d, f)| d.repo == repo && f.path == path);
                let rows: Vec<Div> = notes
                    .into_iter()
                    .map(|note| {
                        let mut line = row()
                            .pl(px(40.))
                            .pr(px(8.))
                            .pb(px(2.))
                            .gap(px(6.))
                            .child(secondary(look, plan_note_label(&note)));
                        if note.outdated {
                            let n = outdated;
                            outdated += 1;
                            line = line.child(
                                secondary(look, "outdated")
                                    .id(SharedString::from(format!("plan-note-outdated-{n}")))
                                    .test_support(),
                            );
                        }
                        line
                    })
                    .collect();
                out.push(
                    col()
                        .child(file_row(ui, state, look, diff, file, on, false))
                        .children(rows)
                        .into_any_element(),
                );
            }
        }
    }
    out
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
    // A version of a Desk document is being compared with the current file
    // (DV-7): its text is the base, so the daemon's diffs don't apply.
    if let Some(body) = crate::doc_history::compare_body(ui, state, look) {
        return col()
            .size_full()
            .child(header(ui, Panel::Changes, full, None, look))
            .child(body);
    }
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
    // The plan's order, groups and notes when the session has one; computed
    // once for this frame.
    let plan = state
        .session
        .open
        .as_ref()
        .and_then(|session| state.review.plan_rows(session));
    // A pull request review (PR-6): the panel names its pull requests and
    // counts its drafts for GitHub instead of offering Push & PR.
    let review = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
        .filter(|session| session.pull_request_review);
    let session_id = state.session.open.clone();
    // What the open session's last read left behind (D179): a failed first
    // read replaces the body; a failed refresh keeps the last diff under a line.
    let fetch = session_id
        .as_ref()
        .and_then(|id| state.review.fetch.get(id));
    // A missing default branch (frame 98) always takes the body: the last
    // diff was against another base.
    let read_error = fetch.and_then(|fetch| {
        fetch
            .error
            .clone()
            .filter(|_| fetch.diffs.is_none() || fetch.no_default.is_some())
    });
    let refresh_error = fetch.and_then(|fetch| {
        fetch
            .error
            .clone()
            .filter(|_| fetch.diffs.is_some() && fetch.no_default.is_none())
    });
    let failed = fetch.is_some_and(|fetch| fetch.error.is_some());
    let shown = if read_error.is_some() { None } else { picked };
    // A review without a plan draws each repository's file and its code
    // inline under its row (frame 68); everything else keeps the list above
    // the code.
    let inline = review.is_some() && plan.is_none() && shown.is_some();
    let refresh_failed = refresh_error.is_some();
    let inline_handle = shown
        .filter(|_| inline)
        .map(|(diff, file)| documents.diff_view(&format!("{}/{}", diff.repo, file.path)).handle);
    let rows: Vec<AnyElement> = match plan {
        Some(plan) => plan_list(ui, state, look, documents, &diffs, picked, plan),
        None => diffs
            .iter()
            .enumerate()
            .map(|(index, diff)| {
                if let (Some(session), true) = (review, inline) {
                    return review_repo(
                        ui,
                        state,
                        look,
                        documents,
                        session,
                        diff,
                        picked,
                        (font_size, line_height, cell_width),
                        width,
                    );
                }
                col()
                    .child(match review {
                        Some(session) => review_repo_head(state, look, session, diff, true),
                        None => repo_head(ui, state, look, documents, index, diff, branch).into_any_element(),
                    })
                    .children(diff.files.iter().map(|file| {
                        let on = picked.is_some_and(|(d, f)| d.repo == diff.repo && f.path == file.path);
                        file_row(ui, state, look, diff, file, on, false)
                    }))
                    .into_any_element()
            })
            .collect(),
    };
    let list = col()
        .p(px(8.))
        // An inline review spaces its PR line and repositories 4 apart
        // (frame 68).
        .gap(px(if inline { 4. } else { 1. }))
        .text_size(px(12.5))
        .when(review.is_none(), |list| list.child(
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
                // REPOS (CH-21): Add repo to this session. A quiet text
                // action like Push & PR, so the summary line keeps the
                // goldens' height (frames 11, 85).
                .children(state.session.open.clone().map(|session| {
                    let ui = ui.clone();
                    let button = Button::new("changes-add-repo")
                        .small()
                        .h(px(18.))
                        .text_size(px(11.5))
                        .label("Add repo…")
                        .accessibility_label("Add repo…")
                        .flex_shrink_0()
                        .on_click(move |_, _, cx| {
                            ui.send(
                                AppAction::SessionAddRepo(humata2_app::SessionAddRepoAction::Open(session.clone())),
                                cx,
                            )
                        });
                    Quiet { button, color: look.accent, fill: look.fill }
                }))
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
        ))
        .children(rows);
    let sendable = session_id
        .as_ref()
        .map_or(0, |session| state.comments.sendable(session).len());
    let current_base = session_id
        .as_ref()
        .map(|id| state.review.targets_of(id).preset)
        .unwrap_or_default();
    let view = col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .when_some(read_error, |view, error| {
            view.child(read_failed_block(ui, state, documents, error, look))
        })
        .when_some(refresh_error, |view, error| {
            view.child(refresh_failed_row(ui, error, look))
        })
        .when_some(shown.filter(|_| !inline), |view, (diff, file)| {
            let name = format!("{}/{}", diff.repo, file.path);
            let scroll = documents.diff_view(&name);
            let wrap = state.review.wrap;
            // Counted once per read of the file, not on every frame.
            let LineFacts { digits, widest } = scroll.facts(&file.lines, line_facts);
            // The numbers take their digits, an 8 pt left inset, the 4 pt right pad
            // and the 8 pt the code overlaps, so 20000 is never clipped or flush.
            let gutter_width = 34_f32.max(digits as f32 * cell_width + 20.);
            let view_width = width - if full { 300. } else { 0. };
            let code_width = view_width - gutter_width - cell_width - 2. + 8.;
            let columns = ((code_width / cell_width).floor().max(4.)) as usize;
            // On Scroll only the code column moves sideways, as far as the
            // widest line needs.
            let max_shift = if wrap {
                0.
            } else {
                (widest as f32 * cell_width + cell_width)
                    - (view_width - (gutter_width - 8.) - cell_width)
            };
            let shift = scroll.shift(max_shift);
            let rows = CodeRows::new(
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
                    view_width,
                    review: false,
                },
                None,
                shift,
            );
            let sideways = {
                let scroll = scroll.clone();
                move |event: &ScrollEvent, _: &mut Window, cx: &mut App| {
                    let delta = event.delta.pixel_delta(px(line_height));
                    if delta.x != px(0.) && scroll.scroll_sideways(f32::from(delta.x), max_shift) {
                        cx.refresh_windows();
                    }
                }
            };
            let code = if file.lines.len() > VIRTUAL_FROM {
                // DV-5: a long file builds only the lines in view. The list
                // keeps its top line across a rewrap by itself.
                let state = scroll.rows(&file.lines, wrap, view_width, line_height);
                let rows = Rc::new(rows);
                let (session, repo, path) = (diff.session.clone(), diff.repo.clone(), file.path.clone());
                let guess = scroll.clone();
                col()
                    .on_children_prepainted(move |_, _, _| guess.guess_heights(line_height))
                    .on_scroll_wheel(sideways)
                    .id(SharedString::from(format!("change-code {name}")))
                    .role(Role::Code)
                    .aria_label(name.clone())
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .font_family("Menlo")
                    .text_size(px(font_size))
                    .line_height(px(line_height))
                    .bg(rgb(look.term))
                    .child(
                        gpui_kit::list(state, move |index, _, _| {
                            rows.ui
                                .read(|state| {
                                    file_lines(state, &session, &repo, &path)
                                        .map(|lines| rows.item(lines, index))
                                })
                                .unwrap_or_else(|| div().into_any_element())
                        })
                        .flex_1()
                        .w_full()
                        .py(px(6.)),
                    )
                    .into_any_element()
            } else {
                let items: Vec<AnyElement> =
                    (0..file.lines.len()).map(|index| rows.item(&file.lines, index)).collect();
                col()
                    .on_scroll_wheel(sideways)
                    .id(SharedString::from(format!("change-code {name}")))
                    .role(Role::Code)
                    .aria_label(name.clone())
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .track_scroll(&scroll.handle)
                    .overflow_y_scroll()
                    .overflow_x_hidden()
                    .child(
                        col()
                            .py(px(6.))
                            .font_family("Menlo")
                            .text_size(px(font_size))
                            .line_height(px(line_height))
                            .bg(rgb(look.term))
                            .on_children_prepainted(move |children, _, cx| {
                                scroll.keep_top(width, wrap, &children, cx)
                            })
                            .children(items),
                    )
                    .into_any_element()
            };
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
                    .child(text(name).font_weight(FontWeight::MEDIUM))
                    .child(div().flex_1())
                    .child(
                        // No editor client exists yet: drawn, but dimmed and inert.
                        secondary(look, "Open in editor")
                            .id("change-open-editor")
                            .opacity(0.5),
                    ),
            )
            .child(code)
        })
        .when(picked.is_none() && !failed, |view| {
            view.child(nothing_against(look, &current_base))
        });
    // Before the first diff arrives the chip names the picked base, never
    // an empty one.
    let base = diffs
        .first()
        .map(|diff| diff.base.clone())
        .filter(|base| !base.is_empty())
        .unwrap_or_else(|| picked_base_name(state, &current_base));
    let review_base = diffs.first().map_or_else(String::new, |diff| diff.base.clone());
    let menu_open = state.review.base_menu;
    // The repositories are compared with different bases: the header says
    // so instead of naming one (frames 84, 85).
    let per_repo = session_id
        .as_ref()
        .is_some_and(|id| state.review.compares_per_repo(id));
    let toggle = ui.clone();
    let head = header(
            ui,
            Panel::Changes,
            full,
            Some(
                row().gap(px(6.)).when(!per_repo, |line| line.child(secondary(look, "against").flex_shrink_0())).child(
                    div()
                        .relative()
                        .min_w_0()
                        .child(
                            row()
                                .id("diff-base")
                                .role(Role::Button)
                                .aria_label(if per_repo {
                                    "Comparing per repo".to_owned()
                                } else {
                                    format!("Against {base}")
                                })
                                .gap(px(6.))
                                .min_w_0()
                                .px(px(9.))
                                .h(px(24.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(rgb(look.line))
                                .bg(rgb(look.fill))
                                .cursor_pointer()
                                .map(|chip| {
                                    if per_repo {
                                        chip.child(
                                            text("Comparing per repo")
                                                .id("diff-base-per-repo")
                                                .test_support(),
                                        )
                                    } else {
                                        // The full base stays in the aria label;
                                        // the chip shows its start and an ellipsis.
                                        chip.child(
                                            text(base.clone())
                                                .min_w_0()
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis(),
                                        )
                                    }
                                })
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .child(icon(IconName::ChevronsUpDown, look.secondary, 12.)),
                                )
                                .on_click(move |_, _, cx| {
                                    toggle.send(
                                        AppAction::Review(ReviewAction::ToggleBaseMenu),
                                        cx,
                                    )
                                })
                                .test_support(),
                        )
                        .when(menu_open, |wrap| {
                            // Deferred, so the menu paints over the panel and
                            // takes the clicks there; the scrim first, then the menu.
                            wrap.child(deferred(base_scrim(ui)))
                                .child(deferred(base_menu(ui, state, look)))
                        }),
                ),
            ),
            look,
        );
    col()
        .size_full()
        // A pull request review's header names the pull request instead
        // of a base to pick (a pull request's base is GitHub's); Full screen
        // and Close stay, as on every panel.
        .child(match review {
            Some(session) => header(
                ui,
                Panel::Changes,
                full,
                Some(review_line(state, look, session, &review_base, diffs.len())),
                look,
            ),
            None => head,
        })
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .when(!full, |body| body.flex_col())
                .child(match inline_handle {
                    Some(handle) => div()
                        .id("review-diff")
                        .flex_1()
                        .min_h_0()
                        .track_scroll(&handle)
                        .overflow_y_scroll()
                        .child(list)
                        .test_support()
                        .into_any_element(),
                    None => div()
                        .id("changes-files")
                        .overflow_y_scroll()
                        .min_h_0()
                        .test_support()
                        .when(full, |files| {
                            files
                                // Stretched by the row, not h_full: a percent
                                // height grows with the list and pushes it
                                // past the window.
                                .w(px(300.))
                                .flex_shrink_0()
                                .border_r_1()
                                .border_color(rgb(look.line))
                        })
                        // Narrow: the list takes at most 220 pt, so the code
                        // below keeps the rest of the panel.
                        .when(!full, |files| files.flex_shrink_0().max_h(px(220.)))
                        .child(list)
                        .into_any_element(),
                })
                .when(!inline || refresh_failed, |body| body.child(view)),
        )
        .child(
            row()
                .gap(px(8.))
                .p(px(10.))
                // A review's footer is 38 tall with its line centred
                // (frame 68).
                .when(review.is_some(), |footer| footer.h(px(38.)).py(px(0.)))
                .border_t_1()
                .border_color(rgb(look.line))
                .flex_shrink_0()
                .child(match (review, session_id.as_ref()) {
                    (Some(_), Some(id)) => {
                        let (agent, you) = github_draft_counts(state, id);
                        // The id carries the counts the line shows.
                        secondary(
                            look,
                            format!("Drafts: {} ({agent} by agent, {you} by you)", agent + you),
                        )
                        .id(SharedString::from(format!("review-drafts-{agent}-{you}")))
                        .test_support()
                        .into_any_element()
                    }
                    _ => secondary(
                        look,
                        format!(
                            "{sendable} draft comment{}",
                            if sendable == 1 { "" } else { "s" }
                        ),
                    )
                    .into_any_element(),
                })
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

/// The drafts for GitHub of `session`, by the agent and by the person.
pub(super) fn github_draft_counts(state: &AppState, session: &SessionId) -> (usize, usize) {
    let drafts = state.comments.github_drafts(session);
    let by_you = drafts
        .iter()
        .filter(|draft| matches!(draft.author, humata2_clients::comments::Author::User))
        .count();
    (drafts.len() - by_you, by_you)
}

/// The pull requests `session` reviews, as the workspace lists them.
fn review_pulls<'a>(state: &'a AppState, session: &Session) -> Vec<&'a humata2_clients::model::PullRequest> {
    state
        .workspace
        .pulls
        .iter()
        .filter(|pull| pull.session.as_ref() == Some(&session.id))
        .collect()
}

/// The review's first line (PR-6): its pull request, what it is compared
/// with and how many repositories it spans (frame 68).
fn review_line(state: &AppState, look: Look, session: &Session, base: &str, repos: usize) -> Div {
    let pulls = review_pulls(state, session);
    let repos = if pulls.is_empty() { repos } else { pulls.len() };
    let name = pulls
        .first()
        .map_or_else(|| "Pull request".to_owned(), |pull| format!("PR #{}", pull.number));
    // The line ellipsizes before the header's buttons give way.
    div().min_w_0().overflow_hidden().child(row()
        .id("review-pr-line")
        .test_support()
        .gap(px(6.))
        .min_w_0()
        .overflow_hidden()
        .child(div().flex_shrink_0().child(icon(IconName::GitPullRequest, look.secondary, 12.)))
        .child(
            secondary(
                look,
                format!(
                    "{name} against {base} · {repos} repo{}",
                    if repos == 1 { "" } else { "s" }
                ),
            )
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_ellipsis(),
        ))
}

thread_local! {
    /// The focus the Diff panel last scrolled to: the review and file, and
    /// the focusing change's number.
    static REVEALED: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
}

/// Scrolls the review's list so the focused row is inside its view, once per
/// focus. Waits for the row's bounds and the view's.
fn reveal_focus(
    handle: &ScrollHandle,
    key: (String, u64),
    index: usize,
    children: &[Bounds<Pixels>],
    cx: &mut App,
) {
    if REVEALED.with(|held| held.borrow().as_ref() == Some(&key)) {
        return;
    }
    let (Some(item), view) = (children.get(index), handle.bounds()) else {
        return;
    };
    if view.size.height <= px(0.) {
        // The view hasn't been laid out yet: look again next frame.
        cx.refresh_windows();
        return;
    }
    let mut offset = handle.offset();
    if item.top() < view.top() {
        offset.y = offset.y + (view.top() - item.top());
    } else if item.bottom() > view.bottom() {
        offset.y = offset.y - (item.bottom() - view.bottom());
    }
    offset.y = offset.y.min(px(0.));
    handle.set_offset(offset);
    REVEALED.with(|held| *held.borrow_mut() = Some(key));
    cx.refresh_windows();
}

/// One repository of a review drawn inline (frame 68): its group row, then,
/// when it holds the picked file, its files bottom-up (leaf changes first),
/// the picked file's code right under its row. Other repositories are folded.
#[allow(clippy::too_many_arguments)]
fn review_repo(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    session: &Session,
    diff: &RepoDiff,
    picked: Option<(&RepoDiff, &humata2_clients::model::FileDiff)>,
    (font_size, line_height, cell_width): (f32, f32, f32),
    width: f32,
) -> AnyElement {
    let open = picked.is_some_and(|(held, _)| held.repo == diff.repo);
    // Frame 68: the repo row, its file and the file's code, 4 apart.
    let mut group = col().gap(px(4.)).child(review_repo_head(state, look, session, diff, open));
    if !open {
        return group.into_any_element();
    }
    let texts: Vec<String> = diff
        .files
        .iter()
        .map(|file| {
            file.lines
                .iter()
                .filter(|line| line.kind != LineKind::Hunk)
                .map(|line| line.text.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect();
    let ordered: Vec<OrderFile> = diff
        .files
        .iter()
        .zip(&texts)
        .map(|(file, text)| OrderFile { path: &file.path, text })
        .collect();
    for index in bottom_up(&ordered) {
        let file = &diff.files[index];
        let on = picked.is_some_and(|(held, f)| held.repo == diff.repo && f.path == file.path);
        group = group.child(file_row(ui, state, look, diff, file, on, true));
        if on {
            group = group.child(review_code(
                ui,
                state,
                look,
                documents,
                session,
                diff,
                file,
                (font_size, line_height, cell_width),
                width,
            ));
        }
    }
    group.into_any_element()
}

/// The picked file's code in a review, with the agent's focus marked and
/// scrolled to.
#[allow(clippy::too_many_arguments)]
fn review_code(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    session: &Session,
    diff: &RepoDiff,
    file: &humata2_clients::model::FileDiff,
    (font_size, line_height, cell_width): (f32, f32, f32),
    width: f32,
) -> AnyElement {
    let avail = width - 16.;
    let name = format!("{}/{}", diff.repo, file.path);
    let scroll = documents.diff_view(&name);
    let LineFacts { digits, .. } = scroll.facts(&file.lines, line_facts);
    // Frame 68's code starts 36 in (wider only for longer numbers), the
    // number ending 8 before it. The sign keeps its own cell, taken from
    // the number column, so the number column is 36 less a cell. Metrics
    // takes the width with its 8 added. Three digits still fit in 36, so
    // the code starts where the design's does and the sign never overlaps it.
    let numbers = 36_f32.max(digits as f32 * cell_width + 6. + cell_width);
    let gutter_width = numbers - cell_width + 8.;
    let code_width = avail - numbers;
    let columns = ((code_width / cell_width).floor().max(4.)) as usize;
    let focus = state.pr_follower.focus.as_ref().filter(|focus| {
        focus.place.path == file.path
            && (focus.place.repo == diff.repo || focus.place.repo.rsplit('/').next() == Some(diff.repo.as_str()))
    });
    let rows = CodeRows::new(
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
            view_width: avail,
            review: true,
        },
        focus.and_then(|focus| focus.line().map(|line| (focus.place.side, line))),
        0.,
    );
    // Frame 68 draws each run of code lines as its own block and the cards
    // between the blocks, 4 apart; a card keeps the panel's line height.
    let block = || {
        col()
            .py(px(6.))
            .overflow_hidden()
            .font_family("Menlo")
            .text_size(px(font_size))
            .line_height(px(line_height))
            .bg(rgb(look.term))
    };
    let mut parts: Vec<AnyElement> = Vec::new();
    let mut lines: Vec<AnyElement> = Vec::new();
    // The part the focused line ends up in, so the focus scrolls to its block.
    let mut focus_part = None;
    for index in 0..file.lines.len() {
        let (line, cards) = rows.line(&file.lines, index);
        if rows.focus_index == Some(index) {
            focus_part = Some(parts.len());
        }
        lines.push(line);
        if !cards.is_empty() {
            parts.push(block().children(std::mem::take(&mut lines)).into_any_element());
            parts.extend(cards);
        }
    }
    if !lines.is_empty() {
        parts.push(block().children(lines).into_any_element());
    }
    let reveal = focus
        .zip(focus_part)
        .map(|(focus, part)| ((format!("{}|{name}", session.id), focus.seq), part));
    col()
        .gap(px(4.))
        .on_children_prepainted(move |children, _, cx| {
            if let Some((key, row)) = reveal.clone() {
                reveal_focus(&scroll.handle, key, row, &children, cx);
            }
        })
        .id(SharedString::from(format!("change-code {name}")))
        .role(Role::Code)
        .aria_label(name)
        .test_support()
        .children(parts)
        .into_any_element()
}

/// A repository's group row in a review (frame 68): its name, its pull
/// request's number in the accent and the pull request's branch, dimmed.
fn review_repo_head(
    state: &AppState,
    look: Look,
    session: &Session,
    diff: &RepoDiff,
    open: bool,
) -> AnyElement {
    let number = humata2_review::comments::github_target(&state.workspace, &session.id, &diff.repo)
        .map(|(_, number)| number);
    let branch = session
        .clones
        .iter()
        .find(|clone| clone.repo == diff.repo || clone.repo.rsplit('/').next() == Some(diff.repo.as_str()))
        .map_or("", |clone| clone.branch.as_str());
    let name = diff.repo.rsplit('/').next().unwrap_or(diff.repo.as_str());
    row()
        .id(SharedString::from(format!("review-repo-{}", diff.repo)))
        .test_support()
        .gap(px(6.))
        .h(px(26.))
        .px(px(4.))
        .child(icon(
            if open { IconName::ChevronDown } else { IconName::ChevronRight },
            look.tertiary,
            12.,
        ))
        .child(icon(IconName::FolderGit2, look.secondary, 12.))
        .child(text(name.to_owned()).font_weight(FontWeight::SEMIBOLD))
        .children(number.map(|number| text(format!("#{number}")).text_color(rgb(look.accent))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_size(px(11.))
                .text_color(rgb(look.tertiary))
                .child(branch.to_owned()),
        )
        .into_any_element()
}

/// What the Diff panel says when nothing is picked and nothing failed: the
/// base decides the words.
/// The base the chip names while no diff has said which branch it is.
fn picked_base_name(state: &AppState, base: &DiffBase) -> String {
    match base {
        DiffBase::OriginMain => "origin/main".to_owned(),
        DiffBase::UncommittedOnly => "uncommitted only".to_owned(),
        DiffBase::SinceLastReviewed => "your last review".to_owned(),
        DiffBase::SinceFork => "the fork".to_owned(),
        DiffBase::OtherSession(id) => state
            .workspace
            .session(id)
            .map_or_else(|| "another session".to_owned(), |other| other.title.clone()),
        DiffBase::PullRequestBase => "the pull request's base".to_owned(),
        DiffBase::Spec(_) => "each repo's base".to_owned(),
    }
}

fn nothing_against(look: Look, base: &DiffBase) -> impl IntoElement {
    let (id, message) = match base {
        DiffBase::OriginMain => ("diff-no-changes-yet", "No changes yet"),
        DiffBase::UncommittedOnly => ("diff-none-against-base", "Nothing uncommitted"),
        DiffBase::SinceLastReviewed => {
            ("diff-none-against-base", "No changes since you last reviewed")
        }
        DiffBase::SinceFork => ("diff-none-against-base", "No changes since the fork"),
        DiffBase::OtherSession(_) => ("diff-none-against-base", "No changes against that branch"),
        DiffBase::PullRequestBase => {
            ("diff-none-against-base", "No changes against the pull request's base")
        }
        DiffBase::Spec(_) => ("diff-none-against-base", "No changes against each repository's base"),
    };
    secondary(look, message).p(px(12.)).id(id).test_support()
}

/// The panel when the open session's first read failed: what failed, and
/// Try again. It stands in for the diff body (D179).
fn read_failed_block(
    ui: &Rc<Ui>,
    state: &AppState,
    documents: &Documents,
    error: String,
    look: Look,
) -> impl IntoElement {
    use gpui_kit::base::Disableable as _;
    let block = col()
        .id("diff-read-failed")
        .flex_shrink_0()
        .gap(px(8.))
        .p(px(12.));
    // Frame 98: a clone with no default branch. Humata finds it; the
    // repository's own base chip is the other way out.
    let missing = state.session.open.as_ref().and_then(|id| {
        let fetch = state.review.fetch.get(id)?;
        let repo = fetch.no_default.clone()?;
        Some((id.clone(), repo, fetch.finding, fetch.find_error.clone()))
    });
    let Some((session, repo, finding, find_error)) = missing else {
        return block
            .child(secondary(look, "Couldn't read the changes."))
            .child(secondary(look, error))
            .child(action(
                ui,
                "diff-retry",
                "Try again",
                AppAction::Review(ReviewAction::Retry),
                look,
                false,
            ))
            .test_support();
    };
    let name = repo.rsplit('/').next().unwrap_or(repo.as_str()).to_owned();
    let find = AppAction::Review(ReviewAction::FindDefaultBranch { repo: repo.clone() });
    let block = match find_error {
        Some(reason) => block
            .child(secondary(look, format!("Couldn't find {name}'s default branch.")))
            .child(secondary(look, reason))
            .child(action(ui, "diff-find-default", "Try again", find, look, false).disabled(finding)),
        None => block
            .child(secondary(look, "Couldn't read the changes."))
            .child(secondary(look, format!("{name} has no default branch to compare with.")))
            .child(
                action(
                    ui,
                    "diff-find-default",
                    if finding { "Finding the default branch…" } else { "Find the default branch" },
                    find,
                    look,
                    true,
                )
                .disabled(finding),
            ),
    };
    let pick = RepoDiff {
        session,
        repo,
        // No default branch to name, so the chip asks for a pick (frame 98).
        base: "pick a branch…".to_owned(),
        files: Vec::new(),
        comments: Vec::new(),
    };
    block
        .child(
            // The hint and the chip stack, so the whole chip fits the panel
            // (frame 98).
            col()
                .gap(px(6.))
                .items_start()
                .child(secondary(look, "Or pick a branch to compare against."))
                .child(repo_base(ui, state, look, documents, usize::MAX, &pick)),
        )
        .test_support()
}

/// A refresh that failed while the last diff read still shows below it
/// (D179): what failed, and Try again.
fn refresh_failed_row(ui: &Rc<Ui>, error: String, look: Look) -> impl IntoElement {
    col()
        .id("diff-refresh-failed")
        .flex_shrink_0()
        .gap(px(2.))
        .px(px(12.))
        .py(px(6.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            row()
                .gap(px(8.))
                .child(secondary(
                    look,
                    "Couldn't refresh. Showing the last diff read.",
                ))
                .child(div().flex_1())
                .child(action(
                    ui,
                    "diff-retry",
                    "Try again",
                    AppAction::Review(ReviewAction::Retry),
                    look,
                    false,
                )),
        )
        .child(secondary(look, error))
        .test_support()
}

/// A transparent layer over the window behind the base menu: a click on it
/// closes the menu and reaches nothing else, as the Desk pop-ups' scrim does.
fn base_scrim(ui: &Rc<Ui>) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("diff-base-scrim")
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| ui.send(AppAction::Review(ReviewAction::CloseBaseMenu), cx))
}

/// One base in the menu: its label, a tick when it is the current base, and
/// the action a click sends.
fn base_row(
    ui: &Rc<Ui>,
    id: SharedString,
    label: impl Into<SharedString>,
    current: bool,
    event: AppAction,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    let label: SharedString = label.into();
    row()
        .id(id)
        .role(Role::ListBoxOption)
        .aria_label(label.clone())
        .aria_selected(current)
        .gap(px(8.))
        .h(px(26.))
        .px(px(8.))
        .rounded(px(5.))
        .cursor_pointer()
        .hover(move |x| x.bg(rgb(look.fill)))
        .child(
            div()
                .w(px(12.))
                .flex_shrink_0()
                .children(current.then(|| icon(IconName::Check, look.accent, 12.))),
        )
        .child(label)
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
        .test_support()
}

/// The base menu under the Diff header (RV-1, RV-6): what the changes are
/// compared with, the current base ticked, then Mark reviewed. The fork
/// base shows only for a fork; other sessions of the channel follow.
fn base_menu(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let here = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id));
    let current = state
        .session
        .open
        .as_ref()
        .map(|id| state.review.targets_of(id).preset)
        .unwrap_or_default();
    let pick = |id: &'static str, label: &'static str, base: DiffBase| {
        let on = base == current;
        base_row(
            ui,
            SharedString::from(id),
            label,
            on,
            AppAction::Review(ReviewAction::PickBase(base)),
            look,
        )
        .into_any_element()
    };
    let mut items = vec![
        pick(
            "diff-base-origin-main",
            "origin/main (where the branch started)",
            DiffBase::OriginMain,
        ),
        pick(
            "diff-base-uncommitted",
            "Uncommitted only",
            DiffBase::UncommittedOnly,
        ),
        pick(
            "diff-base-since-reviewed",
            "Since I last reviewed",
            DiffBase::SinceLastReviewed,
        ),
    ];
    if here.is_some_and(|session| session.pull_request_review) {
        items.insert(
            0,
            pick(
                "diff-base-pull-request",
                "The pull request's base",
                DiffBase::PullRequestBase,
            ),
        );
    }
    if here.is_some_and(|session| session.parent.is_some()) {
        items.push(pick(
            "diff-base-since-fork",
            "Since the fork from its parent",
            DiffBase::SinceFork,
        ));
    }
    let others: Vec<&Session> = here.map_or_else(Vec::new, |here| {
        state
            .workspace
            .sessions
            .iter()
            .filter(|other| {
                other.channel == here.channel && other.id != here.id && !other.archived
            })
            .collect()
    });
    if !others.is_empty() {
        items.push(
            secondary(look, "Another session's branch")
                .px(px(8.))
                .pt(px(6.))
                .into_any_element(),
        );
    }
    for other in others {
        let base = DiffBase::OtherSession(other.id.clone());
        let on = base == current;
        items.push(
            base_row(
                ui,
                SharedString::from(format!("diff-base-other-{}", other.id)),
                format!("{}'s branch", other.title),
                on,
                AppAction::Review(ReviewAction::PickBase(base)),
                look,
            )
            .into_any_element(),
        );
    }
    items.push(
        div()
            .h(px(1.))
            .my(px(4.))
            .bg(rgb(look.line))
            .into_any_element(),
    );
    let mark = ui.clone();
    items.push(
        row()
            .id("diff-mark-reviewed")
            .role(Role::Button)
            .aria_label("Mark reviewed")
            .gap(px(8.))
            .h(px(26.))
            .px(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            .hover(move |x| x.bg(rgb(look.fill)))
            .child(div().w(px(12.)).flex_shrink_0())
            .child("Mark reviewed")
            .on_click(move |_, _, cx| {
                mark.send(AppAction::Review(ReviewAction::MarkReviewed), cx)
            })
            .test_support()
            .into_any_element(),
    );
    col()
        .id("diff-base-menu")
        .role(Role::ListBox)
        .aria_label("Compare with")
        .absolute()
        .top(px(28.))
        .right(px(0.))
        .w(px(260.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .children(items)
        .test_support()
}

/// A repository's header in the Diff panel (frames 84, 85): its name, the
/// session's branch and its totals, then the base it is compared with,
/// which opens its own base menu, and Push & PR.
fn repo_head(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    index: usize,
    diff: &RepoDiff,
    branch: &str,
) -> Div {
    let (added, removed) = diff.files.iter().fold((0_u64, 0_u64), |(a, r), file| {
        (a + u64::from(file.added), r + u64::from(file.removed))
    });
    col()
        .child(
            row()
                .gap(px(6.))
                .h(px(26.))
                .px(px(4.))
                .pr(px(8.))
                .child(icon(IconName::ChevronDown, look.tertiary, 12.))
                .child(icon(IconName::FolderGit2, look.secondary, 12.))
                .child(text(diff.repo.clone()).font_weight(FontWeight::SEMIBOLD))
                .child(div().flex_1())
                .child(secondary(look, branch.to_owned()).text_color(rgb(look.tertiary)))
                .child(text(format!("+{added}")).text_size(px(11.)).text_color(rgb(GREEN)))
                .child(text(format!("−{removed}")).text_size(px(11.)).text_color(rgb(RED))),
        )
        .child(
            row()
                .gap(px(8.))
                .h(px(26.))
                .pl(px(40.))
                // The head row's inset, so a pull request's number ends
                // where the totals above it do.
                .pr(px(8.))
                .child(repo_base(ui, state, look, documents, index, diff))
                .child(div().flex_1())
                .child(push_control(ui, state, look, &diff.repo)),
        )
}

/// A plan's line for one repository (RV-31 with RV-32): the plan's groups can
/// mix repositories, so each repository's base and push sit once above them.
fn plan_repo_line(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    index: usize,
    diff: &RepoDiff,
) -> Div {
    row()
        .gap(px(6.))
        .h(px(26.))
        .px(px(4.))
        .pr(px(4.))
        .child(icon(IconName::FolderGit2, look.secondary, 12.))
        .child(text(diff.repo.clone()).font_weight(FontWeight::SEMIBOLD))
        .child(repo_base(ui, state, look, documents, index, diff))
        .child(div().flex_1())
        .child(push_control(ui, state, look, &diff.repo))
}

/// A repository's "against X" chip and, while it is open, its menu.
fn repo_base(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    index: usize,
    diff: &RepoDiff,
) -> Div {
    let picker = state
        .review
        .repo_picker
        .as_ref()
        .filter(|picker| picker.repo == diff.repo);
    let open = picker.is_some();
    // Another session's branch has no per-repository form: the base shows,
    // but only the header's menu changes it (VIEWS P1).
    let fixed = state.session.open.as_ref().is_some_and(|id| {
        matches!(state.review.targets_of(id).preset, DiffBase::OtherSession(_))
    });
    // While every repository shares the header's base, the chip shows only
    // its chevron: the words are the header's (D510, amended by D739).
    let words = fixed
        || state
            .session
            .open
            .as_ref()
            .is_some_and(|id| state.review.compares_per_repo(id));
    let tone = if open { look.accent } else { look.secondary };
    let toggle = ui.clone();
    let repo = diff.repo.clone();
    let id = SharedString::from(format!("repo-base-{index}"));
    let spoken = format!("{}: against {}", diff.repo, diff.base);
    let pick = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
        toggle.send(
            AppAction::Review(ReviewAction::ToggleRepoBase { repo: repo.clone() }),
            cx,
        )
    };
    let against = if words {
        row()
            .id(id)
            .role(Role::Button)
            .aria_label(spoken)
            .aria_expanded(open)
            .text_size(px(11.5))
            .whitespace_nowrap()
            .min_w_0()
            .child(text("against").flex_shrink_0().mr(px(3.)).text_color(rgb(tone)))
            // A long base (the daemon's "not reviewed yet: …") ends in
            // an ellipsis rather than pushing Push & PR out of the panel.
            .child(
                text(diff.base.clone())
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .text_color(rgb(if open { look.accent } else { look.text })),
            )
            .when(!fixed, |chip| {
                chip.cursor_pointer()
                    .child(
                        div()
                            .flex_shrink_0()
                            .ml(px(3.))
                            .child(icon(IconName::ChevronsUpDown, tone, 12.)),
                    )
                    .on_click(pick)
            })
            .test_support()
            .into_any_element()
    } else {
        // The chevron alone: a small icon button, so it keeps a target, a
        // hover fill, a focus ring and a tooltip that says what it changes.
        Quiet {
            button: Button::new(id)
                .small()
                .w(px(24.))
                .h(px(20.))
                .icon(icon(IconName::ChevronsUpDown, tone, 12.))
                .tooltip(format!("{}: against {} · change", diff.repo, diff.base))
                .accessibility_label(spoken)
                .on_click(pick),
            color: tone,
            fill: look.fill,
        }
        .into_any_element()
    };
    div().relative().min_w_0().child(against).when_some(picker, |wrap, picker| {
        // Deferred, so the menu paints over the panel and takes the clicks
        // there; the scrim first, then the menu.
        wrap.child(deferred(repo_base_scrim(ui)))
            .child(deferred(repo_base_menu(ui, state, look, documents, picker)))
    })
}

/// The layer behind a repository's base menu: a click on it closes the menu
/// and reaches nothing else.
fn repo_base_scrim(ui: &Rc<Ui>) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("repo-base-scrim")
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| ui.send(AppAction::Review(ReviewAction::CloseRepoBase), cx))
        .test_support()
}

/// One repository's base menu (frame 86, VIEWS P3): what to compare with,
/// the branches its clone knew at the last fetch under a filter, and the
/// branch or commit typed into the filter.
fn repo_base_menu(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    picker: &RepoPicker,
) -> impl IntoElement {
    let Some(session) = state.session.open.clone() else {
        return col().id("repo-base-menu").test_support();
    };
    let current = state.review.targets_of(&session).of(&picker.repo);
    let pick = |id: SharedString, label: String, base: RepoBase| {
        let on = base == current;
        base_row(
            ui,
            id,
            label,
            on,
            AppAction::Review(ReviewAction::PickRepoBase {
                session: session.clone(),
                repo: picker.repo.clone(),
                base,
            }),
            look,
        )
        .into_any_element()
    };
    let heading = |words: &'static str| {
        secondary(look, words)
            .px(px(8.))
            .pt(px(4.))
            .pb(px(2.))
            .into_any_element()
    };
    let line = || div().h(px(1.)).my(px(4.)).bg(rgb(look.line)).into_any_element();
    let mut items = vec![
        heading("Compare with"),
        pick("repo-base-default".into(), "Default branch (origin/HEAD)".into(), RepoBase::DefaultBranch),
        pick("repo-base-uncommitted".into(), "Uncommitted only".into(), RepoBase::Uncommitted),
        pick("repo-base-since-reviewed".into(), "Since I last reviewed".into(), RepoBase::LastReviewed),
    ];
    if state
        .workspace
        .session(&session)
        .is_some_and(|here| here.parent.is_some())
    {
        items.push(pick(
            "repo-base-since-fork".into(),
            "Since the fork from its parent".into(),
            RepoBase::ForkPoint,
        ));
    }
    if state
        .workspace
        .session(&session)
        .is_some_and(|here| here.pull_request_review)
    {
        items.insert(
            1,
            pick(
                "repo-base-pr".into(),
                "Pull request's base".into(),
                RepoBase::PullRequestBase,
            ),
        );
    }
    items.push(line());
    items.push(heading("Branches · as of last fetch"));
    items.push(
        div()
            .id("repo-base-filter")
            .test_support()
            .px(px(2.))
            .pb(px(4.))
            .child(
                gpui_kit::component::input::Input::new(&documents.base_filter())
                    .aria_label("Filter branches")
                    .small()
                    .min_h(px(26.))
                    .bg(rgb(look.card))
                    .border_color(rgb(look.line))
                    .text_color(rgb(look.text)),
            )
            .into_any_element(),
    );
    match &picker.bases {
        None => items.push(
            secondary(look, "Reading branches…")
                .px(px(8.))
                .py(px(4.))
                .id("repo-base-loading")
                .test_support()
                .into_any_element(),
        ),
        Some(Err(error)) => {
            let retry = ui.clone();
            items.push(
                col()
                    .id("repo-base-failed")
                    .aria_label(error.clone())
                    .test_support()
                    .gap(px(4.))
                    .px(px(8.))
                    .py(px(4.))
                    .child(secondary(look, error.clone()))
                    .child(
                        Button::new("repo-base-retry")
                            .small()
                            .label("Try again")
                            .on_click(move |_, _, cx| {
                                retry.send(AppAction::Review(ReviewAction::ReloadRepoBases), cx)
                            }),
                    )
                    .into_any_element(),
            );
        }
        Some(Ok(_)) => {
            let branches = picker.branches();
            if branches.is_empty() {
                items.push(
                    secondary(look, "No branch matches")
                        .px(px(8.))
                        .py(px(4.))
                        .id("repo-base-no-match")
                        .test_support()
                        .into_any_element(),
                );
            }
            for (n, branch) in branches.into_iter().enumerate() {
                items.push(pick(
                    SharedString::from(format!("repo-base-pick-{n}")),
                    branch.to_owned(),
                    RepoBase::Branch(branch.to_owned()),
                ));
            }
        }
    }
    items.push(line());
    // The filter's words as a branch, tag or commit; a name that doesn't
    // resolve fails in the read that follows, with a fix (VIEWS P3).
    let typed = picker.filter.trim().to_owned();
    if typed.is_empty() {
        items.push(
            row()
                .id("repo-base-rev")
                .aria_label("Branch or commit: type it in the filter")
                .test_support()
                .gap(px(8.))
                .h(px(26.))
                .px(px(8.))
                .child(div().w(px(12.)).flex_shrink_0())
                .child(text("Branch or commit…").text_color(rgb(look.secondary)))
                .into_any_element(),
        );
    } else {
        items.push(pick(
            "repo-base-rev".into(),
            format!("Branch or commit “{typed}”"),
            RepoBase::Rev(typed),
        ));
    }
    col()
        .id("repo-base-menu")
        .role(Role::ListBox)
        .aria_label(format!("Compare {} with", picker.repo))
        .absolute()
        .top(px(24.))
        .left(px(0.))
        .w(px(240.))
        .p(px(6.))
        .gap(px(1.))
        .rounded(px(9.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .children(items)
        .test_support()
}

/// A comment the workspace carries, already sent.
/// The Push & PR control of one repository group, and what its last push
/// left beside it: the pull request, or why the push failed (RV-2).
fn push_control(ui: &Rc<Ui>, state: &AppState, look: Look, repo: &str) -> Div {
    let outcome = state
        .session
        .open
        .clone()
        .and_then(|session| state.review.pushes.get(&(session, repo.to_owned())));
    let pushing = outcome == Some(&PushState::Pushing);
    let ui_push = ui.clone();
    let repo_push = repo.to_owned();
    let button = Button::new(SharedString::from(format!("changes-push-{repo}")))
        .small()
        .label(if pushing { "Pushing…" } else { "Push & PR" })
        // Every repo group draws the same words, so the accessible name says
        // which repo the button pushes (D388); the drawn text is unchanged.
        .accessibility_label(format!("Push & PR: {repo}"))
        .disabled(pushing)
        .on_click(move |_, _, cx| {
            ui_push.send(
                AppAction::Review(ReviewAction::Push {
                    repo: repo_push.clone(),
                }),
                cx,
            )
        });
    let status = match outcome {
        Some(PushState::Pushed { pull_request, note }) => {
            let label = pull_request.map_or_else(|| "Pushed".to_owned(), |number| format!("PR #{number}"));
            // A pull request left on another base says so (VIEWS P5); the
            // whole note is its accessible name.
            let spoken = if note.is_empty() { label.clone() } else { note.clone() };
            Some(
                row()
                    .id(SharedString::from(format!("changes-pushed-{repo}")))
                    .aria_label(spoken)
                    .test_support()
                    .gap(px(4.))
                    .child(
                        text(label)
                            .text_size(px(11.))
                            .text_color(rgb(look.secondary)),
                    )
                    .when(!note.is_empty(), |status| {
                        status.child(
                            div()
                                .id(SharedString::from(format!("changes-push-note-{repo}")))
                                .test_support()
                                .child(icon(IconName::Info, look.secondary, 12.)),
                        )
                    })
                    .into_any_element(),
            )
        }
        Some(PushState::Failed(reason)) => Some(
            row()
                .id(SharedString::from(format!("changes-push-failed-{repo}")))
                .aria_label(reason.clone())
                .test_support()
                .child(
                    text("Push failed")
                        .text_size(px(11.))
                        .text_color(rgb(RED)),
                )
                .into_any_element(),
        ),
        Some(PushState::Pushing) | None => None,
    };
    row()
        .gap(px(6.))
        .flex_shrink_0()
        .child(Quiet { button, color: look.accent, fill: look.fill })
        .when_some(status, |control, status| control.child(status))
}

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
#[derive(Clone, Copy)]
struct Metrics {
    look: Look,
    columns: usize,
    gutter_width: f32,
    cell_width: f32,
    line_height: f32,
    /// The code view's visible width: on Scroll the rows are wider, but a
    /// comment stays inside what can be seen.
    view_width: f32,
    /// A pull request review's inline code (frame 68): the code starts
    /// where the design's does, 36 in, so the sign's cell is taken from the
    /// number column and the space after the number.
    review: bool,
}

/// The side a line's comments are on and its number there: a removed line
/// is on the old side of the file, every other line on the new side.
/// Whether `session` reviews a pull request (PR-19: its hunks and files
/// offer Ask).
fn reviewing(state: &AppState, session: &SessionId) -> bool {
    state
        .workspace
        .session(session)
        .is_some_and(|found| found.pull_request_review)
}

/// The lines `lines` covers, as Ask on a hunk or a whole file points at
/// them: the new side's first and last number where it has any, else the
/// old side's (a hunk that only removes).
fn span(lines: &[DiffLine]) -> Option<(Side, u32, u32)> {
    [Side::New, Side::Old].into_iter().find_map(|side| {
        lines
            .iter()
            .filter(|line| line.kind != LineKind::Hunk)
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
fn ask_over(
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

/// The index of the line a comment or box ending at `last` on `side` goes
/// under: that line, or the last code line when it is not in the hunks shown.
fn under(lines: &[DiffLine], side: Side, last: u32) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.kind != LineKind::Hunk && at(line, side, last))
        .or_else(|| lines.iter().rposition(|line| line.kind != LineKind::Hunk))
}

/// The index of the line numbered `last` on `side`, when it is shown.
fn exactly_on(lines: &[DiffLine], side: Side, last: u32) -> Option<usize> {
    lines
        .iter()
        .position(|line| line.kind != LineKind::Hunk && at(line, side, last))
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
fn file_lines<'a>(state: &'a AppState, session: &SessionId, repo: &str, path: &str) -> Option<&'a [DiffLine]> {
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
const VIRTUAL_FROM: usize = 1000;

/// Everything one file's code rows need besides its lines, gathered once
/// per frame and owned, so a virtual list can build any line later in the
/// frame: the comments by line, the open box, the click target and the
/// agent's focus.
struct CodeRows {
    ui: Rc<Ui>,
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
    sent: HashMap<usize, Vec<Comment>>,
    reattaching: Option<CommentId>,
    start: StartReattach,
    input: Entity<gpui_kit::component::input::TextareaState>,
    target: Option<Rc<Target>>,
    lit_color: u32,
    /// The agent's focus, and the index of the first line it is on.
    focus: Option<(Side, u32)>,
    focus_index: Option<usize>,
}

impl CodeRows {
    /// The rows of `file`, with the line the agent focuses marked
    /// (`review-focus-<n>`, the accent border and a bold number).
    #[allow(clippy::too_many_arguments)]
    fn new(
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
                .position(|line| line.kind != LineKind::Hunk && at(line, side, n))
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
            sent,
            reattaching: documents.reattach().borrow().clone(),
            start,
            input: documents.input(),
            target,
            lit_color: if look.term == 0xffffff { 0xeaf2ff } else { 0x1c2a3f },
            focus,
            focus_index,
        }
    }

    /// One item: the line at `index` with the cards under it, as one block.
    fn item(&self, lines: &[DiffLine], index: usize) -> AnyElement {
        let (line, cards) = self.line(lines, index);
        if cards.is_empty() {
            line
        } else {
            col().child(line).children(cards).into_any_element()
        }
    }

    /// A card kept inside what can be seen of the code view.
    fn fit(&self, card: AnyElement) -> AnyElement {
        div().max_w(px(self.metrics.view_width)).child(card).into_any_element()
    }

    /// The line at `index` of `lines` (a code line or a hunk header) and the
    /// cards under it: legacy sent comments, the comments and the open box.
    fn line(&self, lines: &[DiffLine], index: usize) -> (AnyElement, Vec<AnyElement>) {
        let Metrics {
            look,
            columns,
            gutter_width,
            cell_width,
            line_height,
            review,
            ..
        } = self.metrics;
        let Some(line) = lines.get(index) else {
            return (div().into_any_element(), Vec::new());
        };
        let (sign, bg) = match line.kind {
            LineKind::Added => ("+", Some(look.add)),
            LineKind::Removed => ("−", Some(look.del)),
            _ => (" ", None),
        };
        if line.kind == LineKind::Hunk {
            let header = text(line.text.clone())
                .px(px(10.))
                .whitespace_nowrap()
                .text_color(rgb(look.term_dim));
            // PR-19: a review's hunk header offers Ask about the hunk.
            let ask = self
                .target
                .as_ref()
                .filter(|_| self.reviewing)
                .and_then(|target| {
                    let (side, first, last) = span(hunk_lines(lines, index))?;
                    let group = SharedString::from(format!("change-hunk-group-{index}"));
                    Some((
                        group.clone(),
                        ask_over(
                            &self.ui,
                            look,
                            SharedString::from(format!("change-ask-hunk-{index}")),
                            group,
                            target.session.clone(),
                            target.anchor(lines, side, first, last),
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
                None => header.into_any_element(),
            };
            return (header, Vec::new());
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
        let mut gutter = text(number.map_or_else(String::new, |n| n.to_string()))
            .w(px((gutter_width - 8.).max(0.)))
            .h(px(line_height))
            .flex()
            .justify_end()
            .pr(px(if review { (8. - cell_width).max(1.) } else { 4. }))
            .text_color(rgb(look.term_dim))
            .flex_shrink_0()
            .id(SharedString::from(format!("change-gutter-{index}")))
            .when(lit || focused, |gutter| gutter.border_l_2().border_color(rgb(look.accent)))
            .when(focused, |gutter| {
                gutter.font_weight(FontWeight::BOLD).text_color(rgb(look.accent))
            });
        if let (Some(target), Some(n)) = (self.target.clone(), number) {
            gutter = gutter
                .role(Role::Button)
                .aria_label(format!("Comment on line {n}"))
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    target.click(side, n, event.modifiers.shift, cx);
                });
        }
        let code = col().children(fragments.into_iter().enumerate().map(|(part, fragment)| {
            div()
                .id(SharedString::from(format!("change-code-{index}-{part}")))
                .h(px(line_height))
                .whitespace_nowrap()
                .text_color(rgb(look.term_text))
                .child(fragment.display)
                .test_text(line.text[fragment.source].to_owned())
                .test_support()
        }));
        // On Scroll the number and sign stay put and only the code column
        // moves sideways (DIFFVIEW-design › Long lines and the way back).
        let code = if self.wrap || review {
            code.flex_1().min_w_0()
        } else {
            col()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(code.relative().left(px(-self.shift)))
        };
        let line_row = row()
            .id(SharedString::from(format!("change-line-{index}")))
            .items_start()
            .w_full()
            .min_h(px(line_height))
            .when_some(bg, |row, bg| row.bg(rgb(bg)))
            .when(lit, |row| row.bg(rgb(self.lit_color)))
            .child(gutter.test_support())
            .child(
                text(sign)
                    .w(px(cell_width))
                    .h(px(line_height))
                    .flex_shrink_0()
                    // Frame 68 draws the sign in the code's first
                    // cell, a cell clear of the number. A review moves
                    // it there when that cell is indentation, so it
                    // never covers code; the columns stay put.
                    .when(review && line.text.starts_with([' ', '\t']), |sign| {
                        sign.relative().left(px(cell_width))
                    })
                    .text_color(rgb(look.term_text)),
            )
            .child(code)
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
        for comment in self.cards.get(&index).into_iter().flatten() {
            cards.push(self.fit(
                comment_cards::card(
                    &self.ui,
                    look,
                    comment,
                    &self.agent,
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

/// The digits of a file's widest line number and its widest line in cells,
/// as drawn on Scroll (tabs to four-column stops).
fn line_facts(lines: &[DiffLine]) -> LineFacts {
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
        .filter(|l| l.kind != LineKind::Hunk)
        .map(|l| code_wrap::unwrapped(&l.text).display.width())
        .max()
        .unwrap_or(0);
    LineFacts { digits, widest }
}
