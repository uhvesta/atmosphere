//! The Diff panel: files, hunks, wrapped lines and comment cards.
use crate::bridge::Ui;
use crate::chrome::{
    GREEN, RED, card, icon, panel_action as action, panel_header_actions, secondary, selected, text,
};
use crate::code_wrap;
use crate::document_tab::{Documents, LineFacts};
use crate::full_text::full_text;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, TestTextExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::comments::Comment as DaemonComment;
use humata2_clients::model::{Comment, LineKind, RepoDiff, Session, SessionId};
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::Panel;
use std::rc::Rc;

mod agent_tab;
mod diff_body;
mod file_list;
mod plan;
mod review_header;
mod states;

use self::diff_body::*;
use self::file_list::*;
use self::plan::*;
use self::review_header::*;
use self::states::*;

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

pub fn changes(
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
            .child(diff_header(ui, state, full, None, look))
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
        .and_then(|session| state.review.plan_layout(session));
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
    // DS-35: a repo whose read failed shows its message and Try again in a row
    // of its own after the repositories that were read (`repo_error_row`).
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
    let failed =
        fetch.is_some_and(|fetch| fetch.error.is_some() || !fetch.repo_errors.is_empty());
    let shown = if read_error.is_some() { None } else { picked };
    // A review without a plan draws each repository's file and its code
    // inline under its row (frame 68); everything else keeps the list above
    // the code.
    let inline = review.is_some() && plan.is_none() && shown.is_some();
    let refresh_failed = refresh_error.is_some();
    let inline_handle = shown
        .filter(|_| inline)
        .map(|(diff, file)| {
            documents
                .session_diff_view(&diff.session, &format!("{}/{}", diff.repo, file.path))
                .handle
        });
    // DS-2: with a file picked, the narrow list folds into the file bar.
    // Full screen keeps its list column and the inline review its own list.
    let file_open = session_id
        .as_ref()
        .and_then(|id| state.review.saved.selected.get(id))
        .is_some_and(|(repo, path)| {
            diffs
                .iter()
                .any(|diff| diff.repo == *repo && diff.files.iter().any(|file| file.path == *path))
        });
    let bar = !full && !inline && file_open && shown.is_some();
    let list_open = bar
        && session_id
            .as_ref()
            .is_some_and(|id| state.review.file_list_open.contains(id));
    let crumbs_hidden_narrow = !full && width < 560.;
    // D852: whether the open file is looked at; a pull request review has no looks.
    let shown_looked = shown.is_some_and(|(diff, file)| {
        review.is_none()
            && humata2_review::looks::is_looked(&state.review, &diff.session, &diff.repo, &file.path, file)
    });
    let crumbs_hidden = shown_looked && crumbs_hidden_narrow;
    // D800: the stack's rows send through this.
    let stack_send = crate::panel_stack::sender(ui);
    // Whether a stack shows (D800): its file header, list and footer follow
    // frames 119 to 121.
    let stacked = review.is_none() && crate::panel_stack::header_line(&state.review).is_some();
    // DS-84: while the file list has focus, the row at the keyboard cursor wears a ring.
    let cursor = session_id
        .as_ref()
        .filter(|_| documents.diff_list.is_focused(window))
        .and_then(|id| state.review.list_cursor(id));
    let has_plan = plan.is_some();
    let mut rows: Vec<AnyElement> = match plan {
        Some(plan) => plan_list(ui, state, look, documents, &diffs, picked, cursor, plan),
        None => diffs
            .iter()
            .enumerate()
            // RV-6: a repository with no matching file has no row.
            .filter(|(_, diff)| {
                session_id.as_ref().is_none_or(|id| {
                    diff.files.is_empty()
                        || diff.files.iter().any(|file| state.review.file_visible(id, &file.path))
                })
            })
            .map(|(index, diff)| {
                let folded = session_id
                    .as_ref()
                    .is_some_and(|id| state.review.collapsed.contains(&(id.clone(), diff.repo.clone())));
                if let (Some(session), true) = (review, inline) {
                    return review_repo(
                        ui,
                        state,
                        look,
                        documents,
                        session,
                        diff,
                        picked,
                        cursor,
                        (font_size, line_height, cell_width),
                        width,
                    );
                }
                col()
                    .child(ringed(
                        match review {
                            Some(session) => review_repo_head(ui, state, look, session, diff, true),
                            None => repo_head(ui, state, look, documents, index, diff, branch, folded),
                        },
                        at_repo(cursor, &diff.repo),
                        look,
                    ))
                    .children({
                        // DS-67: skim files fold into one row at the end.
                        let split = humata2_review::stack::fold_skims(&diff.files, |file| {
                            crate::panel_stack::tier_of(&state.review, diff, file)
                        });
                        let visible = |file: &&humata2_clients::model::FileDiff| {
                            session_id
                                .as_ref()
                                .is_none_or(|id| state.review.file_visible(id, &file.path))
                        };
                        let skims_open = crate::panel_stack::skim_open(&state.review, &diff.repo);
                        let skim_paths: Vec<&str> = split.skims.iter().filter(|file| visible(file)).map(|file| file.path.as_str()).collect();
                        let row_of = |file: &humata2_clients::model::FileDiff| {
                            let on = picked.is_some_and(|(d, f)| d.repo == diff.repo && f.path == file.path);
                            ringed(
                                file_row(ui, state, look, diff, file, on, false),
                                at_file(cursor, &diff.repo, &file.path),
                                look,
                            )
                        };
                        let mut files: Vec<AnyElement> = Vec::new();
                        if !folded {
                            files.extend(split.shown.iter().filter(|file| visible(file)).map(|file| row_of(*file)));
                            files.extend(crate::panel_stack::skim_row(
                                look,
                                &diff.repo,
                                &skim_paths,
                                skims_open,
                                &stack_send,
                            ));
                            if skims_open {
                                files.extend(split.skims.iter().filter(|file| visible(file)).map(|file| row_of(*file)));
                            }
                        }
                        files
                    })
                    .into_any_element()
            })
            .collect(),
    };
    // DS-35: the repositories that couldn't be read, each with its own Try again.
    rows.extend(
        fetch
            .into_iter()
            .flat_map(|fetch| fetch.repo_errors.iter())
            .map(|(repo, message)| repo_error_row(ui, look, repo, message)),
    );
    // DS-28, DS-34: the files stay, dimmed, until the new diff arrives.
    let loading = fetch.is_some_and(|fetch| fetch.loading);
    // RV-6: above 20 changed files a field filters the list.
    let filter_field = session_id
        .as_ref()
        .filter(|id| !has_plan && !inline && state.review.file_filter_shown(id))
        .map(|id| file_filter_field(ui, look, documents, id));
    let list = col()
        .p(px(8.))
        .when(loading, |list| list.opacity(0.5))
        // An inline review spaces its PR line and repositories 4 apart
        // (frame 68).
        .gap(px(if inline { 4. } else { 1. }))
        .text_size(px(12.5))
        .children(since_caption(state, look))
        .children(filter_field)
        .child(list_rows(ui, documents, if inline { 4. } else { 1. }).children(rows));
    let sendable = session_id
        .as_ref()
        .map_or(0, |session| state.comments.sendable(session).len());
    let (looked_n, looks_total) = looked_of(state, &diffs);
    let looked_all = looks_total > 0 && looked_n == looks_total;
    let roomy = full || width >= 560.;
    // How many characters of the status fit: the width less the padding and
    // what shares its row (the buttons), at about 5.2 pt a character.
    let button_room = if looked_all { 100. } else { 90. };
    let room = if roomy {
        (width - 403.).max(0.)
    } else {
        (width - 20. - 8. - button_room).max(0.)
    } / 5.2;
    let looks_line = looks_status(state, &diffs, sendable, room as usize);
    let looks_status_el = || {
        looks_line
            .clone()
            .filter(|_| review.is_none())
            .map(|(line, green)| {
                full_text(
                    text(line.clone())
                        .id("diff-looks-status")
                        .test_support()
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_size(px(11.5))
                        .text_color(rgb(if green { LOOK_GREEN } else { look.secondary })),
                    line,
                )
                .into_any_element()
            })
    };
    // Looks good: bordered, not primary. Once every file shown is looked at
    // it reads "✓ Looks good" in green and pressing it clears those looks. A
    // pull request keeps Submit review… instead.
    let looks_button_el = || {
        (review.is_none() && looks_total > 0).then(|| {
            let all = looked_all;
            let dark = look.term != 0xffffff;
            let (tint, edge) = if dark { (0x17301f, 0x24542f) } else { (0xe9f7ed, 0xb7e0c2) };
            let button = action(
                ui,
                "diff-looks-good",
                if all { "✓ Looks good" } else { "Looks good" },
                AppAction::Review(if all { ReviewAction::ClearLooks } else { ReviewAction::LooksGood }),
                look,
                false,
            );
            if all {
                // One style in every state: fill, border and label set together;
                // hover and press only darken the fill.
                let (hover, press) = if dark { (0x1d3a26, 0x23452d) } else { (0xdcf1e2, 0xcfebd7) };
                let ui = ui.clone();
                div()
                    .id("diff-looks-good")
                    .role(Role::Button)
                    .aria_label("✓ Looks good")
                    .test_support()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .h(px(24.))
                    .px(px(11.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(rgb(edge))
                    .bg(rgb(tint))
                    .text_color(rgb(LOOK_GREEN))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_nowrap()
                    .cursor_pointer()
                    .hover(move |style| style.bg(rgb(hover)))
                    .active(move |style| style.bg(rgb(press)))
                    .on_click(move |_, _, cx| ui.send(AppAction::Review(ReviewAction::ClearLooks), cx))
                    .child("✓ Looks good")
                    .into_any_element()
            } else {
                button.into_any_element()
            }
        })
    };
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
            // DS-40: ⌥Z flips Wrap and Scroll while focus is in the file's
            // header or code. DS-15: full screen is the code's colour from
            // top to bottom.
            let view = view
                .track_focus(&documents.diff_code)
                .key_context(crate::keys::DIFF)
                .when(full, |view| view.bg(rgb(look.term)));
            let name = format!("{}/{}", diff.repo, file.path);
            let scroll = documents.session_diff_view(&diff.session, &name);
            let wrap = state.review.wrap;
            // DS-27: the change stepped to, and whether it moved since the
            // last frame, so the code scrolls its hunk to the top once.
            let changes = humata2_review::change_count(file);
            let change = state.review.current_change(&diff.session);
            let hunk_line = file
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.kind == humata2_clients::model::LineKind::Hunk)
                .map(|(at, _)| at)
                .nth(change);
            let moved = change_moved(&format!("{}:{name}", diff.session), change);
            let stepper = changes > 1 && file.lines.len() > 60;
            // Counted once per read of the file, not on every frame.
            let LineFacts { digits, widest } = scroll.facts(&file.lines, line_facts);
            // The numbers take their digits, an 8 pt left inset, the 4 pt right pad
            // and the 8 pt the code overlaps, so 20000 is never clipped or flush.
            let gutter_width = 34_f32.max(digits as f32 * cell_width + 20.);
            let view_width = width - if full { 300. } else { 0. };
            // DS-19: full screen draws an old and a new number column,
            // beside the terminal one.
            let start = code_start_of(gutter_width, cell_width, full);
            let code_width = view_width - start - 2.;
            let columns = ((code_width / cell_width).floor().max(4.)) as usize;
            // On Scroll only the code column moves sideways, as far as the
            // widest line needs.
            let max_shift = if wrap {
                0.
            } else {
                (widest as f32 * cell_width + cell_width) - (view_width - start)
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
                    old: full,
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
                if moved {
                    state.scroll_to(ListOffset {
                        item_ix: hunk_line.unwrap_or(0),
                        offset_in_item: px(0.),
                    });
                }
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
                let handle = scroll.handle.clone();
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
                    // DS-15: the code's colour below the last line too.
                    .bg(rgb(look.term))
                    .child(
                        col()
                            .py(px(6.))
                            .font_family("Menlo")
                            .text_size(px(font_size))
                            .line_height(px(line_height))
                            .bg(rgb(look.term))
                            .on_children_prepainted(move |children, _, cx| {
                                scroll.keep_top(width, wrap, &children, cx);
                                if moved && let Some(child) = children.get(hunk_line.unwrap_or(0)) {
                                    put_at_top(&handle, child, cx);
                                }
                            })
                            .children(items),
                    )
                    .into_any_element()
            };
            // DS-24, DS-36: a file with nothing to draw shows its note as
            // its body.
            let note = file_note(file);
            let coded = file.lines.iter().any(is_code);
            let code = if coded {
                code
            } else {
                col()
                    .id("change-file-empty")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .p(px(12.))
                    .child(
                        secondary(
                            look,
                            note.clone().unwrap_or_else(|| {
                                // A file listed with counts has changes; only
                                // its lines are missing.
                                if file.added + file.removed > 0 {
                                    "The changed lines aren't available".to_owned()
                                } else {
                                    "No changes to show".to_owned()
                                }
                            }),
                        )
                            .text_size(px(12.)),
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
                    .when(!stacked, |header| header.child(icon(IconName::FileCode, look.secondary, 12.)))
                    .child({
                        // `repo · dir/name`: the directory gives way first and
                        // the name stays; the block ends before "Open in editor".
                        let (dir, file_name) = split_path(&file.path);
                        // A stack names a repository by its last part, as its rows do.
                        let shown_repo = if stacked {
                            crate::panel_stack::stack_repo_name(&diff.repo, diffs.iter().map(|each| each.repo.as_str()))
                        } else {
                            repo_label(&diff.repo)
                        };
                        row()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .gap(px(4.))
                            // Squeezed beside the terminal with a look to say, only
                            // the name stays: no stubs of the crumbs.
                            .when(!crumbs_hidden, |line| {
                                line.child(full_text(
                                secondary(look, shown_repo.to_owned())
                                    .id("diff-file-header-repo")
                                    .flex_shrink_0()
                                    .max_w(relative(0.25))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis(),
                                shown_repo.to_owned(),
                            ))
                            // The "·" stays when the repo itself ends in "…".
                            .child(secondary(look, "·").flex_shrink_0())
                            .child(full_text(
                                secondary(look, dir.to_owned())
                                    .id("diff-file-header-dir")
                                    .min_w_0()
                                    .flex_shrink(1.)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis(),
                                dir,
                            ))
                            })
                            // The name keeps most of the row, then ends in an ellipsis.
                            .child(full_text(
                                text(file_name.to_owned())
                                    .id("diff-file-header-name")
                                    .flex_shrink_0()
                                    .max_w(relative(0.68))
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .font_weight(FontWeight::MEDIUM),
                                file_name.to_owned(),
                            ))
                    })
                    // The switch does nothing on a file with no code lines. A
                    // stack's header leaves it to ⌥Z (frames 119 to 121).
                    .when(coded && !stacked, |header| header.child(wrap_switch(ui, look, wrap)))
                    .when(stepper, |header| {
                        header.child(change_stepper(ui, look, change, changes))
                    })
                    // With the file bar showing, Open in editor sits there.
                    .when(shown_looked && !bar, |header| {
                        header.child(
                            text("✓ You looked at this")
                                .id("diff-file-looked")
                                .test_support()
                                .flex_shrink_0()
                                .whitespace_nowrap()
                                .text_color(rgb(LOOK_GREEN)),
                        )
                    })
                    .when(!bar && !stacked, |header| header.child(open_in_editor(look)))
                    .when(!bar && stacked, |header| header.child(open_in_editor_words(look))),
            )
            // With no code lines the note is the body, so it isn't said twice.
            .children(note.filter(|_| coded).map(|note| {
                let whole = note.clone();
                let line = secondary(look, note)
                    .id("change-file-note")
                    .test_support()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .px(px(10.))
                    .py(px(4.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis();
                full_text(line, whole)
            }))
            .child(code)
        })
        .when(picked.is_none() && !failed, |view| {
            // DS-33: before the first diff arrives the panel is reading, not empty.
            if loading && diffs.is_empty() {
                view.child(
                    secondary(look, "Reading changes…")
                        .p(px(12.))
                        .id("changes-reading")
                        .test_support(),
                )
            } else {
                view.child(nothing_against(ui, state, look, &current_base))
            }
        });
    // Before the first diff arrives the chip names the picked base, never
    // an empty one.
    // DS-34: while a read is under way the chip names the base just picked.
    let base = diffs
        .first()
        .map(|diff| diff.base.clone())
        .filter(|base| !base.is_empty() && !loading)
        .unwrap_or_else(|| picked_base_name(state, &current_base));
    let review_base = diffs.first().map_or_else(String::new, |diff| diff.base.clone());
    let menu_open = state.review.base_menu && !crate::panel_stack::hides_base_picker(&state.review);
    // The repositories are compared with different bases: the header says
    // so instead of naming one (frames 84, 85).
    let per_repo = session_id
        .as_ref()
        .is_some_and(|id| state.review.compares_per_repo(id));
    let toggle = ui.clone();
    // The base chip's words: its label, and what its hover shows.
    let base_chip = if per_repo {
        "Comparing per repo".to_owned()
    } else {
        format!("Against {base}")
    };
    let head = diff_header(
            ui,
            state,
            full,
            Some(
                row().gap(px(6.)).when(!per_repo, |line| line.child(secondary(look, "against").flex_shrink_0())).child(
                    div()
                        .relative()
                        .min_w_0()
                        .child(full_text(
                            row()
                                .id("diff-base")
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
                            base_chip,
                        )
                        .role(Role::Button))
                        .when(menu_open, |wrap| {
                            // Deferred, so the menu paints over the panel and
                            // takes the clicks there; the scrim first, then the menu.
                            wrap.child(deferred(base_scrim(ui)))
                                .child(deferred(base_menu(ui, state, look)))
                        }),
                ).when(full, |line| line.child(since_switch(ui, state, look))),
            ),
            look,
        );
    crate::panel_stack::stack_keys(col(), &state.review)
        .size_full()
        // A pull request review's header names the pull request instead
        // of a base to pick (a pull request's base is GitHub's); Full screen
        // and Close stay, as on every panel.
        .child(match review {
            Some(session) => diff_header(
                ui,
                state,
                full,
                Some(review_line(state, look, session, &review_base, diffs.len())),
                look,
            ),
            None => match crate::panel_stack::header_line(&state.review) {
                Some(line) => diff_header(
                    ui,
                    state,
                    full,
                    Some(row().min_w_0().child(
                        secondary(look, line).id("stack-header").test_support(),
                    )),
                    look,
                ),
                None => head,
            },
        })
        // D852: beside the terminal the switch has a row of its own under the
        // header once a look exists (36 pt: a 28 pt control, 4 above and
        // below); a stack (fixed bases) and a pull request review have none.
        .when(
            !full
                && review.is_none()
                && look_exists(state)
                && crate::panel_stack::header_line(&state.review).is_none(),
            |panel| {
                panel.child(
                    row()
                        .h(px(36.))
                        .px(px(10.))
                        .items_center()
                        .flex_shrink_0()
                        .child(since_switch(ui, state, look)),
                )
            },
        )
        // D800, DS-60 to DS-65: the layer list sits above the files.
        .children(crate::panel_stack::stack_top(&state.review, &state.comments, look, &stack_send, width).map(|top| {
            // D804: the layer keys work in the Diff panel, so a click in
            // the layer list gives it the focus.
            let handle = documents.diff_stack.clone();
            div()
                .flex_shrink_0()
                .track_focus(&documents.diff_stack)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| window.focus(&handle, cx))
                .child(top)
        }))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                // D807: the loading sweep lies over the body's top edge.
                .relative()
                // The inline review keeps one column, bar above list, in full screen too.
                .when(!full || inline, |body| body.flex_col())
                .children(shown.filter(|_| bar).map(|(diff, file)| {
                    file_bar(ui, look, &diffs, diff, file, Some(list_open), shown_looked, crumbs_hidden)
                }))
                // DS-53: the inline review's list doesn't fold, so its bar is no fold
                // button; it steps files across every pull request of the review.
                .children(
                    shown
                        .filter(|_| inline)
                        .map(|(diff, file)| file_bar(ui, look, &diffs, diff, file, None, false, false)),
                )
                .children(match inline_handle {
                    Some(handle) => Some(
                        div()
                            .id("review-diff")
                            .flex_1()
                            .min_h_0()
                            .track_scroll(&handle)
                            .overflow_y_scroll()
                            .child(list)
                            .test_support()
                            .into_any_element(),
                    ),
                    // Folded into the file bar: no list box.
                    None if bar && !list_open => None,
                    None => Some(
                        div()
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
                        // below keeps the rest of the panel. A stack's list
                        // carries its repository heads too (frame 121).
                        .when(!full, |files| {
                            files.flex_shrink_0().max_h(px(if stacked { 240. } else { 220. }))
                        })
                        .child(list)
                        .into_any_element(),
                    ),
                })
                .when(!inline || refresh_failed, |body| body.child(view))
                .children(crate::changes_loading::changes_loading(
                    look,
                    &state.review.fetch,
                    session_id.as_ref(),
                )),
        )
        .children((!roomy).then(|| {
            let status = looks_status_el();
            let button = looks_button_el();
            // A stack's footer holds Looks good itself (frames 120, 121).
            (review.is_none() && !stacked && session_id.is_some()).then(|| {
                row()
                    .id("diff-looks-bar")
                    .test_support()
                    .h(px(34.))
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(rgb(look.line))
                    .child(status.unwrap_or_else(|| div().into_any_element()))
                    .child(div().flex_1())
                    .children(button)
            })
        }).flatten())
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
                    // Frames 119 to 121: Whole stack counts its pull requests,
                    // a layer steps to the layers beside it.
                    (None, Some(_)) if stacked => {
                        let links = crate::panel_stack::footer_links(&state.review, look, &stack_send, roomy)
                            .unwrap_or_default();
                        row()
                            .gap(px(12.))
                            .children(links)
                            .when(state.review.push_menu && !diffs.is_empty(), |wrap| {
                                wrap.relative()
                                    .child(deferred(push_scrim(ui)))
                                    .child(deferred(push_menu(ui, state, look, &diffs)))
                            })
                            .into_any_element()
                    }
                    (Some(_), Some(id)) => {
                        let (agent, you) = github_draft_counts(state, id);
                        // The id carries the counts the line shows. The
                        // line never gives way mid-phrase: when it can't sit
                        // beside the buttons it reads the bare "4 drafts".
                        let shaped = |value: &str, size: f32| {
                            let run = TextRun {
                                len: value.len(),
                                font: Font {
                                    family: ".SystemUIFont".into(),
                                    ..Default::default()
                                },
                                color: rgb(look.secondary).into(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            };
                            f32::from(
                                window
                                    .text_system()
                                    .shape_line(value.to_owned().into(), px(size), &[run], None)
                                    .width,
                            )
                        };
                        // The footer's padding and gaps, Submit review…,
                        // and Send when a draft waits (buttons pad 12 a side).
                        let send = state
                            .workspace
                            .session(id)
                            .map_or("agent", |found| found.agent.command());
                        let buttons = shaped("Submit review…", 13.) + 24.
                            + if sendable > 0 { shaped(&format!("Send to {send}"), 13.) + 24. + 8. } else { 0. };
                        let room = width - 20. - 16. - buttons;
                        let line = drafts_line(state, id, agent, you, |line| shaped(line, 11.5) <= room);
                        full_text(
                            secondary(look, line.clone())
                                .id(SharedString::from(format!("review-drafts-{agent}-{you}")))
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap(),
                            line,
                        )
                        .test_support()
                        .into_any_element()
                    }
                    _ => {
                        // Push & PR… opens one menu of every repo (DS-10);
                        // Add repo… stays here, quiet like it.
                        let menu = ui.clone();
                        let add = ui.clone();
                        row()
                            .gap(px(8.))
                            .when(!diffs.is_empty(), |line| {
                                line.child(
                                    div()
                                        .relative()
                                        .flex_shrink_0()
                                        .child(Quiet {
                                            button: Button::new("changes-push-menu")
                                                .small()
                                                .h(px(18.))
                                                .text_size(px(11.5))
                                                .label("Push & PR…")
                                                .accessibility_label("Push & PR…")
                                                .flex_shrink_0()
                                                .on_click(move |_, _, cx| {
                                                    menu.send(
                                                        AppAction::Review(ReviewAction::TogglePushMenu),
                                                        cx,
                                                    )
                                                }),
                                            color: look.secondary,
                                            fill: look.fill,
                                        })
                                        .when(state.review.push_menu, |wrap| {
                                            wrap.child(deferred(push_scrim(ui)))
                                                .child(deferred(push_menu(ui, state, look, &diffs)))
                                        }),
                                )
                            })
                            .children(session_id.clone().map(|session| Quiet {
                                button: Button::new("changes-add-repo")
                                    .small()
                                    .h(px(18.))
                                    .text_size(px(11.5))
                                    .label("Add repo…")
                                    .accessibility_label("Add repo…")
                                    .flex_shrink_0()
                                    .on_click(move |_, _, cx| {
                                        add.send(
                                            AppAction::SessionAddRepo(humata2_app::SessionAddRepoAction::Open(
                                                session.clone(),
                                            )),
                                            cx,
                                        )
                                    }),
                                color: look.secondary,
                                fill: look.fill,
                            }))
                            .into_any_element()
                    }
                })
                .child(div().flex_1())
                // D852: what was looked at in the view shown, then the drafts,
                // and Looks good, left of Send. Beside the terminal the panel is
                // too narrow for them and the buttons, so they take a row above.
                .children(looks_status_el().filter(|_| roomy && !stacked))
                // A stack's layer is looked at as one (D852); Whole stack sends.
                .children(looks_button_el().filter(|_| {
                    if stacked { !crate::panel_stack::whole_picked(&state.review) } else { roomy }
                }))
                // The plain Diff always shows Send, neutral and disabled when
                // nothing waits. A pull request review shows it only when a
                // draft waits, as a secondary button left of Submit review….
                .when_some(session_id.clone().filter(|_| {
                    if stacked {
                        crate::panel_stack::whole_picked(&state.review) || sendable > 0
                    } else {
                        review.is_none() || sendable > 0
                    }
                }), |footer, session| {
                    let agent = state
                        .workspace
                        .session(&session)
                        .map_or("agent", |found| found.agent.command());
                    footer
                        .child(
                            action(
                                ui,
                                "send-comments",
                                format!("Send to {agent}"),
                                AppAction::Comments(CommentsAction::SendAll(session)),
                                look,
                                review.is_none() && sendable > 0,
                            )
                            .when(sendable == 0, |button| {
                                button.text_color(rgb(look.secondary)).cursor_default()
                            })
                            .disabled(sendable == 0),
                        )
                })
                // DS-16: Submit review… sits in the footer, beside the
                // terminal and in full screen.
                .when_some(session_id.clone().filter(|_| review.is_some()), |footer, id| {
                    footer.child(
                        widgets::primary_in(
                            look,
                            ui,
                            "diff-submit-review",
                            "Submit review…",
                            AppAction::PrSubmit(humata2_pr_review::submit::SubmitAction::Open(id)),
                        )
                        .small()
                        .h(px(24.))
                        .flex_shrink_0()
                        .bg(rgb(look.accent))
                        .text_color(rgb(0xffffff)),
                    )
                }),
        )
}

/// The Diff panel's header: full screen adds New agent tab (UXSIMPLE 3-3).
fn diff_header(ui: &Rc<Ui>, state: &AppState, full: bool, extra: Option<Div>, look: Look) -> Div {
    let actions = full.then(|| agent_tab::button(ui, state, look));
    panel_header_actions(ui, state, Panel::Changes, full, extra, actions, look)
}
