//! What sits over a screen: the ⌘K palette, the New session dialog and a
//! toast.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;

fn scrim(look: Look) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(if look.dark { rgba(0x0000_0080) } else { rgba(0x0000_0030) })
}

/// `under` with the palette open over it.
pub fn palette(look: Look, under: Div) -> Div {
    let result = |glyph: IconName, text: &'static str, note: &'static str, keys: Option<&'static str>, on: bool| {
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
                .child(result(IconName::SquareTerminal, "Ledger schema migration", "# payments-rewrite · needs you", None, true))
                .child(result(IconName::GitFork, "Try BRIN index", "fork of Ledger schema migration", None, false))
                .child(group("Files"))
                .child(result(IconName::FileCode, "ledger/migrations/0042_accounts.sql", "Ledger schema migration", None, false))
                .child(group("Commands"))
                .child(result(IconName::GitFork, "Fork “Ledger schema migration”", "", Some("⇧⌘F"), false))
                .child(result(IconName::Columns2, "Toggle side panel", "", Some("⌥⌘0"), false))
                .child(result(IconName::Maximize2, "Terminal full screen", "", Some("⇧⌘↩"), false)),
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
                .child(row("Agent", div().flex().gap(px(8.)).child(popup(look, "Claude Code")).child(popup(look, "Opus")).child(div().flex_1())))
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
