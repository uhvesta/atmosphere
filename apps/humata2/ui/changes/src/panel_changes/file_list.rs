use super::*;
use humata2_clients::comments::Anchor;
use humata2_clients::model::DiffLine;
use std::cell::RefCell;
use std::collections::HashMap;

/// A repository as it reads: a local repository's leading `/` is not drawn.
pub(super) fn repo_label(repo: &str) -> &str {
    repo.strip_prefix('/').unwrap_or(repo)
}

/// The short name of a repository: `humata2` for `org/humata2`, `large` for `/large`.
fn short_repo_name(repo: &str) -> &str {
    let label = repo_label(repo);
    label.rsplit('/').next().unwrap_or(label)
}

/// How a repository is displayed beside the others in `all`: only the short
/// repo name (`humata2` for `org/humata2`), unless two or more repositories in
/// `all` share that short name, in which case `org/repo` is shown to
/// disambiguate them.
pub(super) fn display_repo_name<'a, 'b>(
    repo: &'a str,
    all: impl IntoIterator<Item = &'b str>,
) -> &'a str {
    let short = short_repo_name(repo);
    let collision = all
        .into_iter()
        .any(|other| other != repo && short_repo_name(other) == short);
    if collision { repo_label(repo) } else { short }
}

/// Effective width of the full-screen Diff file list column for a panel of
/// `panel_width` logical pixels: clamped to `[MIN_FILE_LIST_WIDTH, MAX_FILE_LIST_WIDTH]`
/// and bounded so the code pane keeps at least 160 px when the window is narrow.
pub(super) fn effective_file_list_width(saved_width: f32, panel_width: f32) -> f32 {
    let clamped = humata2_review::clamp_file_list_width(saved_width);
    let max_for_window = (panel_width - 160.).max(humata2_review::MIN_FILE_LIST_WIDTH);
    clamped.min(max_for_window)
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
            .flatten()
            .filter(|comment| !humata2_review::comments::is_agent_note(comment))
            .count()
    });
    sent + held
}

/// How many notes an agent left on `path` of `repo`, which a file row counts
/// apart from the drafts (UXSIMPLE 3-4).
pub(super) fn file_notes(state: &AppState, diff: &RepoDiff, path: &str) -> usize {
    state.session.open.as_ref().map_or(0, |session| {
        state
            .comments
            .on_file(session, &diff.repo, path)
            .values()
            .flatten()
            .filter(|comment| humata2_review::comments::is_agent_note(comment))
            .count()
    })
}

/// How many comments are on `path` of `repo` when every one of them is done
/// (the "✓ 3 done" a file row shows), else `None`.
pub(super) fn file_done(state: &AppState, diff: &RepoDiff, path: &str) -> Option<usize> {
    if diff.comments.iter().any(|comment| on_path(comment, diff, path)) {
        return None;
    }
    let session = state.session.open.as_ref()?;
    let on_file = state.comments.on_file(session, &diff.repo, path);
    crate::comment_cards::all_done(on_file.values().flatten().copied())
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
    file_row_also(ui, state, look, diff, file, on, short, "")
}

/// [`file_row`] with `also` in grey after the directory (D867): "also 2" for
/// a file a higher layer lists too; empty for none.
#[allow(clippy::too_many_arguments)]
pub(super) fn file_row_also(
    ui: &Rc<Ui>,
    state: &AppState,
    look: Look,
    diff: &RepoDiff,
    file: &humata2_clients::model::FileDiff,
    on: bool,
    short: bool,
    also: &str,
) -> impl IntoElement {
    let ui = ui.clone();
    let repo = diff.repo.clone();
    let path = file.path.clone();
    let label = format!("{repo}/{path}");
    let comments = file_comments(state, diff, &file.path);
    let notes = file_notes(state, diff, &file.path);
    let done = file_done(state, diff, &file.path);
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
    // Frames 119 to 121: a stack's rows sit 24 pt apart, with a smaller,
    // quieter circle.
    let stacked = crate::panel_stack::header_line(&state.review).is_some();
    let tick = (!short && !reviewing(state, &diff.session)).then(|| {
        let seen = humata2_review::looks::file_look(&state.review, &diff.session, &repo, &path, file);
        look_tick(&ui, look, &repo, &path, seen, stacked)
    });
    // The inline pull request review (`short`) has no Looks good (D852), so
    // its file row leads with the file glyph where the tick would stand, and
    // the letter, if any, follows the name (frame 68).
    let glyph = short.then(|| {
        div()
            .id(SharedString::from(format!("change-file-glyph {repo}/{path}")))
            .test_support()
            .flex_shrink_0()
            .child(icon(IconName::FileCode, look.secondary, 12.))
    });
    // DS-8, DS-82: a letter, never colour alone; a modified file's column
    // stays empty. It stands after the name so the circle sits 8 pt from it.
    let mark = {
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
        }.into_any_element();
    let row_el = row()
        .id(SharedString::from(format!("change-file {repo}/{path}")))
        .group(group.clone())
        .relative()
        .aria_selected(on)
        .test_support()
        .gap(px(6.))
        .h(px(if short || stacked { 24. } else { 25. }))
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
        .children(tick)
        .children(glyph)
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
                .child(
                    text(name.to_owned())
                        .id(SharedString::from(format!("change-file-name {repo}/{path}")))
                        .test_support()
                        // Shrinks only once the folder (basis 0) has none left,
                        // so a long name ends in "…" before the counts.
                        .min_w_0()
                        .flex_shrink_0()
                        .max_w_full()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                )
                .child(
                    text(dir.to_owned())
                        .text_color(rgb(look.tertiary))
                        // Gives way first: far more eager to shrink than the name.
                        .flex_shrink(1000.)
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis(),
                )
                .when(!also.is_empty(), |line| {
                    line.child(
                        text(also.to_owned())
                            .id(SharedString::from(format!("change-file-also {repo}/{path}")))
                            .test_support()
                            .flex_shrink_0()
                            .text_color(rgb(look.tertiary)),
                    )
                })
                .into_any_element()
        })
        .child(mark)
        // DS-67: a key file's chip stands before its counts.
        .when(
            !short && crate::panel_stack::tier_of(&state.review, diff, file) == humata2_review::stack::Tier::Key,
            |row| row.child(crate::panel_stack::key_chip(look, format!("change-key {}/{}", diff.repo, file.path))),
        )
        .when_some(done.filter(|_| !short), |row, done| {
            row.child(
                text(format!("✓ {done} done"))
                    .id(SharedString::from(format!("change-file-done {repo}/{path}")))
                    .test_support()
                    .flex_shrink_0()
                    .text_size(px(11.))
                    .text_color(rgb(GREEN)),
            )
        })
        .when(comments > 0 && !short && done.is_none(), |row| {
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
        .when(notes > 0 && !short && done.is_none(), |row| {
            row.child(
                widgets::row()
                    .id(SharedString::from(format!("change-file-notes {repo}/{path}")))
                    .test_support()
                    .gap(px(3.))
                    .flex_shrink_0()
                    .child(icon(IconName::MessageSquare, LOOK_ORANGE, 11.))
                    .child(text(notes.to_string()).text_size(px(11.)).text_color(rgb(LOOK_ORANGE))),
            )
        })
        .when_some(stat_word, |row, word| {
            row.child(
                text(word)
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(11.))
                    .text_color(rgb(look.secondary)),
            )
        })
        .when(stat_word.is_none(), |row| {
            row.child(
                text(format!("+{}", file.added))
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(11.))
                    .text_color(rgb(GREEN)),
            )
            .child(
                text(format!("−{}", file.removed))
                    .flex_shrink_0()
                    .whitespace_nowrap()
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
        });
    // The row's role is ListBoxOption, which wins over the label's role.
    full_text(row_el, label).role(Role::ListBoxOption)
}

/// The green of a look (D852, UXSIMPLE frames 3-2 to 4-3).
pub(super) const LOOK_GREEN: u32 = 0x28a745;

/// The orange of a file or layer that changed since it was looked at.
pub(super) const LOOK_ORANGE: u32 = 0xd98200;

/// The 14 pt circle before a file's name (D852): empty, or green with a
/// white tick once the file was looked at. A click marks or clears that one
/// file and leaves the row's own click (opening the file) alone.
fn look_tick(
    ui: &Rc<Ui>,
    look: Look,
    repo: &str,
    path: &str,
    seen: humata2_review::looks::FileLook,
    quiet: bool,
) -> impl IntoElement {
    use humata2_review::looks::FileLook;
    let ui = ui.clone();
    let (repo_of, path_of) = (repo.to_owned(), path.to_owned());
    div()
        .id(SharedString::from(format!("change-file-look {repo}/{path}")))
        .role(Role::CheckBox)
        .aria_label(match seen {
            FileLook::Never => "Not looked at",
            FileLook::Looked => "Looked at",
            FileLook::Changed => "Changed since you looked",
        })
        .test_support()
        .size(px(if quiet { 13. } else { 14. }))
        .flex_shrink_0()
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        // A file read gives way to the name 8 pt after the circle.
        .mr(px(2.))
        .map(|tick| match seen {
            FileLook::Looked => tick
                .bg(rgb(LOOK_GREEN))
                .child(icon(IconName::Check, 0xffffff, 9.)),
            FileLook::Never => tick
                .border_1()
                .border_color(rgb(if quiet { look.line } else { look.tertiary })),
            // A 1.5 pt amber ring on the pale amber fill.
            FileLook::Changed => tick
                .border(px(1.5))
                .border_color(rgb(LOOK_ORANGE))
                .bg(rgb(look.attention_bg)),
        })
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(
                AppAction::Review(ReviewAction::ToggleLooked {
                    repo: repo_of.clone(),
                    path: path_of.clone(),
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

/// Open in editor in words, quiet, for a stack's file header (frames 119 to
/// 121); inert, like the icon.
pub(super) fn open_in_editor_words(look: Look) -> Quiet {
    Quiet {
        button: Button::new("change-open-editor")
            .small()
            .h(px(18.))
            .text_size(px(11.5))
            .label("Open in editor")
            .accessibility_label("Open in editor")
            .flex_shrink_0(),
        color: look.secondary,
        fill: look.fill,
    }
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
    looked: bool,
    compact: bool,
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
    let shown_repo = display_repo_name(&diff.repo, diffs.iter().map(|each| each.repo.as_str()));
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
                .when(!compact, |line| {
                    line.child(full_text(
                        secondary(look, shown_repo)
                            .id("diff-file-bar-repo")
                            .flex_shrink_0()
                            .max_w(relative(0.25))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis(),
                        repo_label(&diff.repo),
                    ))
                    .child(secondary(look, "·").flex_shrink_0())
                })
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
                .when(!compact, |line| {
                    line.child(full_text(
                        secondary(look, dir.to_owned())
                            .id("diff-file-bar-dir")
                            .min_w(px(40.))
                            .flex_shrink(1.)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis(),
                        dir,
                    ))
                }),
        )
        .when(looked, |bar| {
            bar.child(
                text("✓ You looked at this")
                    .id("diff-file-looked")
                    .test_support()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_color(rgb(LOOK_GREEN)),
            )
        })
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

#[cfg(test)]
mod tests {
    use super::{
        display_repo_name, effective_file_list_width, file_note, repo_label, split_path,
    };
    use humata2_clients::model::{DiffLine, FileChange, FileDiff, LineKind};

    fn code_line(text: &str) -> DiffLine {
        DiffLine {
            kind: LineKind::Added,
            old: None,
            new: Some(1),
            text: text.to_owned(),
        }
    }

    #[test]
    fn repo_label_strips_one_leading_slash() {
        assert_eq!(repo_label("/large"), "large");
        assert_eq!(repo_label("acme/api"), "acme/api");
        assert_eq!(repo_label("api"), "api");
    }

    #[core::prelude::v1::test]
    fn display_repo_name_omits_owner_when_unique_and_disambiguates_collisions() {
        // Unique repo names omit `org/`.
        assert_eq!(display_repo_name("org/humata2", ["org/humata2"]), "humata2");
        assert_eq!(
            display_repo_name("org/humata2", ["org/humata2", "acme/web"]),
            "humata2"
        );
        assert_eq!(
            display_repo_name("acme/web", ["org/humata2", "acme/web"]),
            "web"
        );
        assert_eq!(display_repo_name("/large", ["/large"]), "large");

        // Colliding short repo names show `org/repo`, while non-colliding siblings stay short.
        let repos = ["acme/api", "other/api", "acme/web"];
        assert_eq!(display_repo_name("acme/api", repos), "acme/api");
        assert_eq!(display_repo_name("other/api", repos), "other/api");
        assert_eq!(display_repo_name("acme/web", repos), "web");

        // Local repo colliding with an org repo disambiguates the org repo.
        let mixed = ["/api", "acme/api"];
        assert_eq!(display_repo_name("/api", mixed), "api");
        assert_eq!(display_repo_name("acme/api", mixed), "acme/api");
    }

    #[core::prelude::v1::test]
    fn file_list_width_clamps_to_bounds() {
        use humata2_review::{
            DEFAULT_FILE_LIST_WIDTH, MAX_FILE_LIST_WIDTH, MIN_FILE_LIST_WIDTH,
        };

        assert_eq!(effective_file_list_width(100., 1200.), MIN_FILE_LIST_WIDTH);
        assert_eq!(effective_file_list_width(360., 1200.), 360.);
        assert_eq!(effective_file_list_width(900., 1200.), MAX_FILE_LIST_WIDTH);
        assert_eq!(
            effective_file_list_width(f32::NAN, 1200.),
            DEFAULT_FILE_LIST_WIDTH
        );
        // Narrow panel still keeps room for the code pane while honoring min width.
        assert_eq!(effective_file_list_width(500., 500.), 340.);
        assert_eq!(effective_file_list_width(500., 300.), MIN_FILE_LIST_WIDTH);
    }

    #[test]
    fn split_path_separates_directory_and_file_name() {
        assert_eq!(split_path("src/net/client.rs"), ("src/net/", "client.rs"));
        assert_eq!(split_path("README.md"), ("", "README.md"));
        assert_eq!(split_path("a/b/c/d.rs"), ("a/b/c/", "d.rs"));
    }

    #[test]
    fn file_note_covers_binary_rename_mode_empty_deleted_and_error() {
        let binary = FileDiff {
            path: "logo.png".into(),
            binary: true,
            ..FileDiff::default()
        };
        assert!(file_note(&binary).is_some_and(|note| note.to_lowercase().contains("binary")));

        let renamed = FileDiff {
            path: "src/http.rs".into(),
            change: FileChange::Renamed,
            old_path: Some("src/net.rs".into()),
            similarity: 92,
            lines: vec![code_line("fn a() {}")],
            ..FileDiff::default()
        };
        let note = file_note(&renamed).expect("rename note");
        assert!(note.contains("src/net.rs") && note.contains("92%"), "{note}");

        let exact_rename = FileDiff {
            similarity: 100,
            ..renamed
        };
        let note = file_note(&exact_rename).expect("exact rename note");
        assert!(note.contains("src/net.rs") && !note.contains("100%"), "{note}");

        let mode_only = FileDiff {
            path: "run.sh".into(),
            old_mode: 100_644,
            new_mode: 100_755,
            ..FileDiff::default()
        };
        let note = file_note(&mode_only).expect("mode note");
        assert!(note.contains("644") && note.contains("755"), "{note}");

        let empty_added = FileDiff {
            path: "empty.txt".into(),
            change: FileChange::Added,
            ..FileDiff::default()
        };
        assert!(file_note(&empty_added).is_some());

        let deleted = FileDiff {
            path: "old.rs".into(),
            change: FileChange::Deleted,
            ..FileDiff::default()
        };
        assert!(file_note(&deleted).is_some());

        let errored = FileDiff {
            path: "bad.rs".into(),
            error: Some("clone locked".into()),
            ..FileDiff::default()
        };
        assert_eq!(file_note(&errored).as_deref(), Some("clone locked"));

        let plain = FileDiff {
            path: "src/lib.rs".into(),
            added: 1,
            lines: vec![code_line("let x = 1;")],
            ..FileDiff::default()
        };
        assert_eq!(file_note(&plain), None);
    }
}
