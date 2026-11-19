//! What sits over a screen: the ⌘K palette, the New session dialog and a
//! toast.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;

/// The dimmed layer under a dialog.
pub fn scrim(look: Look) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(if look.dark { rgba(0x0000_0080) } else { rgba(0x0000_0030) })
}

/// `under` with the palette open over it.
pub fn palette(look: Look, under: Div) -> Div {
    let result = |glyph: IconName, text: &'static str, note: &'static str, used: Option<&'static str>, keys: Option<&'static str>, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(9.))
            .h(px(32.))
            .mx(px(6.))
            .px(px(10.))
            .rounded(px(6.))
            .when(on, |row| row.bg(c(look.selection)).text_color(c(look.selection_text)))
            .child(icon(glyph, if on { look.selection_text } else { look.secondary }))
            .child(div().child(text))
            .child(div().flex_1().text_size(px(11.5)).text_color(c(if on { look.selection_text } else { look.secondary })).child(note))
            .children(used.map(|used| div().text_size(px(11.)).text_color(c(if on { look.selection_text } else { look.tertiary })).child(used)))
            .children(keys.map(|keys| kbd(look, keys)))
    };
    let group = |title: &'static str| div().px(px(16.)).pt(px(8.)).pb(px(3.)).child(label(look, title));
    under.relative().child(scrim(look)).child(
        div().absolute().top(px(110.)).left_0().w_full().flex().justify_center().child(
            div()
                .w(px(600.))
                .flex()
                .flex_col()
                .pb(px(6.))
                .rounded(px(12.))
                .bg(c(look.card))
                .border_1()
                .border_color(c(look.line))
                .shadow_lg()
                .text_size(px(13.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .h(px(48.))
                        .px(px(16.))
                        .border_b_1()
                        .border_color(c(look.line))
                        .child(icon(IconName::Search, look.secondary))
                        .child(div().text_size(px(16.)).child("ledg"))
                        .child(div().w(px(1.5)).h(px(18.)).bg(c(look.accent)))
                        .child(div().flex_1()),
                )
                .child(group("Sessions"))
                .child(result(IconName::SquareTerminal, "Ledger schema migration", "# payments-rewrite · working", Some("used Today 12:52"), None, true))
                .child(result(IconName::SquareTerminal, "zsh in Ledger schema migration", "devbox-1", None, None, false))
                .child(result(IconName::GitFork, "Try BRIN index", "fork of Ledger schema migration", Some("used Today 13:02"), None, false))
                .child(group("Files"))
                .child(result(IconName::FileCode, "ledger/migrations/0042_accounts.sql", "Ledger schema migration", None, None, false))
                .child(group("Commands"))
                .child(result(IconName::GitFork, "Fork “Ledger schema migration”", "", None, Some("⇧⌘F"), false))
                .child(result(IconName::Columns2, "Toggle side panel", "", None, Some("⌥⌘0"), false))
                .child(result(IconName::Maximize2, "Terminal full screen", "", None, Some("⇧⌘↩"), false)),
        ),
    )
}

/// `under` with the New session dialog open over it (SE-2).
pub fn new_session(look: Look, under: Div) -> Div {
    let row = |key: &'static str, value: Div| {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().w(px(110.)).flex().justify_end().text_color(c(look.secondary)).child(key))
            .child(value.flex_1())
    };
    let repo = |name: &'static str, choice: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(small_icon(IconName::FolderGit2, look.secondary))
            .child(div().w(px(120.)).child(name))
            .child(popup(look, choice))
    };
    under.relative().child(scrim(look)).child(
        div().absolute().top(px(70.)).left_0().w_full().flex().justify_center().child(
            div()
                .w(px(560.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(20.))
                .rounded(px(12.))
                .bg(c(look.card))
                .border_1()
                .border_color(c(look.line))
                .shadow_lg()
                .text_size(px(12.5))
                .child(div().flex().items_center().gap(px(8.)).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("New session")).child(secondary(look, "in # payments-rewrite · for PAY-9")))
                .child(row("Title", field(look, "Backfill progress bar", false)))
                .child(row(
                    "Runs",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(div().flex().child(segmented(look, &["Claude Code", "Codex", "Copilot", "OpenCode", "Terminal"], 0)).child(div().flex_1()))
                        .child(div().flex().gap(px(8.)).child(secondary(look, "An agent's own TUI, with its own defaults, or a plain shell")).child(div().flex_1())),
                ))
                .child(row("Machine", div().flex().gap(px(8.)).child(popup(look, "devbox-1")).child(secondary(look, "11 GB free")).child(div().flex_1())))
                .child(row(
                    "Repos",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(repo("ledger", "Own clone, new branch from main"))
                        .child(repo("api", "Leave this repo out")),
                ))
                .child(row(
                    "First message",
                    div()
                        .h(px(70.))
                        .p(px(8.))
                        .rounded(px(6.))
                        .bg(c(look.card))
                        .border_1()
                        .border_color(c(look.line))
                        .text_color(c(look.tertiary))
                        .child("Optional. Typed into the agent once it starts, e.g. “Work on PAY-9”."),
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .pt(px(4.))
                        .child(secondary(look, "Branch: backfill-progress-bar"))
                        .child(div().flex_1())
                        .child(button(look, Kind::Secondary, "Cancel"))
                        .child(button(look, Kind::Primary, "Start session")),
                ),
        ),
    )
}

/// The app's shared progress line: title and label over a 2 px track
/// (`widgets::progress_line`). `fill` None draws the still sweep.
fn progress_line(look: Look, title: &'static str, label: Option<&'static str>, fill: Option<f32>) -> Div {
    let bar = match fill {
        Some(share) => div().h_full().rounded(px(1.)).bg(c(look.accent)).w(relative(share)),
        None => div().absolute().top(px(0.)).h_full().w(relative(0.25)).left(relative(0.37)).rounded(px(1.)).bg(c(look.accent)),
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .items_center()
                .w_full()
                .gap(px(6.))
                .text_size(px(11.))
                .whitespace_nowrap()
                .child(div().flex_1().min_w_0().truncate().text_color(c(look.secondary)).child(title))
                .children(label.map(|text| div().flex_shrink_0().text_color(c(look.secondary)).child(text))),
        )
        .child(div().w_full().h(px(2.)).rounded(px(1.)).bg(c(look.line)).relative().overflow_hidden().child(bar))
}

/// Frame 122: the New session sheet in its Starting… state, with the
/// clone's progress line under the buttons (PROGRESS).
pub fn new_session_cloning(look: Look, under: Div) -> Div {
    let row = |key: &'static str, value: Div| {
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().w(px(110.)).flex().justify_end().text_color(c(look.secondary)).child(key))
            .child(value.flex_1())
    };
    let repo = |name: &'static str, choice: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(small_icon(IconName::FolderGit2, look.secondary))
            .child(div().w(px(120.)).child(name))
            .child(popup(look, choice))
    };
    under.relative().child(scrim(look)).child(
        div().absolute().top(px(70.)).left_0().w_full().flex().justify_center().child(
            div()
                .w(px(560.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(20.))
                .rounded(px(12.))
                .bg(c(look.card))
                .border_1()
                .border_color(c(look.line))
                .shadow_lg()
                .text_size(px(12.5))
                .child(div().flex().items_center().gap(px(8.)).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("New session")).child(secondary(look, "in # payments-rewrite · for PAY-9")))
                .child(row("Title", field(look, "Backfill progress bar", false)))
                .child(row(
                    "Runs",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(div().flex().child(segmented(look, &["Claude Code", "Codex", "Copilot", "OpenCode", "Terminal"], 0)).child(div().flex_1()))
                        .child(div().flex().gap(px(8.)).child(secondary(look, "An agent's own TUI, with its own defaults, or a plain shell")).child(div().flex_1())),
                ))
                .child(row("Machine", div().flex().gap(px(8.)).child(popup(look, "devbox-1")).child(secondary(look, "11 GB free")).child(div().flex_1())))
                .child(row(
                    "Repos",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(repo("ledger", "Own clone, new branch from main"))
                        .child(repo("api", "Leave this repo out")),
                ))
                .child(row(
                    "First message",
                    div()
                        .h(px(70.))
                        .p(px(8.))
                        .rounded(px(6.))
                        .bg(c(look.card))
                        .border_1()
                        .border_color(c(look.line))
                        .text_color(c(look.tertiary))
                        .child("Optional. Typed into the agent once it starts, e.g. “Work on PAY-9”."),
                ))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .pt(px(4.))
                        .child(secondary(look, "Branch: backfill-progress-bar"))
                        .child(div().flex_1())
                        .child(button(look, Kind::Secondary, "Cancel"))
                        .child(button(look, Kind::Primary, "Starting…").opacity(0.5)),
                )
                .child(progress_line(look, "Cloning ledger · Receiving objects", Some("42%"), Some(0.42))),
        ),
    )
}

/// `under` with a toast at the bottom (NV-10).
pub fn toast(look: Look, under: Div, text: &'static str, action: &'static str) -> Div {
    under.relative().child(
        div().absolute().bottom(px(24.)).left_0().w_full().flex().justify_center().child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .px(px(14.))
                .py(px(9.))
                .rounded(px(10.))
                .bg(c(if look.dark { 0x3a3a3d } else { 0x2c2c2e }))
                .text_color(c(0xffffff))
                .text_size(px(12.5))
                .shadow_lg()
                .child(small_icon(IconName::Archive, 0xffffff))
                .child(text)
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(c(0x64a8ff)).child(action)),
        ),
    )
}
