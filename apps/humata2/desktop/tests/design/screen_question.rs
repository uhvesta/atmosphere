//! D781: a question read off its agent's screen, in the wide Inbox. The
//! card offers the choices; under it the live terminal draws the tab at its
//! own size (100x24 here, Copilot's folder-trust box at the bottom), then a
//! hint row with Dismiss. Frame 114 is unfocused, 115 focused with the
//! cursor drawn, 116 has no terminal because its machine is offline.

use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::{inbox_row, page, toolbar};
use crate::sidebar::Side;

/// The font the 100 columns fit the detail at, and its row.
const FONT: f32 = 10.5;
const LINE: f32 = 17.;

/// What the terminal box shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Unfocused,
    Focused,
    Offline,
}

/// The 24 rows of the agent's screen; the box opens scrolled to the bottom,
/// so the first row is above it.
fn rows() -> [&'static str; 24] {
    [
        "",
        " Welcome to GitHub Copilot CLI",
        " Version 1.0",
        "",
        " Copilot can write, test and debug code right from your terminal.",
        " Describe a task to get started or enter ? for help.",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        " ╭──────────────────────────────────────────────────────────────────────────────────────────────╮",
        " │ Do you trust the files in this folder?                                                       │",
        " │                                                                                              │",
        " │ ~/work/refunds                                                                               │",
        " │ Copilot may read, change and run files in this folder.                                       │",
        " │                                                                                              │",
        " │ ❯ 1. Yes                                                                                     │",
        " │   2. Yes, and remember this folder for future sessions                                       │",
        " │   3. No (Esc)                                                                                │",
        " ╰──────────────────────────────────────────────────────────────────────────────────────────────╯",
    ]
}

fn terminal(look: Look, focused: bool) -> Div {
    let row = |index: usize, text: &'static str| {
        let highlight = index == 20;
        div()
            .h(px(LINE))
            .flex()
            .whitespace_nowrap()
            .text_color(c(if highlight { look.accent } else if index >= 15 { look.term_text } else { look.term_dim }))
            .child(text)
            .when(focused && index == 20, |row| {
                row.child(div().w(px(FONT * 0.6)).h(px(LINE - 3.)).bg(c(look.term_text)))
            })
    };
    // Scrolled to the bottom: the rows that fit, the last ones.
    let fit = ((420. - 2. * 8.) / LINE) as usize;
    let all = rows();
    let first = all.len().saturating_sub(fit);
    div()
        .w_full()
        .h(px(420.))
        .rounded(px(8.))
        .bg(c(look.term))
        .border(px(if focused { 2. } else { 1. }))
        .border_color(c(if focused { look.accent } else { look.line }))
        .p(px(if focused { 7. } else { 8. }))
        .overflow_hidden()
        .flex()
        .flex_col()
        .justify_end()
        .font_family(MONO)
        .text_size(px(FONT))
        .line_height(px(LINE))
        .children(all.iter().enumerate().skip(first).map(|(index, text)| row(index, text)))
}

fn card_body(look: Look) -> Div {
    card(look)
        .max_w(px(560.))
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(status(St::NeedsYou))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Copilot asks: Do you trust the files in this folder?"))
                .child(div().flex_1())
                .child(secondary(look, "1m")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .child(secondary(look, "Copilot in Refund retries · devbox-1 · # payments-rewrite"))
                .child(div().flex_1())
                .child(div().text_size(px(11.5)).text_color(c(look.accent)).child("Open session ↗")),
        )
        .child(secondary(look, "~/work/refunds"))
        .child(div().child("Copilot is asking on its screen, and waits until it is answered: Do you trust the files in this folder?"))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .flex_wrap()
                .child(button(look, Kind::Primary, "Yes"))
                .child(button(look, Kind::Secondary, "Yes, and remember this folder for future sessions"))
                .child(button(look, Kind::Secondary, "No")),
        )
        .child(secondary(
            look,
            "Yes lets the agent read, change and run the files in that folder. Humata never answers this for you: it picks only what you reply.",
        ))
}

fn hint(look: Look) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.5))
                .text_color(c(look.secondary))
                .child("Answer in the terminal or pick a choice above. Dismiss closes this item and presses nothing."),
        )
        .child(button(look, Kind::Secondary, "Dismiss"))
}

fn offline(look: Look) -> Div {
    div()
        .flex()
        .flex_wrap()
        .gap(px(6.))
        .text_size(px(11.5))
        .text_color(c(look.secondary))
        .child("devbox-1 is offline, so its terminal can't be shown.")
        .child(div().text_color(c(look.accent)).child("Open session ↗"))
        .child("to try there.")
}

fn screen_question(look: Look, screen: Screen) -> Div {
    let heading = |name: &'static str| div().px(px(14.)).pt(px(10.)).pb(px(4.)).child(label(look, name));
    page(
        look,
        Side::at("Inbox"),
        toolbar(look, "Inbox", Some("5 waiting"), vec![segmented(look, &["Waiting", "Answered", "All"], 0)]),
        div()
            .flex()
            .text_size(px(12.5))
            .child(
                div()
                    .w(px(330.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .flex_shrink_0()
                    .border_r_1()
                    .border_color(c(look.line))
                    .child(heading("# payments-rewrite"))
                    .child(inbox_row(look, "Copilot asks: Do you trust the files in this folder?", "Copilot · Refund retries", "1m", true, false))
                    .child(inbox_row(look, "Move PAY-3 to Done", "Claude Code · Ledger schema migration", "2m", false, true))
                    .child(inbox_row(look, "Push try-brin to origin", "Claude Code · Try BRIN index", "9m", false, false))
                    .child(heading("# infra"))
                    .child(inbox_row(look, "Run a command on gpu-box", "Codex · Benchmark BRIN", "22m", false, false))
                    .child(inbox_row(look, "Delete the old snapshots", "Claude Code · Upgrade Postgres 17", "1h", false, false)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(card_body(look))
                    .when(screen != Screen::Offline, |detail| detail.child(terminal(look, screen == Screen::Focused)))
                    .when(screen == Screen::Offline, |detail| detail.child(offline(look)))
                    .child(hint(look)),
            ),
    )
}

/// Frame 114: the open item's terminal, unfocused.
pub fn unfocused(look: Look) -> Div {
    screen_question(look, Screen::Unfocused)
}

/// Frame 115: the terminal focused by a click or Tab, its cursor drawn.
pub fn focused(look: Look) -> Div {
    screen_question(look, Screen::Focused)
}

/// Frame 116: its machine is offline, so no terminal; the reason says where
/// to try instead.
pub fn offline_screen(look: Look) -> Div {
    screen_question(look, Screen::Offline)
}
