//! The Diff panel: files, hunks, wrapped lines and comment cards.
use super::*;
use crate::comment_cards::{self, StartReattach};
use crate::document_tab::{Documents, LineFacts};
use crate::full_text::full_text;
use humata2_clients::comments::{Anchor, CommentId, Side};
use humata2_clients::model::{DiffBase, DiffLine, RepoBase};
use humata2_review::order::{OrderFile, bottom_up};
use humata2_review::{PlanRow, PlanRowNote, PushState, RepoPicker};
use gpui_kit::component::button::ButtonCustomVariant;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

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

/// A repository as it reads: a local repository's leading `/` is not drawn.
fn repo_label(repo: &str) -> &str {
    repo.strip_prefix('/').unwrap_or(repo)
}

/// A path split into its directory, with the trailing `/` (empty when there
/// is none), and its file name.
fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(at) => path.split_at(at + 1),
        None => ("", path),
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

/// Whether `line` is code (shown with a number, and open to comments), as
/// opposed to a hunk header or a note such as "No newline at end of file".
fn is_code(line: &DiffLine) -> bool {
    matches!(line.kind, LineKind::Context | LineKind::Added | LineKind::Removed)
}

/// Whether the file's mode changed (both sides have one, and they differ).
fn mode_changed(file: &humata2_clients::model::FileDiff) -> bool {
    file.old_mode != 0 && file.new_mode != 0 && file.old_mode != file.new_mode
}

/// "644 → 755" when only the permission digits changed, else the full modes.
fn mode_words(file: &humata2_clients::model::FileDiff) -> String {
    let (old, new) = (file.old_mode, file.new_mode);
    if old / 1000 == new / 1000 {
        format!("Mode {:03} → {:03}", old % 1000, new % 1000)
    } else {
        format!("Mode {old} → {new}")
    }
}

/// The one dim line a file's header carries when there is something to say
/// about it (DS-24): binary, renamed, mode change, new, deleted, or why its
/// lines could not be read (DS-36).
fn file_note(file: &humata2_clients::model::FileDiff) -> Option<String> {
    use humata2_clients::model::FileChange;
    let coded = file.lines.iter().any(is_code);
    let mut parts: Vec<String> = Vec::new();
    if file.binary {
        parts.push("Binary file".to_owned());
    }
    if file.change == FileChange::Renamed {
        parts.push(match (&file.old_path, file.similarity) {
            (Some(old), similarity) if similarity < 100 => {
                format!("Renamed from {old} ({similarity}% alike)")
            }
            (Some(old), _) => format!("Renamed from {old}"),
            (None, _) => "Renamed".to_owned(),
        });
    }
    if mode_changed(file) {
        parts.push(mode_words(file));
    }
    if !coded && !file.binary {
        match file.change {
            FileChange::Added => {
                let words = if file.added == 0 { "Empty file" } else { "New file" };
                parts.push(words.to_owned());
            }
            FileChange::Deleted => parts.push("Deleted".to_owned()),
            FileChange::Modified | FileChange::Renamed => {}
        }
    }
    if let Some(error) = &file.error {
        parts.push(error.clone());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
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
    // "bin" or "mode" stands where +a −r would be (DS-8).
    let stat_word = if file.binary {
        Some("bin")
    } else if mode_changed(file) && !file.lines.iter().any(is_code) {
        Some("mode")
    } else {
        None
    };
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
        // DS-8, DS-82: a letter, never colour alone; a modified file's
        // column stays empty so the names line up.
        .child({
            let letter = match file.change {
                humata2_clients::model::FileChange::Added => Some(("A", GREEN)),
                humata2_clients::model::FileChange::Deleted => Some(("D", RED)),
                humata2_clients::model::FileChange::Renamed => Some(("R", look.secondary)),
                humata2_clients::model::FileChange::Modified => None,
            };
            div()
                .w(px(14.))
                .flex_shrink_0()
                .flex()
                .justify_center()
                .children(letter.map(|(letter, color)| {
                    text(letter)
                        .id(SharedString::from(format!("change-file-mark {repo}/{path}")))
                        .test_support()
                        .text_size(px(11.))
                        .text_color(rgb(color))
                }))
        })
        .child(if short {
            text(path.rsplit('/').next().unwrap_or(path.as_str()).to_owned())
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .into_any_element()
        } else {
            // The name comes first and keeps its room; the directory,
            // dimmed, gives way before it does (DS-7, DS-47).
            let (dir, name) = split_path(&path);
            row()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .when(
                    crate::panel_stack::tier_of(&state.review, diff, file) == humata2_review::stack::Tier::Key,
                    |line| line.child(crate::panel_stack::key_dot(look, &diff.repo, &file.path)),
                )
                .child(
                    text(name.to_owned())
                        .id(SharedString::from(format!("change-file-name {repo}/{path}")))
                        .test_support()
                        .flex_shrink_0()
                        .max_w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                )
                .child(
                    text(dir.to_owned())
                        .text_color(rgb(look.tertiary))
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                )
                .into_any_element()
        })
        .when(comments > 0 && !short, |row| {
            row.child(
                widgets::row()
                    .gap(px(3.))
                    .flex_shrink_0()
                    .child(icon(IconName::MessageSquare, look.secondary, 11.))
                    .child(
                        text(comments.to_string())
                            .text_size(px(11.))
                            .text_color(rgb(look.secondary)),
                    ),
            )
        })
        .when_some(stat_word, |row, word| {
            row.child(
                text(word)
                    .text_size(px(11.))
                    .text_color(rgb(look.secondary)),
            )
        })
        .when(stat_word.is_none(), |row| {
            row.child(
                text(format!("+{}", file.added))
                    .text_size(px(11.))
                    .text_color(rgb(GREEN)),
            )
            .child(
                text(format!("−{}", file.removed))
                    .text_size(px(11.))
                    .text_color(rgb(RED)),
            )
        })
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

/// The box round the file list's rows (DS-84). A click in it gives the list the
/// focus, and while it has it the arrows and Enter move the list's cursor, open
/// the file there or fold its repository.
fn list_rows(ui: &Rc<Ui>, documents: &Documents, gap: f32) -> Div {
    let handle = documents.diff_list.clone();
    let key = |action: ReviewAction| {
        let ui = ui.clone();
        move |cx: &mut App| ui.send(AppAction::Review(action.clone()), cx)
    };
    let (down, up, open, collapse, expand) = (
        key(ReviewAction::ListDown),
        key(ReviewAction::ListUp),
        key(ReviewAction::ListOpen),
        key(ReviewAction::ListCollapse),
        key(ReviewAction::ListExpand),
    );
    col()
        .gap(px(gap))
        .track_focus(&handle)
        .key_context(crate::keys::DIFF_LIST)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| window.focus(&handle, cx))
        .on_action(move |_: &crate::keys::FileListDown, _, cx| down(cx))
        .on_action(move |_: &crate::keys::FileListUp, _, cx| up(cx))
        .on_action(move |_: &crate::keys::FileListOpen, _, cx| open(cx))
        .on_action(move |_: &crate::keys::FileListCollapse, _, cx| collapse(cx))
        .on_action(move |_: &crate::keys::FileListExpand, _, cx| expand(cx))
}

/// Whether the file list's cursor is on `repo`'s row (DS-84).
fn at_repo(cursor: Option<&humata2_review::ListCursor>, repo: &str) -> bool {
    matches!(cursor, Some(humata2_review::ListCursor::Repo(at)) if at == repo)
}

/// Whether the file list's cursor is on `path` of `repo` (DS-84).
fn at_file(cursor: Option<&humata2_review::ListCursor>, repo: &str, path: &str) -> bool {
    matches!(
        cursor,
        Some(humata2_review::ListCursor::File { repo: at, path: of }) if at == repo && of == path
    )
}

/// `row`, with the focus ring over it when the file list's cursor is `on` it
/// (DS-84). The ring is laid over the row, so nothing around it moves.
fn ringed(row: impl IntoElement, on: bool, look: Look) -> AnyElement {
    if !on {
        return row.into_any_element();
    }
    div()
        .relative()
        .child(row)
        .child(
            div()
                .id("diff-list-cursor")
                .test_support()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .rounded(px(5.))
                .border_2()
                .border_color(rgb(look.accent)),
        )
        .into_any_element()
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
#[allow(clippy::too_many_arguments)]
fn plan_list(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    diffs: &[&RepoDiff],
    picked: Option<(&RepoDiff, &humata2_clients::model::FileDiff)>,
    cursor: Option<&humata2_review::ListCursor>,
    plan: humata2_review::PlanLayout,
) -> Vec<AnyElement> {
    let mut groups = 0_usize;
    let mut outdated = 0_usize;
    let humata2_review::PlanLayout { rows: plan, skims } = plan;
    let send = &crate::panel_stack::sender(ui);
    // DS-67: one folded row per repository's skim files, which open in
    // place under it.
    let draw_skim = |skim: &humata2_review::PlanSkim| -> Vec<AnyElement> {
        let open = crate::panel_stack::skim_open(&state.review, &skim.repo);
        let paths: Vec<&str> = skim.paths.iter().map(String::as_str).collect();
        let mut drawn: Vec<AnyElement> =
            crate::panel_stack::skim_row(look, &skim.repo, &paths, open, send).into_iter().collect();
        if open && let Some(diff) = diffs.iter().find(|diff| diff.repo == skim.repo) {
            drawn.extend(
                skim.paths
                    .iter()
                    .filter_map(|path| diff.files.iter().find(|file| &file.path == path))
                    .map(|file| {
                        let on = picked.is_some_and(|(d, f)| d.repo == diff.repo && f.path == file.path);
                        ringed(
                            file_row(ui, state, look, diff, file, on, false),
                            at_file(cursor, &diff.repo, &file.path),
                            look,
                        )
                    }),
            );
        }
        drawn
    };
    let mut skims = skims.into_iter().peekable();
    let mut out = Vec::with_capacity(diffs.len() + plan.len());
    out.extend(
        diffs
            .iter()
            .enumerate()
            .map(|(index, diff)| {
                ringed(
                    plan_repo_line(ui, state, look, documents, index, diff),
                    at_repo(cursor, &diff.repo),
                    look,
                )
            }),
    );
    for (at, entry) in plan.into_iter().enumerate() {
        while let Some(skim) = skims.next_if(|skim| skim.after == at) {
            out.extend(draw_skim(&skim));
        }
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
                        .child(ringed(
                            file_row(ui, state, look, diff, file, on, false),
                            at_file(cursor, &repo, &path),
                            look,
                        ))
                        .children(rows)
                        .into_any_element(),
                );
            }
        }
    }
    for skim in skims {
        out.extend(draw_skim(&skim));
    }
    out
}

thread_local! {
    /// The change each file was last drawn at, by session and file, so a
    /// step scrolls once and a first draw does not scroll at all.
    static STEPPED: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

/// Notes that `key`'s file is drawn at `change`; true when that differs
/// from the last draw.
fn change_moved(key: &str, change: usize) -> bool {
    STEPPED.with(|held| {
        held.borrow_mut()
            .insert(key.to_owned(), change)
            .is_some_and(|before| before != change)
    })
}

/// Scrolls `handle` so `item` (as drawn now) sits at the top of its view.
fn put_at_top(handle: &ScrollHandle, item: &Bounds<Pixels>, cx: &mut App) {
    let view = handle.bounds();
    if view.size.height <= px(0.) {
        cx.refresh_windows();
        return;
    }
    let mut offset = handle.offset();
    offset.y = offset.y + (view.top() - item.top());
    offset.y = offset.y.min(px(0.)).max(-handle.max_offset().y);
    handle.set_offset(offset);
    cx.refresh_windows();
}

/// A small ghost icon button of the file bar and the file header.
fn step_button(
    ui: &Rc<Ui>,
    look: Look,
    id: &'static str,
    label: &'static str,
    tooltip: &'static str,
    glyph: IconName,
    action: ReviewAction,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .ghost()
        .small()
        .w(px(20.))
        .h(px(20.))
        .p_0()
        .flex_shrink_0()
        .icon(icon(glyph, look.secondary, 12.))
        .tooltip(tooltip)
        .accessibility_label(label)
        .on_click(move |_, _, cx| {
            // A click on a control of the bar is not a click on the bar.
            cx.stop_propagation();
            ui.send(AppAction::Review(action.clone()), cx);
        })
}

/// Open in editor: drawn dimmed and inert, as no editor client exists yet.
fn open_in_editor(look: Look) -> Button {
    Button::new("change-open-editor")
        .ghost()
        .small()
        .w(px(20.))
        .h(px(20.))
        .p_0()
        .flex_shrink_0()
        .opacity(0.5)
        .icon(icon(IconName::ArrowUpRight, look.secondary, 12.))
        .tooltip("Open in editor")
        .accessibility_label("Open in editor")
}

/// Wrap / Scroll for a file's header (DS-40): it flips the preference
/// Settings › Long lines sets, and the label names what a click switches to.
fn wrap_switch(ui: &Rc<Ui>, look: Look, wrapping: bool) -> Quiet {
    let ui = ui.clone();
    Quiet {
        button: Button::new("diff-wrap")
            .small()
            .h(px(18.))
            .text_size(px(11.5))
            .label(if wrapping { "Scroll" } else { "Wrap" })
            .accessibility_label(if wrapping { "Scroll long lines" } else { "Wrap long lines" })
            .tooltip("Wrap long lines  ⌥Z")
            .flex_shrink_0()
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                ui.send(AppAction::Settings(humata2_settings::SettingsAction::ToggleWrap), cx);
            }),
        color: look.secondary,
        fill: look.fill,
    }
}

/// "Change N of M ‹ ›" for a long file's header (DS-27).
fn change_stepper(ui: &Rc<Ui>, look: Look, change: usize, changes: usize) -> Div {
    row()
        .gap(px(2.))
        .flex_shrink_0()
        .child(
            secondary(look, format!("Change {} of {changes}", change + 1))
                .id("diff-change-count")
                .test_support()
                .flex_shrink_0()
                .whitespace_nowrap(),
        )
        .child(step_button(
            ui,
            look,
            "diff-prev-change",
            "Previous change",
            "Previous change  ⌃⌘↑",
            IconName::ChevronLeft,
            ReviewAction::PrevChange,
        ))
        .child(step_button(
            ui,
            look,
            "diff-next-change",
            "Next change",
            "Next change  ⌃⌘↓",
            IconName::ChevronRight,
            ReviewAction::NextChange,
        ))
}

/// The one-row file bar that stands in for the narrow list while a file is
/// open (DS-2, DS-3): chevron, `repo · name` and its folder, "N of M", the
/// step buttons and Open in editor. With `fold` it is the fold button: a click
/// on it opens the list, and `Some(true)` says the list is open. The inline
/// pull request review, whose list never folds, passes `None`: no chevron, no
/// click, and the bar only steps files across every pull request (DS-53).
fn file_bar(
    ui: &Rc<Ui>,
    look: Look,
    diffs: &[&RepoDiff],
    diff: &RepoDiff,
    file: &humata2_clients::model::FileDiff,
    fold: Option<bool>,
) -> AnyElement {
    let order: Vec<(&str, &str)> = diffs
        .iter()
        .flat_map(|each| each.files.iter().map(move |f| (each.repo.as_str(), f.path.as_str())))
        .collect();
    let at = order
        .iter()
        .position(|(repo, path)| *repo == diff.repo && *path == file.path)
        .map_or(1, |at| at + 1);
    let (dir, file_name) = split_path(&file.path);
    let toggle = ui.clone();
    row()
        .id("diff-file-bar")
        .h(px(30.))
        .px(px(8.))
        .gap(px(6.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .text_size(px(12.))
        .when_some(fold, |bar, open| {
            bar.role(Role::Button)
                .aria_label("Files")
                .aria_expanded(open)
                .cursor_pointer()
                .on_click(move |_, _, cx| {
                    toggle.send(AppAction::Review(ReviewAction::ToggleFileList), cx)
                })
                .child(div().flex_shrink_0().child(icon(
                    if open { IconName::ChevronDown } else { IconName::ChevronRight },
                    look.secondary,
                    12.,
                )))
        })
        .child(
            // The folder gives way first, then the repo; the name keeps its room.
            row()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .gap(px(4.))
                .child(full_text(
                    secondary(look, format!("{} ·", repo_label(&diff.repo)))
                        .id("diff-file-bar-repo")
                        .min_w_0()
                        .flex_shrink(4.)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                    repo_label(&diff.repo),
                ))
                .child(
                    text(file_name.to_owned())
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(full_text(
                    secondary(look, dir.to_owned())
                        .id("diff-file-bar-dir")
                        .min_w_0()
                        .flex_shrink(8.)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                    dir,
                )),
        )
        .child(
            secondary(look, format!("{at} of {}", order.len()))
                .id("diff-file-count")
                .test_support()
                .flex_shrink_0()
                .whitespace_nowrap(),
        )
        .child(step_button(
            ui,
            look,
            "diff-prev-file",
            "Previous file",
            "Previous file  ⌥⌘↑",
            IconName::ChevronLeft,
            ReviewAction::PrevFile,
        ))
        .child(step_button(
            ui,
            look,
            "diff-next-file",
            "Next file",
            "Next file  ⌥⌘↓",
            IconName::ChevronRight,
            ReviewAction::NextFile,
        ))
        .child(open_in_editor(look))
        .test_support()
        .into_any_element()
}

/// The "Filter files" field above a long list (RV-6). Esc clears it; with
/// nothing typed Esc goes on to do what it did before.
fn file_filter_field(ui: &Rc<Ui>, look: Look, documents: &Documents, session: &SessionId) -> AnyElement {
    let clear = ui.clone();
    let session = session.clone();
    div()
        .id("changes-file-filter")
        .test_support()
        .pb(px(4.))
        .on_action(move |_: &crate::keys::Dismiss, _, cx| {
            let typed = clear.read(|state| !state.review.file_filter(&session).is_empty());
            if typed {
                clear.send(
                    AppAction::Review(ReviewAction::FilterFiles(session.clone(), String::new())),
                    cx,
                );
            } else {
                cx.propagate();
            }
        })
        .child(
            gpui_kit::component::input::Input::new(&documents.file_filter())
                .aria_label("Filter files")
                .small()
                .min_h(px(26.))
                .bg(rgb(look.card))
                .border_color(rgb(look.line))
                .text_color(rgb(look.text)),
        )
        .into_any_element()
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
            .child(panel_header(ui, state, Panel::Changes, full, None, look))
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
    // D800: the stack's rows send through this.
    let stack_send = crate::panel_stack::sender(ui);
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
                            Some(session) => review_repo_head(state, look, session, diff, true),
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
        .children(filter_field)
        .child(list_rows(ui, documents, if inline { 4. } else { 1. }).children(rows));
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
            // DS-40: ⌥Z flips Wrap and Scroll while focus is in the file's
            // header or code. DS-15: full screen is the code's colour from
            // top to bottom.
            let view = view
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
            let start = code_start_of(gutter_width, cell_width, full, false);
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
            let code = if file.lines.iter().any(is_code) {
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
                        secondary(look, note.clone().unwrap_or_else(|| "No changes to show".to_owned()))
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
                    .child(icon(IconName::FileCode, look.secondary, 12.))
                    .child({
                        // `repo · dir/name`: the directory gives way first and
                        // the name stays; the block ends before "Open in editor".
                        let (dir, file_name) = split_path(&file.path);
                        row()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .gap(px(4.))
                            .child(secondary(look, format!("{} ·", repo_label(&diff.repo))).flex_shrink_0())
                            .child(full_text(
                                secondary(look, dir.to_owned())
                                    .id("diff-file-header-dir")
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis(),
                                dir,
                            ))
                            .child(
                                text(file_name.to_owned())
                                    .flex_shrink_0()
                                    .font_weight(FontWeight::MEDIUM),
                            )
                    })
                    .child(wrap_switch(ui, look, wrap))
                    .when(stepper, |header| {
                        header.child(change_stepper(ui, look, change, changes))
                    })
                    // With the file bar showing, Open in editor sits there.
                    .when(!bar, |header| header.child(open_in_editor(look))),
            )
            .children(note.map(|note| {
                secondary(look, note)
                    .id("change-file-note")
                    .test_support()
                    .flex_shrink_0()
                    .text_size(px(12.))
                    .px(px(10.))
                    .py(px(4.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
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
    let head = panel_header(
            ui,
            state,
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
    crate::panel_stack::stack_keys(col(), &state.review)
        .size_full()
        // A pull request review's header names the pull request instead
        // of a base to pick (a pull request's base is GitHub's); Full screen
        // and Close stay, as on every panel.
        .child(match review {
            Some(session) => panel_header(
                ui,
                state,
                Panel::Changes,
                full,
                Some(review_line(state, look, session, &review_base, diffs.len())),
                look,
            ),
            None => match crate::panel_stack::header_line(&state.review) {
                Some(line) => panel_header(
                    ui,
                    state,
                    Panel::Changes,
                    full,
                    Some(row().min_w_0().child(
                        secondary(look, line).id("stack-header").test_support(),
                    )),
                    look,
                ),
                None => head,
            },
        })
        // D800, DS-60 to DS-65: the layer list sits above the files.
        .children(crate::panel_stack::stack_top(&state.review, look, &stack_send))
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
                    file_bar(ui, look, &diffs, diff, file, Some(list_open))
                }))
                // DS-53: the inline review's list doesn't fold, so its bar is no fold
                // button; it steps files across every pull request of the review.
                .children(
                    shown
                        .filter(|_| inline)
                        .map(|(diff, file)| file_bar(ui, look, &diffs, diff, file, None)),
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
                        // below keeps the rest of the panel.
                        .when(!full, |files| files.flex_shrink_0().max_h(px(220.)))
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
                            drafts_line(state, id, agent, you),
                        )
                        .id(SharedString::from(format!("review-drafts-{agent}-{you}")))
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
                })
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

/// The footer's count of drafts for GitHub (DS-55): "4 drafts · 3 by
/// Copilot, 1 by you", naming the session's agent.
fn drafts_line(state: &AppState, session: &SessionId, agent: usize, you: usize) -> String {
    let total = agent + you;
    if total == 0 {
        return "No drafts".to_owned();
    }
    let name = state
        .workspace
        .sessions
        .iter()
        .find(|each| &each.id == session)
        .map_or("the agent", |each| match each.agent {
            humata2_clients::model::Agent::Claude => "Claude Code",
            other => other.name(),
        });
    let mut by = Vec::new();
    if agent > 0 {
        by.push(format!("{agent} by {name}"));
    }
    if you > 0 {
        by.push(format!("{you} by you"));
    }
    format!("{total} draft{} · {}", if total == 1 { "" } else { "s" }, by.join(", "))
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
        .child({
            let line = format!(
                "{name} against {base} · {repos} repo{}",
                if repos == 1 { "" } else { "s" }
            );
            full_text(
                secondary(look, line.clone())
                    .id("review-pr-line-text")
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis(),
                line,
            )
        }))
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
    cursor: Option<&humata2_review::ListCursor>,
    (font_size, line_height, cell_width): (f32, f32, f32),
    width: f32,
) -> AnyElement {
    let open = picked.is_some_and(|(held, _)| held.repo == diff.repo);
    // Frame 68: the repo row, its file and the file's code, 4 apart.
    let mut group = col().gap(px(4.)).child(ringed(
        review_repo_head(state, look, session, diff, open),
        at_repo(cursor, &diff.repo),
        look,
    ));
    if !open {
        return group.into_any_element();
    }
    let texts: Vec<String> = diff
        .files
        .iter()
        .map(|file| {
            file.lines
                .iter()
                .filter(|line| is_code(line))
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
    // DS-26: the open file's head stays at the top of the review's view
    // while its lines scroll, and leaves with the file's last line.
    let mut pinned: Option<(usize, usize, (String, ScrollHandle, f32))> = None;
    let mut children = 1;
    for index in bottom_up(&ordered) {
        let file = &diff.files[index];
        let on = picked.is_some_and(|(held, f)| held.repo == diff.repo && f.path == file.path);
        let ring = at_file(cursor, &diff.repo, &file.path);
        if on {
            let name = format!("{}/{}", diff.repo, file.path);
            let handle = documents.session_diff_view(&diff.session, &name).handle;
            let shift = pinned_shift(&name, &handle);
            let head = div()
                .when(shift > 0., |head| head.relative().top(px(shift)).bg(rgb(look.panel)))
                .child(ringed(file_row(ui, state, look, diff, file, on, true), ring, look));
            group = group.child(if shift > 0. {
                deferred(head).into_any_element()
            } else {
                head.into_any_element()
            });
            pinned = Some((children, children + 1, (name, handle, shift)));
        } else {
            group = group.child(ringed(file_row(ui, state, look, diff, file, on, true), ring, look));
        }
        children += 1;
        if on {
            children += 1;
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
    if let Some((head, code, (name, handle, shift))) = pinned {
        group = group.on_children_prepainted(move |bounds, _, _| {
            if let (Some(head), Some(code)) = (bounds.get(head), bounds.get(code)) {
                note_pinned(&name, &handle, shift, head, code);
            }
        });
    }
    group.into_any_element()
}

thread_local! {
    /// Per review file: where its head sits in the list's content when not
    /// pinned (its top, its height) and where its code ends.
    static PINNED: RefCell<HashMap<String, (f32, f32, f32)>> = RefCell::new(HashMap::new());
}

/// How far the open file's head is moved down so it stays at the top of the
/// review's view: none until its natural place scrolls off, and never past
/// the end of the file's code. Read from the last frame's place.
fn pinned_shift(name: &str, handle: &ScrollHandle) -> f32 {
    let Some((top, height, bottom)) = PINNED.with(|held| held.borrow().get(name).copied()) else {
        return 0.;
    };
    let scrolled = -f32::from(handle.offset().y);
    (scrolled - top).clamp(0., (bottom - height - top).max(0.))
}

/// Notes where the head and the code ended up this frame, in the list's own
/// coordinates, with the shift the head was drawn with taken back out.
fn note_pinned(
    name: &str,
    handle: &ScrollHandle,
    shift: f32,
    head: &Bounds<Pixels>,
    code: &Bounds<Pixels>,
) {
    let origin = f32::from(handle.bounds().origin.y) + f32::from(handle.offset().y);
    let top = f32::from(head.origin.y) - shift - origin;
    let bottom = f32::from(code.bottom()) - origin;
    PINNED.with(|held| {
        held.borrow_mut()
            .insert(name.to_owned(), (top, f32::from(head.size.height), bottom));
    });
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
    let scroll = documents.session_diff_view(&diff.session, &name);
    let LineFacts { digits, widest } = scroll.facts(&file.lines, line_facts);
    // Frame 68's code starts 36 in (wider only for longer numbers), the
    // number ending 8 before it. The sign keeps its own cell, taken from
    // the number column, so the number column is 36 less a cell. Metrics
    // takes the width with its 8 added. Three digits still fit in 36, so
    // the code starts where the design's does and the sign never overlaps it.
    let numbers = 36_f32.max(digits as f32 * cell_width + 6. + cell_width);
    let gutter_width = numbers - cell_width + 8.;
    let code_width = avail - numbers;
    let columns = ((code_width / cell_width).floor().max(4.)) as usize;
    // Scroll (DS-42): the number and sign stay put, only the code slides,
    // as far as the widest line needs.
    let max_shift = if state.review.wrap {
        0.
    } else {
        let start = code_start_of(gutter_width, cell_width, false, true);
        (widest as f32 * cell_width + cell_width) - (avail - start)
    };
    let shift = scroll.shift(max_shift);
    let sideways = {
        let scroll = scroll.clone();
        move |event: &ScrollEvent, _: &mut Window, cx: &mut App| {
            let delta = event.delta.pixel_delta(px(line_height));
            if delta.x != px(0.) && scroll.scroll_sideways(f32::from(delta.x), max_shift) {
                cx.refresh_windows();
            }
        }
    };
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
            old: false,
        },
        focus.and_then(|focus| focus.line().map(|line| (focus.place.side, line))),
        shift,
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
        .on_scroll_wheel(sideways)
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
        .child({
            let shown = div()
                .id(SharedString::from(format!("review-repo-branch-{}", diff.repo)))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(11.))
                .text_color(rgb(look.tertiary))
                .child(branch.to_owned());
            // No branch means nothing to name: an empty tooltip would only
            // cover the free space beside the repo.
            if branch.is_empty() {
                shown
            } else {
                full_text(shown, branch.to_owned())
            }
        })
        .into_any_element()
}

/// What the Diff panel says when nothing is picked and nothing failed: the
/// base decides the words.
/// The base the chip names while no diff has said which branch it is.
fn picked_base_name(state: &AppState, base: &DiffBase) -> String {
    // D800: a stack layer or the whole stack comes before every plain base.
    if let Some(scope) = stack_scope(state, base) {
        return scope.name().to_owned();
    }
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

/// What a stack shows in the Diff (D800): one layer, or the whole stack.
#[derive(Clone, Copy)]
enum StackScope {
    Layer,
    Whole,
}

impl StackScope {
    /// What the base chip names.
    fn name(self) -> &'static str {
        match self {
            Self::Layer => "this layer",
            Self::Whole => "the whole stack",
        }
    }

    /// What the panel says when nothing changed there.
    fn nothing(self) -> &'static str {
        match self {
            Self::Layer => "No changes in this layer",
            Self::Whole => "No changes in the stack",
        }
    }
}

/// Which part of a stack the open session's Diff shows, when it shows one: the
/// base is a stack's own (a spec whose bases are a range or an overlay), or the
/// session's plan carries a stack, which picks its bases itself (DS-62).
fn stack_scope(state: &AppState, base: &DiffBase) -> Option<StackScope> {
    let own = matches!(
        base,
        DiffBase::Spec(specs) if specs.iter().any(|spec| {
            matches!(spec.base, RepoBase::Range { .. } | RepoBase::Overlay { .. })
        })
    );
    if !own && !crate::panel_stack::hides_base_picker(&state.review) {
        return None;
    }
    let layer = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.review.layer_of(id));
    Some(if layer.is_some() { StackScope::Layer } else { StackScope::Whole })
}

fn nothing_against(ui: &Rc<Ui>, state: &AppState, look: Look, base: &DiffBase) -> impl IntoElement {
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
    let message = stack_scope(state, base).map_or(message, StackScope::nothing);
    // DS-32: centred, with the one way out: pick another base.
    col()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .child(secondary(look, message).p(px(12.)).id(id).test_support())
        .child(action(
            ui,
            "diff-compare-with",
            "Compare with…",
            AppAction::Review(ReviewAction::ToggleBaseMenu),
            look,
            false,
        ))
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

/// A repository whose read failed while the others were read (DS-35): its
/// name, why it failed, and Try again, which reads the session again.
fn repo_error_row(ui: &Rc<Ui>, look: Look, repo: &str, message: &str) -> AnyElement {
    let retry = ui.clone();
    col()
        .id(SharedString::from(format!("repo-error-{repo}")))
        .gap(px(2.))
        .px(px(4.))
        .pb(px(4.))
        .child(
            row()
                .gap(px(6.))
                .h(px(26.))
                .child(icon(IconName::FolderGit2, look.secondary, 12.))
                .child(
                    text(repo_label(repo).to_owned())
                        .font_weight(FontWeight::SEMIBOLD)
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                )
                .child(div().flex_1())
                .child(Quiet {
                    button: Button::new(SharedString::from(format!("repo-retry-{repo}")))
                        .small()
                        .h(px(18.))
                        .text_size(px(11.5))
                        .label("Try again")
                        .accessibility_label("Try again")
                        .flex_shrink_0()
                        .on_click(move |_, _, cx| {
                            retry.send(AppAction::Review(ReviewAction::Retry), cx)
                        }),
                    color: look.secondary,
                    fill: look.fill,
                }),
        )
        .child(secondary(look, message.to_owned()).pl(px(18.)))
        .test_support()
        .into_any_element()
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

/// The layer behind the Push & PR menu: a click on it closes the menu and
/// reaches nothing else.
fn push_scrim(ui: &Rc<Ui>) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("changes-push-scrim")
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| ui.send(AppAction::Review(ReviewAction::ClosePushMenu), cx))
        .test_support()
}

/// The Push & PR menu above the footer: one row per repository with its own
/// push button, which pushes that repository only.
fn push_menu(ui: &Rc<Ui>, state: &AppState, look: Look, diffs: &[&RepoDiff]) -> impl IntoElement {
    col()
        .id("changes-push-list")
        .role(Role::ListBox)
        .aria_label("Push & PR")
        .absolute()
        .bottom(px(28.))
        .left(px(0.))
        .w(px(280.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .children(diffs.iter().map(|diff| {
            row()
                .gap(px(8.))
                .h(px(28.))
                .px(px(8.))
                .child(full_text(
                    text(repo_label(&diff.repo).to_owned())
                        .id(SharedString::from(format!("push-list-repo-{}", diff.repo)))
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                    repo_label(&diff.repo).to_owned(),
                ))
                .child(div().flex_1())
                .child(push_control(ui, state, look, &diff.repo))
        }))
        .test_support()
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

/// A repository's one row in the Diff panel (DS-5): the chevron that folds
/// its files, its name, its clone's branch when that isn't the session's, the
/// base it is compared with when that differs from the header's, and its
/// totals at the right. Push & PR lives in the footer.
#[allow(clippy::too_many_arguments)]
fn repo_head(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    documents: &Documents,
    index: usize,
    diff: &RepoDiff,
    branch: &str,
    folded: bool,
) -> AnyElement {
    let (added, removed) = diff.files.iter().fold((0_u64, 0_u64), |(a, r), file| {
        (a + u64::from(file.added), r + u64::from(file.removed))
    });
    let own = state
        .workspace
        .session(&diff.session)
        .and_then(|session| {
            session.clones.iter().find(|clone| {
                clone.repo == diff.repo || clone.repo.rsplit('/').next() == Some(diff.repo.as_str())
            })
        })
        .map(|clone| clone.branch.clone())
        .filter(|own| !own.is_empty() && own != branch);
    let toggle = ui.clone();
    let repo = diff.repo.clone();
    let label = if folded { "Expand" } else { "Collapse" };
    // One line: when space runs out the branch gives way first, then the
    // base chip, then the repo name; the totals never shrink.
    row()
        .id(SharedString::from(format!("repo-row-{index}")))
        .gap(px(6.))
        .h(px(26.))
        .px(px(4.))
        .pr(px(8.))
        .min_w_0()
        .whitespace_nowrap()
        .child(
            Button::new(SharedString::from(format!("repo-toggle-{index}")))
                .ghost()
                .small()
                .w(px(20.))
                .h(px(20.))
                .flex_shrink_0()
                .icon(icon(
                    if folded { IconName::ChevronRight } else { IconName::ChevronDown },
                    look.tertiary,
                    12.,
                ))
                .accessibility_label(format!("{label} {}", diff.repo))
                .on_click(move |_, _, cx| {
                    toggle.send(
                        AppAction::Review(ReviewAction::ToggleRepo { repo: repo.clone() }),
                        cx,
                    )
                }),
        )
        .child(full_text(
            text(repo_label(&diff.repo).to_owned())
                .id(SharedString::from(format!("repo-name-{index}")))
                .font_weight(FontWeight::SEMIBOLD)
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis(),
            repo_label(&diff.repo).to_owned(),
        ))
        .children(own.map(|own| {
            full_text(
                secondary(look, own.clone())
                    .id(SharedString::from(format!("repo-branch-{index}")))
                    .test_support()
                    .text_color(rgb(look.tertiary))
                    .min_w_0()
                    .flex_shrink(8.)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis(),
                own,
            )
        }))
        .child(repo_base(ui, state, look, documents, index, diff))
        .child(div().flex_1())
        .child(
            row()
                .id(SharedString::from(format!("repo-totals-{index}")))
                .gap(px(6.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .child(
                    text(format!("+{added}"))
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_size(px(11.))
                        .text_color(rgb(GREEN)),
                )
                .child(
                    text(format!("−{removed}"))
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .text_size(px(11.))
                        .text_color(rgb(RED)),
                )
                .test_support(),
        )
        .test_support()
        .into_any_element()
}

/// A plan's line for one repository (RV-31 with RV-32): the plan's groups can
/// mix repositories, so each repository's base sits once above them.
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
        .child(text(repo_label(&diff.repo).to_owned()).font_weight(FontWeight::SEMIBOLD))
        .child(repo_base(ui, state, look, documents, index, diff))
        .child(div().flex_1())
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
                .flex_shrink_0()
                .icon(icon(IconName::ChevronsUpDown, tone, 12.))
                .tooltip(format!("{}: against {} · change", diff.repo, diff.base))
                .accessibility_label(spoken)
                .on_click(pick),
            color: tone,
            fill: look.fill,
        }
        .into_any_element()
    };
    // Not clipped: the menu below is a deferred child that must draw past it.
    // The chip's text ellipsizes, and it gives way before the repo name.
    div().relative().min_w_0().flex_shrink(4.).child(against).when_some(picker, |wrap, picker| {
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
        // RV-7, DS-50: anchored by its right edge to the chip, so a chip near
        // the panel's edge never pushes the menu past it.
        .right(px(0.))
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
    /// Two number columns, old then new (DS-19): full screen only.
    old: bool,
}

impl Metrics {
    /// How far in from the row's left edge the code starts.
    fn code_start(&self) -> f32 {
        code_start_of(self.gutter_width, self.cell_width, self.old, self.review)
    }
}

/// Where the code starts in a row (DS-19, DS-20): the number column, or two,
/// then the sign's cell and, outside a review, one space after it. A
/// review's rows keep frame 68's geometry, the sign in a cell of its own.
fn code_start_of(gutter_width: f32, cell_width: f32, old: bool, review: bool) -> f32 {
    let one = (gutter_width - 8.).max(0.);
    let numbers = if old { one * 2. } else { one };
    numbers + if review { cell_width } else { cell_width * 2. }
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
            .filter(|line| is_code(line))
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
    fn line(&self, lines: &[DiffLine], index: usize) -> (AnyElement, Vec<AnyElement>) {
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
                .pr(px(if review { (8. - cell_width).max(1.) } else { 4. }))
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
                    .w(px(if review { cell_width } else { cell_width * 2. }))
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
