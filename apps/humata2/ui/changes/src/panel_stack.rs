//! A stack review's layer list at the top of the Diff panel (D800, DS-60 to
//! DS-62, DS-65): Whole stack, then the layers from the top down, the
//! picked one highlighted with its purpose, a moved repository with one
//! action to ask the agent to send it again.
//!
//! The Diff panel draws [`stack_top`] above its files, [`header_line`] in
//! its header, [`skim_row`] at the end of each repository and [`key_chip`] on
//! key files, hides its base picker when [`hides_base_picker`] says so, and
//! takes the layer keys through [`stack_keys`]. This file only draws and
//! sends [`ReviewAction`]s.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{TestSupportExt as _, TestTextExt as _};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::{FileDiff, RepoDiff};
use humata2_review::stack::{StackHeader, StackRow, StackRows, Tier, skim_label};
use humata2_review::{ReviewAction, ReviewState};

use crate::bridge::Ui;
use crate::chrome::icon;
use crate::full_text::full_text;
use crate::theme::Look;
use crate::widgets::{self, col, row};

/// Sends an action to the app; the panel passes [`sender`], tests a log.
pub type Sender = Rc<dyn Fn(AppAction, &mut App)>;

/// The app's own [`Sender`].
pub fn sender(ui: &Rc<Ui>) -> Sender {
    let ui = ui.clone();
    Rc::new(move |action, cx| ui.send(action, cx))
}

/// How many rows show before the list scrolls.
const VISIBLE_ROWS: f32 = 6.;
/// One row's height; the picked row's purpose adds a line to it.
const ROW_HEIGHT: f32 = 26.;

thread_local! {
    /// The picked row the list was last scrolled to, so a redraw does not
    /// pull the list back while the person scrolls it.
    static REVEALED: Cell<Option<usize>> = const { Cell::new(None) };
    /// The layer list's scroll, kept between frames so it stays where the
    /// person left it.
    static SCROLL: RefCell<Option<ScrollHandle>> = const { RefCell::new(None) };
}

fn list_scroll() -> ScrollHandle {
    SCROLL.with(|held| held.borrow_mut().get_or_insert_with(ScrollHandle::new).clone())
}

fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}

fn secondary(look: Look, value: impl Into<SharedString>) -> Div {
    text(value)
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
}

/// The header's comparing line (DS-62): "Whole stack · 3 layers" or
/// "Layer 2 of 3 · journal".
pub fn stack_header_line(header: &StackHeader) -> String {
    match header {
        StackHeader::Whole { layers } => format!(
            "Whole stack \u{b7} {layers} {}",
            if *layers == 1 { "layer" } else { "layers" }
        ),
        StackHeader::Layer { number, of, title } => {
            format!("Layer {number} of {of} \u{b7} {title}")
        }
    }
}

/// What a row says after its title (UXSIMPLE 2-2, owner's mark): a layer
/// names the repositories it touches, "api · ledger · sdk"; Whole stack
/// counts them and its files, "5 repos · 23 files".
fn row_note(entry: &StackRow) -> String {
    if entry.number.is_some() {
        return entry
            .repos
            .iter()
            .map(|repo| short_repo(repo))
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
    }
    let repos = entry.repos.len();
    format!(
        "{repos} {} \u{b7} {}",
        if repos == 1 { "repo" } else { "repos" },
        files_label(entry.files)
    )
}

fn files_label(files: u32) -> String {
    format!("{files} {}", if files == 1 { "file" } else { "files" })
}

/// The layer list. A row's click picks its layer; a moved repository's
/// "Ask the agent to send it again" asks once (DS-65). `scroll` is the
/// list's scroll handle, which lives between frames.
pub fn stack_list(
    look: Look,
    review: &ReviewState,
    comments: &humata2_review::comments::CommentsState,
    rows: &StackRows,
    scroll: &ScrollHandle,
    send: &Sender,
    width: f32,
) -> impl IntoElement {
    if REVEALED.with(Cell::get) != Some(rows.picked) {
        REVEALED.with(|held| held.set(Some(rows.picked)));
        scroll.scroll_to_item(rows.picked);
    }
    col()
        .id("stack-list")
        .role(Role::ListBox)
        .aria_label("Layers")
        .test_support()
        .max_h(px(VISIBLE_ROWS * ROW_HEIGHT))
        .flex_shrink_0()
        .track_scroll(scroll)
        .overflow_y_scroll()
        .children(rows.rows.iter().enumerate().map(|(index, entry)| {
            stack_row(look, review, comments, rows, index, entry, index == rows.picked, send, width)
        }))
}

fn stack_row(
    look: Look,
    review: &ReviewState,
    comments: &humata2_review::comments::CommentsState,
    rows: &StackRows,
    index: usize,
    entry: &StackRow,
    on: bool,
    send: &Sender,
    width: f32,
) -> impl IntoElement {
    let target = (!entry.title.is_empty() && entry.number.is_some()).then(|| entry.title.clone());
    let click = send.clone();
    let (name, label) = match entry.number {
        Some(number) => (
            format!("{number}"),
            format!("Layer {number}: {}", entry.title),
        ),
        // Whole stack's badge is the ≡ mark (frame 119).
        None => ("\u{2261}".to_owned(), "Whole stack".to_owned()),
    };
    let title = if entry.number.is_some() {
        entry.title.clone()
    } else {
        "Whole stack".to_owned()
    };
    let note = row_note(entry);
    let look_note = layer_look_note(review, rows, entry);
    // About how wide the parts draw, in points: the row's inside less the
    // number column, the title (semibold 6.7 a character) and the status
    // (5.7), with the gaps between them.
    let inside = width - 34. - 22.;
    let title_w = title.chars().count() as f32 * 6.7;
    let counts = layer_counts(review, comments, entry);
    let has_counts = counts.is_some();
    let status_w = look_note.map_or(0., |(words, _)| words.chars().count() as f32 * 5.7 + 6.)
        + counts.as_ref().map_or(0., |words| words.chars().count() as f32 * 5.7 + 6.);
    let first_word = note.split(' ').next().map_or(0., |word| word.chars().count() as f32 * 5.9);
    let note_fits = inside - title_w - status_w - 6. >= first_word;
    let head = row()
        .id(if on {
            SharedString::from("stack-row-picked")
        } else {
            SharedString::from(format!("stack-row-head-{index}"))
        })
        .test_support()
        .gap(px(6.))
        .h(px(ROW_HEIGHT - 1.))
        .whitespace_nowrap()
        .child(badge(look, index, &name, on))
        .child(
            text(title)
                .flex_shrink_0()
                .when(entry.number.is_none() || on, |title| {
                    title.font_weight(FontWeight::SEMIBOLD)
                }),
        )
        // The note gives way first, ending in an ellipsis; hover names it
        // in full. It shows only when at least its first word fits whole
        // beside the title and the look's status; otherwise it is left out.
        .children(note_fits.then(|| {
            full_text(
                secondary(look, note.clone())
                    .id(SharedString::from(format!("stack-row-note-{index}")))
                    .test_support()
                    .text_color(rgb(look.tertiary))
                    .min_w(px(first_word))
                    .overflow_hidden()
                    .text_ellipsis(),
                note.clone(),
            )
        }))
        .children(counts.map(|words| {
            text(words)
                .id(SharedString::from(format!("stack-row-counts-{index}")))
                .test_support()
                .flex_shrink_0()
                .ml_auto()
                .text_size(px(11.))
                .text_color(rgb(look.accent))
        }))
        .children(look_note.map(|(words, color)| look_note_el(index, words, color, has_counts)));
    let mut body = col().w_full().child(head);
    // Without a purpose the picked row stays one line, like the others.
    if on && !entry.purpose.is_empty() {
        body = body.child(
            secondary(look, entry.purpose.clone())
                .id("stack-row-purpose")
                .test_support()
                .pl(px(22.))
                .pb(px(4.)),
        );
    }
    let picked_row = col()
        .id(SharedString::from(format!("stack-row-{index}")))
        .role(Role::ListBoxOption)
        .aria_label(label)
        .aria_selected(on)
        .test_support()
        .px(px(8.))
        .rounded(px(5.))
        .cursor_pointer()
        .when(on, |row| row.bg(rgb(look.fill)))
        .on_click(move |_, _, cx| {
            click(AppAction::Review(ReviewAction::PickLayer(target.clone())), cx);
        })
        .child(body);
    col().child(picked_row)
}

/// A row's round number (frames 119 to 121): 16 pt, a thin ring with the
/// number in tertiary, or filled with the accent and a white number once
/// picked. Whole stack's reads ≡.
fn badge(look: Look, index: usize, number: &str, on: bool) -> impl IntoElement {
    let badge = div()
        .id(SharedString::from(format!("stack-row-badge-{index}")))
        .test_support()
        .size(px(16.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_size(px(10.))
        .child(number.to_owned());
    if on {
        badge
            .bg(rgb(look.accent))
            .text_color(rgb(0xffffff))
            .font_weight(FontWeight::SEMIBOLD)
    } else {
        badge
            .border_1()
            .border_color(rgb(look.line))
            .text_color(rgb(look.tertiary))
    }
}

/// Where a layer's files start inside a repository of Whole stack (D867):
/// its small outlined number at the file tick's indent, then its title in
/// secondary grey. `layer` is `(number, title)`; `None` is "Other changes",
/// which has no circle and no click. A click picks the layer.
pub fn layer_caption(look: Look, repo: &str, layer: Option<(usize, &str)>, send: &Sender) -> AnyElement {
    let key = layer.map_or_else(|| "other".to_owned(), |(number, _)| number.to_string());
    let line = row()
        .id(SharedString::from(format!("stack-layer-caption {repo} {key}")))
        .test_support()
        .gap(px(6.))
        .h(px(18.))
        .pl(px(22.))
        .pr(px(8.))
        .whitespace_nowrap()
        .min_w_0();
    match layer {
        Some((number, title)) => {
            let target = Some(title.to_owned());
            let click = send.clone();
            line.cursor_pointer()
                .child(
                    div()
                        .size(px(13.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(look.line))
                        .text_size(px(9.))
                        .text_color(rgb(look.tertiary))
                        .mr(px(2.))
                        .child(number.to_string()),
                )
                .child(secondary(look, title.to_owned()).min_w_0().overflow_hidden().text_ellipsis())
                .on_click(move |_, _, cx| click(AppAction::Review(ReviewAction::PickLayer(target.clone())), cx))
                .into_any_element()
        }
        None => line
            .child(div().w(px(15.)).flex_shrink_0())
            .child(secondary(look, "Other changes"))
            .into_any_element(),
    }
}

/// "web moved since it was sent", with its one action (DS-65).
/// What a row says about the looks (D852): "✓ Looks good" or "● Changed since
/// you looked", after the repos, with its colour. Whole stack reads Looks good
/// once every layer does.
fn layer_look_note(
    review: &ReviewState,
    rows: &StackRows,
    entry: &StackRow,
) -> Option<(&'static str, u32)> {
    use humata2_review::looks::{LayerLook, layer_look};
    let session = review.session.as_ref()?;
    let of = |title: &str| layer_look(review, session, title);
    let state = if entry.number.is_some() {
        of(&entry.title)
    } else {
        let layers: Vec<LayerLook> = rows
            .rows
            .iter()
            .filter(|row| row.number.is_some())
            .map(|row| of(&row.title))
            .collect();
        if !layers.is_empty() && layers.iter().all(|each| *each == LayerLook::LooksGood) {
            LayerLook::LooksGood
        } else {
            LayerLook::None
        }
    };
    match state {
        LayerLook::None => None,
        LayerLook::LooksGood => Some(("✓ Looks good", 0x28a745)),
        LayerLook::ChangedSinceLooked => Some(("● Changed since you looked", 0xd98200)),
    }
}

/// "3 drafts · 1 note": the drafts and the agent's notes on the files of a
/// row, in accent (UXSIMPLE 3-4). `None` when both are none, or when the
/// row's files were not read yet.
fn layer_counts(
    review: &ReviewState,
    comments: &humata2_review::comments::CommentsState,
    entry: &StackRow,
) -> Option<String> {
    let session = review.session.as_ref()?;
    let title = entry.number.map(|_| entry.title.as_str());
    let files = humata2_review::looks::layer_files(review, session, title)?;
    let held = comments.sessions.get(session)?;
    let (mut drafts, mut notes) = (0, 0);
    for comment in &held.comments {
        let here = files
            .iter()
            .any(|(repo, path)| *repo == comment.anchor.repo && *path == comment.anchor.path);
        if !here {
            continue;
        }
        if comment.counts_as_draft() {
            drafts += 1;
        } else if humata2_review::comments::is_agent_note(comment) {
            notes += 1;
        }
    }
    let noun = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
    match (drafts, notes) {
        (0, 0) => None,
        (d, 0) => Some(noun(d, "draft")),
        (0, n) => Some(noun(n, "note")),
        (d, n) => Some(format!("{} \u{b7} {}", noun(d, "draft"), noun(n, "note"))),
    }
}

fn look_note_el(index: usize, words: &'static str, color: u32, after_counts: bool) -> AnyElement {
    text(words)
        .when(!after_counts, |status| status.ml_auto())
        .id(SharedString::from(format!("stack-row-look-{index}")))
        .test_support()
        .flex_shrink_0()
        .text_size(px(11.))
        .text_color(rgb(color))
        .into_any_element()
}

fn drifted_line(look: Look, index: usize, layer: &str, repo: &str, send: &Sender) -> impl IntoElement {
    let short = short_repo(repo).to_owned();
    let send = send.clone();
    let ask = ReviewAction::AskAgain {
        layer: layer.to_owned(),
        repo: repo.to_owned(),
    };
    // A filled box under the list (frame 121); the button wraps under the
    // line when the two don't fit side by side.
    row()
        .id(SharedString::from(format!("stack-drift-{index}-{repo}")))
        .test_support()
        .flex_wrap()
        .justify_between()
        .gap(px(6.))
        .px(px(8.))
        .py(px(5.))
        .my(px(2.))
        .rounded(px(6.))
        .bg(rgb(look.card))
        .child(
            secondary(look, format!("{short} moved since it was sent"))
                .text_size(px(12.))
                .whitespace_nowrap(),
        )
        .child(
            // The footer's grey button with a hairline (frame 121).
            widgets::push_quiet_shape(
                SharedString::from(format!("stack-send-again-{index}-{repo}")),
                "Ask the agent to send it again",
                look,
            )
            .border_1()
            .on_click(move |_, _, cx| send(AppAction::Review(ask.clone()), cx)),
        )
}

/// How a stack names `repo` beside the others in `all`: `web` for
/// `acme/web`, or the whole name when another repository ends the same way.
pub fn stack_repo_name<'a, 'b>(repo: &'a str, all: impl IntoIterator<Item = &'b str>) -> &'a str {
    let short = short_repo(repo);
    let clash = all.into_iter().any(|other| other != repo && short_repo(other) == short);
    if clash { repo.strip_prefix('/').unwrap_or(repo) } else { short }
}

/// `web` for `acme/web`.
fn short_repo(repo: &str) -> &str {
    let repo = repo.strip_prefix('/').unwrap_or(repo);
    repo.rsplit('/').next().unwrap_or(repo)
}

/// The one quiet line a repository the picked layer doesn't touch gets
/// (DS-63): "web isn't in this layer".
pub fn untouched_line(look: Look, repo: &str) -> impl IntoElement {
    secondary(look, format!("{} isn't in this layer", short_repo(repo)))
        .id(SharedString::from(format!("stack-untouched {repo}")))
        .test_support()
        .text_size(px(12.))
        .text_color(rgb(look.tertiary))
        // Under the file names: 22 pt in, the 14 pt tick and its 6 pt gap.
        .pl(px(42.))
        .pr(px(8.))
        .py(px(2.))
}

/// The quiet lines that end the file list while a layer is picked: one for
/// each repository of the session the layer doesn't touch (DS-63).
pub fn untouched_lines(review: &ReviewState, look: Look) -> Vec<AnyElement> {
    review.stack_rows().map_or_else(Vec::new, |rows| {
        rows.untouched_repos
            .iter()
            .map(|repo| untouched_line(look, repo).into_any_element())
            .collect()
    })
}

/// Everything a stack draws above the Diff panel's files: the layer list,
/// a line for each repository the picked layer doesn't touch, and why the
/// last ask to send the stack again failed. `None` when the open session
/// shows no stack.
pub fn stack_top(
    review: &ReviewState,
    comments: &humata2_review::comments::CommentsState,
    look: Look,
    send: &Sender,
    width: f32,
) -> Option<AnyElement> {
    let rows = review.stack_rows()?;
    let failed = review
        .session
        .as_ref()
        .and_then(|session| review.ask_again_error.get(session))
        .cloned();
    Some(
        col()
            .id("stack-top")
            .test_support()
            .flex_shrink_0()
            .px(px(8.))
            .pt(px(8.))
            .gap(px(2.))
            .child(
                text(format!(
                    "STACK \u{b7} {} {}",
                    rows.rows.len().saturating_sub(1),
                    if rows.rows.len() == 2 { "LAYER" } else { "LAYERS" }
                ))
                .id("stack-caption")
                .test_support()
                .px(px(8.))
                .pb(px(4.))
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary)),
            )
            .child(stack_list(look, review, comments, &rows, &list_scroll(), send, width))
            // DS-65: the picked layer's moved repositories, under the list
            // (frame 121; Whole stack and the other layers draw none).
            .children(rows.rows.iter().enumerate().filter(|(index, entry)| *index == rows.picked && entry.number.is_some()).flat_map(
                |(index, entry)| {
                    entry
                        .drifted
                        .iter()
                        .map(move |repo| drifted_line(look, index, &entry.title, repo, send))
                },
            ))
            .when_some(failed, |top, error| {
                top.child(
                    secondary(look, format!("Couldn't ask the agent: {error}"))
                        .id("stack-ask-failed")
                        .test_support()
                        .px(px(8.)),
                )
            })
            .into_any_element(),
    )
}

/// A quiet accent link of the stack's footer.
fn footer_link(look: Look, id: &'static str, words: String, action: ReviewAction, send: &Sender) -> AnyElement {
    let send = send.clone();
    text(words.clone())
        .id(id)
        .role(Role::Button)
        .aria_label(words)
        .test_support()
        .flex_shrink_0()
        .whitespace_nowrap()
        .text_size(px(12.))
        .text_color(rgb(look.accent))
        .cursor_pointer()
        .on_click(move |_, _, cx| send(AppAction::Review(action.clone()), cx))
        .into_any_element()
}

/// The left of a stack's footer (frames 119 to 121): with Whole stack
/// picked, "3 pull requests open", which opens the push menu (nothing when
/// the stack has none); with a layer picked, Previous and Next layer. `wide`
/// (full screen) spells out "Previous layer". `None` when no stack shows.
pub fn footer_links(review: &ReviewState, look: Look, send: &Sender, wide: bool) -> Option<Vec<AnyElement>> {
    let rows = review.stack_rows()?;
    let picked = rows.rows.get(rows.picked)?;
    if picked.number.is_none() {
        // The open pull requests, or Push & PR when none is open yet; both
        // open the push menu.
        let open = picked.pull_requests.len();
        let (id, words) = if open > 0 {
            let noun = if open == 1 { "request" } else { "requests" };
            ("stack-pull-requests", format!("{open} pull {noun} open"))
        } else {
            ("stack-push", "Push & PR\u{2026}".to_owned())
        };
        return Some(vec![footer_link(look, id, words, ReviewAction::TogglePushMenu, send)]);
    }
    Some(vec![
        footer_link(
            look,
            "stack-prev-layer",
            if wide { "\u{2039} Previous layer" } else { "\u{2039} Previous" }.to_owned(),
            ReviewAction::PrevLayer,
            send,
        ),
        footer_link(look, "stack-next-layer", "Next layer \u{203a}".to_owned(), ReviewAction::NextLayer, send),
    ])
}

/// Whether Whole stack is the stack's picked row.
pub fn whole_picked(review: &ReviewState) -> bool {
    review.stack_rows().is_some_and(|rows| rows.picked == 0)
}

/// The header's comparing line while a stack shows (DS-62); `None` keeps
/// the panel's own base line.
pub fn header_line(review: &ReviewState) -> Option<String> {
    review.stack_rows().map(|rows| stack_header_line(&rows.header))
}

/// Whether the header's base picker hides: a stack picks its own bases
/// (DS-62).
pub fn hides_base_picker(review: &ReviewState) -> bool {
    review.stack_rows().is_some()
}

/// `panel` with the layer keys (⌥⌘] and ⌥⌘[) live while a stack shows.
pub fn stack_keys<E: InteractiveElement>(panel: E, review: &ReviewState) -> E {
    if review.stack_rows().is_some() {
        panel.key_context(crate::keys::DIFF_STACK)
    } else {
        panel
    }
}

/// How much `file` of `diff` matters (D802), as the review reads it.
pub fn tier_of(review: &ReviewState, diff: &RepoDiff, file: &FileDiff) -> Tier {
    review.tier_of(&diff.session, &diff.repo, file)
}

/// Whether `repo`'s skim files are open in the open session.
pub fn skim_open(review: &ReviewState, repo: &str) -> bool {
    review
        .session
        .as_ref()
        .is_some_and(|session| review.skim_open.contains(&(session.clone(), repo.to_owned())))
}

/// The one row `repo`'s skim files fold into at its end (DS-67): "3 files
/// to skim · Cargo.lock, …". A click opens them in place under it, or folds
/// them again. Nothing for no skim files.
pub fn skim_row(look: Look, repo: &str, paths: &[&str], open: bool, send: &Sender) -> Option<AnyElement> {
    if paths.is_empty() {
        return None;
    }
    let send = send.clone();
    let toggle = ReviewAction::ToggleSkim { repo: repo.to_owned() };
    let label = skim_label(paths);
    Some(
        row()
            .id(SharedString::from(format!("stack-skim {repo}")))
            .role(Role::Button)
            .aria_label(label.clone())
            .aria_expanded(open)
            .test_support()
            .gap(px(6.))
            .h(px(25.))
            .pl(px(22.))
            .pr(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            .on_click(move |_, _, cx| send(AppAction::Review(toggle.clone()), cx))
            .child(div().flex_shrink_0().child(icon(
                if open { IconName::ChevronDown } else { IconName::ChevronRight },
                look.secondary,
                12.,
            )))
            .child(
                secondary(look, label)
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis(),
            )
            .into_any_element(),
    )
}

/// The "key" chip a key file carries before its counts (DS-67, frames 119
/// to 121): accent text on a pale accent fill.
pub fn key_chip(look: Look, id: impl Into<SharedString>) -> impl IntoElement {
    let mut fill = Hsla::from(rgb(look.accent));
    fill.a = 0.14;
    text("key")
        .id(id.into())
        .aria_label("Key file")
        .test_support()
        .flex_shrink_0()
        .px(px(5.))
        .rounded(px(4.))
        .bg(fill)
        .text_size(px(10.5))
        .text_color(rgb(look.accent))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::sync::Arc;

    use super::*;
    // The glob above brings GPUI's `test` macro; `#[test]` is the standard one.
    use ::core::prelude::v1::test;
    use gpui_kit::test::TestWindowExt as _;
    use humata2_clients_fakes::fixture;
    use humata2_clients::model::{RepoBase, RepoDiffSpec, SessionId};
    use humata2_clients::views::{LayerDrift, LayerRepoView, LayerView, PlanView, StackView};

    fn session() -> SessionId {
        SessionId::new("ledger")
    }

    fn part(repo: &str, drift: Option<LayerDrift>) -> LayerRepoView {
        LayerRepoView {
            repo: repo.to_owned(),
            spec: RepoDiffSpec {
                repo: repo.to_owned(),
                base: RepoBase::Range { base: "a0".to_owned(), head: "a1".to_owned() },
                options: Default::default(),
            },
            pull_request: String::new(),
            files: 2,
            drift,
        }
    }

    /// Ledger (in acme/api) with two layers: "schema" in api, then
    /// "screen" in web, which moved since it was sent.
    fn stacked(layer: Option<&str>) -> ReviewState {
        let mut workspace = fixture::design();
        for each in &mut workspace.sessions {
            if each.id == session() {
                each.repo = "acme/api".to_owned();
            }
        }
        workspace.diffs = Vec::new();
        let mut review = ReviewState {
            workspace: Arc::new(workspace),
            session: Some(session()),
            ..ReviewState::default()
        };
        let layer_of = |title: &str, repos| LayerView {
            title: title.to_owned(),
            purpose: format!("Why {title}"),
            repos,
            ..LayerView::default()
        };
        review.plans.insert(
            session(),
            PlanView {
                stack: Some(StackView {
                    layers: vec![
                        layer_of("schema", vec![part("acme/api", None)]),
                        layer_of("screen", vec![part("acme/web", Some(LayerDrift::Moved))]),
                    ],
                    whole: Vec::new(),
                }),
                ..PlanView::default()
            },
        );
        review.layer.insert(session(), layer.map(str::to_owned));
        review
    }

    fn plain() -> ReviewState {
        ReviewState {
            session: Some(session()),
            ..ReviewState::default()
        }
    }

    struct View {
        review: ReviewState,
        send: Sender,
        focus: FocusHandle,
        skims: Vec<&'static str>,
        keys: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Render for View {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let look = Look::of(cx);
            let next = self.keys.clone();
            let previous = self.keys.clone();
            stack_keys(
                div()
                    .id("panel")
                    .size_full()
                    .track_focus(&self.focus)
                    .on_action(move |_: &crate::keys::NextLayer, _, _| next.borrow_mut().push("next"))
                    .on_action(move |_: &crate::keys::PrevLayer, _, _| previous.borrow_mut().push("previous")),
                &self.review,
            )
            .children(stack_top(&self.review, &humata2_review::comments::CommentsState::default(), look, &self.send, 340.))
            .children(untouched_lines(&self.review, look))
            .children(footer_links(&self.review, look, &self.send, false).unwrap_or_default())
            .children(skim_row(look, "acme/api", &self.skims, skim_open(&self.review, "acme/api"), &self.send))
            .child(key_chip(look, "change-key acme/api/src/lib.rs"))
        }
    }

    struct Drawn {
        sent: Vec<AppAction>,
        keys: Vec<&'static str>,
        ids: Vec<&'static str>,
    }

    /// Draws `review` with `skims` as acme/api's skim files, runs `step`,
    /// and answers with what was sent, the layer keys caught, and which of
    /// the ids in `look_for` were drawn before `step`.
    fn run(
        cx: &mut TestAppContext,
        review: ReviewState,
        skims: Vec<&'static str>,
        look_for: &[&'static str],
        step: impl FnOnce(&mut Window, &mut App),
    ) -> Drawn {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let keys = Rc::new(RefCell::new(Vec::new()));
        let (log, caught) = (sent.clone(), keys.clone());
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::init(cx);
                crate::keys::bind(cx);
                gpui_kit::open_window(WindowOptions::default(), cx, move |window, cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    cx.new(|_| View {
                        review,
                        send: Rc::new(move |action, _| log.borrow_mut().push(action)),
                        focus,
                        skims,
                        keys: caught,
                    })
                })
            })
            .expect("the test window opens");
        let ids = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                let ids = look_for
                    .iter()
                    .copied()
                    .filter(|id| window.try_find(*id).is_some())
                    .collect();
                step(window, cx);
                ids
            })
            .expect("the test window is open");
        Drawn {
            sent: sent.take(),
            keys: keys.take(),
            ids,
        }
    }

    fn review_sent(drawn: &Drawn) -> Vec<ReviewAction> {
        drawn
            .sent
            .iter()
            .filter_map(|action| match action {
                AppAction::Review(review) => Some(review.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_header_names_the_whole_stack_or_the_layer_showing() {
        assert_eq!(
            stack_header_line(&StackHeader::Whole { layers: 3 }),
            "Whole stack \u{b7} 3 layers"
        );
        assert_eq!(
            stack_header_line(&StackHeader::Layer {
                number: 2,
                of: 3,
                title: "journal".to_owned()
            }),
            "Layer 2 of 3 \u{b7} journal"
        );
    }

    #[test]
    fn a_layer_names_its_repositories_and_whole_stack_counts_them() {
        let layer = StackRow {
            number: Some(2),
            repos: vec!["acme/api".to_owned(), "acme/ledger".to_owned()],
            files: 4,
            ..StackRow::default()
        };
        assert_eq!(row_note(&layer), "api \u{b7} ledger");
        let whole = StackRow {
            repos: vec!["acme/api".to_owned(); 5],
            files: 23,
            ..StackRow::default()
        };
        assert_eq!(row_note(&whole), "5 repos \u{b7} 23 files");
        let one = StackRow { repos: vec!["acme/api".to_owned()], files: 1, ..StackRow::default() };
        assert_eq!(row_note(&one), "1 repo \u{b7} 1 file");
    }

    #[test]
    fn a_stack_takes_the_header_and_hides_the_base_picker() {
        let review = stacked(Some("screen"));
        assert!(hides_base_picker(&review));
        let line = header_line(&review).unwrap_or_default();
        assert!(line.contains("screen"), "{line}");
        assert!(!hides_base_picker(&plain()));
        assert_eq!(header_line(&plain()), None);
    }

    #[gpui_kit::test]
    fn nothing_to_skim_draws_no_skim_row(cx: &mut TestAppContext) {
        let drawn = run(cx, plain(), Vec::new(), &["stack-skim acme/api"], |_, _| {});
        assert!(drawn.ids.is_empty());
    }

    #[gpui_kit::test]
    fn clicking_a_layer_picks_it_and_whole_stack_picks_none(cx: &mut TestAppContext) {
        let drawn = run(cx, stacked(None), Vec::new(), &[], |window, cx| {
            window.click("stack-row-1", cx);
            window.click("stack-row-0", cx);
        });
        assert_eq!(
            review_sent(&drawn),
            vec![
                ReviewAction::PickLayer(Some("screen".to_owned())),
                ReviewAction::PickLayer(None),
            ]
        );
    }

    #[gpui_kit::test]
    fn a_moved_repository_asks_the_agent_to_send_its_layer_again(cx: &mut TestAppContext) {
        let drawn = run(cx, stacked(Some("screen")), Vec::new(), &[], |window, cx| {
            window.click("stack-send-again-1-acme/web", cx);
        });
        assert_eq!(
            review_sent(&drawn),
            vec![ReviewAction::AskAgain {
                layer: "screen".to_owned(),
                repo: "acme/web".to_owned(),
            }]
        );
    }

    #[test]
    fn repositories_ending_alike_keep_their_whole_names() {
        assert_eq!(stack_repo_name("acme/web", ["acme/api", "acme/web"]), "web");
        assert_eq!(stack_repo_name("acme/ledger", ["/ledger", "acme/ledger"]), "acme/ledger");
        assert_eq!(stack_repo_name("/ledger", ["/ledger", "acme/ledger"]), "ledger");
    }

    #[gpui_kit::test]
    fn only_the_picked_layer_shows_its_moved_repository(cx: &mut TestAppContext) {
        let id = "stack-drift-1-acme/web";
        assert!(run(cx, stacked(None), Vec::new(), &[id], |_, _| {}).ids.is_empty());
        assert!(run(cx, stacked(Some("schema")), Vec::new(), &[id], |_, _| {}).ids.is_empty());
        assert_eq!(run(cx, stacked(Some("screen")), Vec::new(), &[id], |_, _| {}).ids, vec![id]);
    }

    #[gpui_kit::test]
    fn a_layers_footer_steps_to_the_layers_beside_it(cx: &mut TestAppContext) {
        let drawn = run(cx, stacked(Some("schema")), Vec::new(), &[], |window, cx| {
            window.click("stack-next-layer", cx);
            window.click("stack-prev-layer", cx);
        });
        assert_eq!(review_sent(&drawn), vec![ReviewAction::NextLayer, ReviewAction::PrevLayer]);
    }

    #[gpui_kit::test]
    fn whole_stack_opens_the_push_menu_from_its_pull_requests(cx: &mut TestAppContext) {
        let ids = ["stack-pull-requests", "stack-next-layer"];
        let none = run(cx, stacked(None), Vec::new(), &["stack-push"], |window, cx| {
            window.click("stack-push", cx);
        });
        assert_eq!(review_sent(&none), vec![ReviewAction::TogglePushMenu], "no pull request yet: Push & PR");
        let mut review = stacked(None);
        if let Some(stack) = review.plans.get_mut(&session()).and_then(|plan| plan.stack.as_mut()) {
            stack.layers[0].repos[0].pull_request = "acme/api#412".to_owned();
        }
        let drawn = run(cx, review, Vec::new(), &ids, |window, cx| {
            window.click("stack-pull-requests", cx);
        });
        assert_eq!(drawn.ids, vec!["stack-pull-requests"]);
        assert_eq!(review_sent(&drawn), vec![ReviewAction::TogglePushMenu]);
    }

    #[gpui_kit::test]
    fn a_stack_names_its_layer_count_above_the_list(cx: &mut TestAppContext) {
        let drawn = run(cx, stacked(None), Vec::new(), &["stack-caption", "stack-row-badge-0"], |_, _| {});
        assert_eq!(drawn.ids, vec!["stack-caption", "stack-row-badge-0"]);
    }

    #[gpui_kit::test]
    fn a_layer_says_which_repositories_it_leaves_out(cx: &mut TestAppContext) {
        let ids = ["stack-top", "stack-untouched acme/api", "stack-untouched acme/web"];
        let drawn = run(cx, stacked(Some("screen")), Vec::new(), &ids, |_, _| {});
        assert_eq!(drawn.ids, vec!["stack-top", "stack-untouched acme/api"]);
    }

    #[gpui_kit::test]
    fn a_failed_ask_shows_under_the_list(cx: &mut TestAppContext) {
        let mut review = stacked(None);
        review.ask_again_error.insert(session(), "the session has exited".to_owned());
        let drawn = run(cx, review, Vec::new(), &["stack-ask-failed"], |_, _| {});
        assert_eq!(drawn.ids, vec!["stack-ask-failed"]);
    }

    #[gpui_kit::test]
    fn without_a_stack_nothing_of_it_draws(cx: &mut TestAppContext) {
        let drawn = run(cx, plain(), Vec::new(), &["stack-top", "stack-list"], |_, _| {});
        assert!(drawn.ids.is_empty());
    }

    #[gpui_kit::test]
    fn the_skim_row_opens_and_folds_its_repository(cx: &mut TestAppContext) {
        let ids = ["stack-skim acme/api", "change-key acme/api/src/lib.rs"];
        let drawn = run(cx, plain(), vec!["Cargo.lock", "gen/api.rs"], &ids, |window, cx| {
            window.click("stack-skim acme/api", cx);
        });
        assert_eq!(drawn.ids, ids.to_vec());
        assert_eq!(
            review_sent(&drawn),
            vec![ReviewAction::ToggleSkim { repo: "acme/api".to_owned() }]
        );
    }

    #[gpui_kit::test]
    fn the_layer_keys_work_only_while_a_stack_shows(cx: &mut TestAppContext) {
        let keys = |window: &mut Window, cx: &mut App| {
            window.press("alt-cmd-]", cx);
            window.press("alt-cmd-[", cx);
        };
        assert_eq!(run(cx, stacked(None), Vec::new(), &[], keys).keys, vec!["next", "previous"]);
        assert!(run(cx, plain(), Vec::new(), &[], keys).keys.is_empty());
    }
}
