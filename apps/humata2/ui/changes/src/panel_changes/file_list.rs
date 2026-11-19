use super::*;
use humata2_clients::comments::Anchor;
use humata2_clients::model::DiffLine;
use std::cell::RefCell;
use std::collections::HashMap;

/// A repository as it reads: a local repository's leading `/` is not drawn.
pub(super) fn repo_label(repo: &str) -> &str {
    repo.strip_prefix('/').unwrap_or(repo)
}

/// A path split into its directory, with the trailing `/` (empty when there
/// is none), and its file name.
pub(super) fn split_path(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(at) => path.split_at(at + 1),
        None => ("", path),
    }
}

/// Whether a comment the workspace carries is about `path` of `diff`'s
/// repository.
pub(super) fn on_path(comment: &Comment, diff: &RepoDiff, path: &str) -> bool {
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
pub(super) fn is_code(line: &DiffLine) -> bool {
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
pub(super) fn file_note(file: &humata2_clients::model::FileDiff) -> Option<String> {
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
pub(super) fn file_row(
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
                .text_ellipsis()
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
                        // Shrinks only once the folder (basis 0) has none left,
                        // so a long name ends in "…" before the counts.
                        .min_w_0()
                        .flex_shrink(1.)
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
pub(super) fn list_rows(ui: &Rc<Ui>, documents: &Documents, gap: f32) -> Div {
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
pub(super) fn at_repo(cursor: Option<&humata2_review::ListCursor>, repo: &str) -> bool {
    matches!(cursor, Some(humata2_review::ListCursor::Repo(at)) if at == repo)
}

/// Whether the file list's cursor is on `path` of `repo` (DS-84).
pub(super) fn at_file(cursor: Option<&humata2_review::ListCursor>, repo: &str, path: &str) -> bool {
    matches!(
        cursor,
        Some(humata2_review::ListCursor::File { repo: at, path: of }) if at == repo && of == path
    )
}

/// `row`, with the focus ring over it when the file list's cursor is `on` it
/// (DS-84). The ring is laid over the row, so nothing around it moves.
pub(super) fn ringed(row: impl IntoElement, on: bool, look: Look) -> AnyElement {
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

thread_local! {
    /// The change each file was last drawn at, by session and file, so a
    /// step scrolls once and a first draw does not scroll at all.
    static STEPPED: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

/// Notes that `key`'s file is drawn at `change`; true when that differs
/// from the last draw.
pub(super) fn change_moved(key: &str, change: usize) -> bool {
    STEPPED.with(|held| {
        held.borrow_mut()
            .insert(key.to_owned(), change)
            .is_some_and(|before| before != change)
    })
}

/// Scrolls `handle` so `item` (as drawn now) sits at the top of its view.
pub(super) fn put_at_top(handle: &ScrollHandle, item: &Bounds<Pixels>, cx: &mut App) {
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

/// Open in editor: the ↗ icon (DS-24), inert, as no editor client
/// exists yet.
pub(super) fn open_in_editor(look: Look) -> Button {
    Button::new("change-open-editor")
        .ghost()
        .small()
        .w(px(20.))
        .h(px(20.))
        .p_0()
        .flex_shrink_0()
        .icon(icon(IconName::ArrowUpRight, look.secondary, 12.))
        .tooltip("Open in editor")
        .accessibility_label("Open in editor")
}

/// Wrap / Scroll for a file's header (DS-40): it flips the preference
/// Settings › Long lines sets, and the label names what a click switches to.
pub(super) fn wrap_switch(ui: &Rc<Ui>, look: Look, wrapping: bool) -> Quiet {
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
pub(super) fn change_stepper(ui: &Rc<Ui>, look: Look, change: usize, changes: usize) -> Div {
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
pub(super) fn file_bar(
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
            // The folder gives way first: below 40 pt it wraps onto a hidden
            // second line instead of showing a sliver. The repo keeps a quarter
            // of the row and the name most of the rest, each ending in "…";
            // the "·" after the repo always shows.
            row()
                .flex_1()
                .min_w_0()
                .h(px(18.))
                .flex_wrap()
                .overflow_hidden()
                .gap(px(4.))
                .child(full_text(
                    secondary(look, repo_label(&diff.repo))
                        .id("diff-file-bar-repo")
                        .flex_shrink_0()
                        .max_w(relative(0.25))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                    repo_label(&diff.repo),
                ))
                .child(secondary(look, "·").flex_shrink_0())
                .child(full_text(
                    text(file_name.to_owned())
                        .id("diff-file-bar-name")
                        .flex_shrink_0()
                        // With the repo's quarter, the "·" and the gaps this
                        // still fits line one at any panel width over ~180 pt,
                        // so only the folder ever wraps away.
                        .max_w(relative(0.68))
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .font_weight(FontWeight::MEDIUM),
                    file_name.to_owned(),
                ))
                .child(full_text(
                    secondary(look, dir.to_owned())
                        .id("diff-file-bar-dir")
                        .min_w(px(40.))
                        .flex_shrink(1.)
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
pub(super) fn file_filter_field(ui: &Rc<Ui>, look: Look, documents: &Documents, session: &SessionId) -> AnyElement {
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
