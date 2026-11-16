//! Frame 70: a prepared review opened as a tab in the session (VW-6, VW-8;
//! flow 26.12): the Showing bar under the header, the review tab with its
//! step list on the left and the step's file with the agent's note in the
//! main area.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::session::{header_with, shell, tab};
use crate::sidebar::Side;

/// One numbered step in the left list: its number, repo, path and note.
fn step(look: Look, n: &'static str, repo: &'static str, path: &'static str, note: &'static str, on: bool) -> Div {
    div()
        .flex()
        .gap(px(9.))
        .p(px(8.))
        .rounded(px(7.))
        .when(on, |row| row.bg(c(look.fill)))
        .child(
            div()
                .size(px(18.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .text_size(px(10.5))
                .font_weight(FontWeight::SEMIBOLD)
                .when(on, |dot| dot.bg(c(look.accent)).text_color(c(0xffffff)))
                .when(!on, |dot| dot.border_1().border_color(c(look.tertiary)).text_color(c(look.secondary)))
                .child(n),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .child(div().text_color(c(look.accent)).flex_shrink_0().child(repo))
                        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().font_weight(FontWeight::MEDIUM).child(path)),
                )
                .child(secondary(look, note)),
        )
}

/// The left list: title, who prepared it, the summary and the steps.
fn steps(look: Look) -> Div {
    div()
        .w(px(280.))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(12.))
        .bg(c(look.panel))
        .border_r_1()
        .border_color(c(look.line))
        .text_size(px(12.5))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(14.)).child("PR review: ledger schema"))
                .child(secondary(look, "prepared by Claude Code · 10 min ago")),
        )
        .child(div().text_size(px(12.)).child("The accounts and entries tables, the dual write that fills them, and the reads that move over."))
        .child(hline(look))
        .child(label(look, "Steps · 4"))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(step(look, "1", "ledger", "migrations/0042_accounts.sql", "New tables and the entry index", true))
                .child(step(look, "2", "ledger", "src/client.rs", "The dual write, in one transaction", false))
                .child(step(look, "3", "api", "src/ledger/reads.rs", "Reads switch to entries", false))
                .child(step(look, "4", "ledger", "tests/backfill.rs", "Counts match after the backfill", false)),
        )
}

/// The step's file in the editor: mono lines with numbers, `+` lines tinted.
fn editor(look: Look) -> Div {
    let line = |n: &'static str, added: bool, text: &'static str| {
        div()
            .flex()
            .when(added, |row| row.bg(c(look.add_bg)))
            .child(div().w(px(40.)).pr(px(10.)).flex().justify_end().flex_shrink_0().text_color(c(look.term_dim)).child(n))
            .child(div().whitespace_nowrap().text_color(c(look.term_text)).child(if text.is_empty() { " " } else { text }))
    };
    div()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .flex()
        .flex_col()
        .py(px(6.))
        .bg(c(look.term))
        .font_family(MONO)
        .text_size(px(12.))
        .line_height(px(17.))
        .child(line("1", true, "+ CREATE TABLE accounts ("))
        .child(line("2", true, "+     id          uuid PRIMARY KEY,"))
        .child(line("3", true, "+     owner_id    uuid NOT NULL,"))
        .child(line("4", true, "+     currency    char(3) NOT NULL,"))
        .child(line("5", true, "+     created_at  timestamptz NOT NULL DEFAULT now()"))
        .child(line("6", true, "+ );"))
        .child(line("7", false, ""))
        .child(line("8", true, "+ CREATE TABLE entries ("))
        .child(line("9", true, "+     id          bigserial PRIMARY KEY,"))
        .child(line("10", true, "+     account_id  uuid NOT NULL REFERENCES accounts (id),"))
        .child(line("11", true, "+     amount      numeric(20, 4) NOT NULL,"))
        .child(line("12", true, "+     posted_at   timestamptz NOT NULL"))
        .child(line("13", true, "+ );"))
        .child(line("14", false, ""))
        .child(line("15", true, "+ CREATE INDEX entries_posted_brin ON entries"))
        .child(line("16", true, "+     USING brin (posted_at);"))
}

/// The main area: path, step count, the agent's note, the file, Prev/Next.
fn main_area(look: Look) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(30.))
                .px(px(12.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(c(look.line))
                .text_size(px(12.))
                .child(small_icon(IconName::FileCode, look.secondary))
                .child(div().whitespace_nowrap().font_weight(FontWeight::MEDIUM).child("ledger › migrations › 0042_accounts.sql"))
                .child(div().flex_1())
                .child(secondary(look, "Step 1 of 4")),
        )
        .child(
            div().p(px(10.)).flex_shrink_0().child(
                card(look)
                    .p(px(10.))
                    .gap(px(4.))
                    .text_size(px(12.5))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(small_icon(IconName::Sparkles, PURPLE))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Claude Code")),
                    )
                    .child(div().child(
                        "The new accounts table holds one row per account, and entries replaces ledger_entries. Check the BRIN index on posted_at: it only helps while rows arrive in time order.",
                    )),
            ),
        )
        .child(editor(look))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(44.))
                .px(px(12.))
                .flex_shrink_0()
                .border_t_1()
                .border_color(c(look.line))
                .child(button(look, Kind::Secondary, "‹ Prev").opacity(0.4))
                .child(div().flex_1())
                .child(secondary(look, "1 / 4"))
                .child(div().flex_1())
                .child(button(look, Kind::Primary, "Next ›")),
        )
}

/// Frame 70 (light): the opened PR review as a tab, step 1 of 4, with the
/// Showing bar under the header.
pub fn view_tab(look: Look) -> Div {
    let middle = div()
        .child(header_with(
            look,
            "# payments-rewrite",
            St::Working,
            "Ledger schema migration",
            None,
            vec![secondary(look, "Claude Code · devbox-1")],
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(14.))
                .py(px(7.))
                .flex_shrink_0()
                .bg(c(look.attention_bg))
                .border_b_1()
                .border_color(c(look.line))
                .text_size(px(12.5))
                .child(small_icon(IconName::Sparkles, PURPLE))
                .child(div().flex_1().child("Showing “PR review: ledger schema”, prepared by Claude Code"))
                .child(button(look, Kind::Secondary, "Revert"))
                .child(button(look, Kind::Primary, "Keep")),
        )
        .child(
            div()
                .flex()
                .items_center()
                .h(px(32.))
                .flex_shrink_0()
                .bg(c(look.panel))
                .border_b_1()
                .border_color(c(look.line))
                .child(tab(look, Some(status(St::Working).into_any_element()), "claude", false, false))
                .child(tab(look, Some(small_icon(IconName::SquareTerminal, look.secondary).into_any_element()), "zsh", false, false))
                .child(tab(look, Some(small_icon(IconName::Sparkles, PURPLE).into_any_element()), "PR review: ledger schema", true, true))
                .child(div().px(px(10.)).child(small_icon(IconName::Plus, look.secondary)))
                .child(div().flex_1()),
        )
        .child(div().flex_1().min_h_0().flex().bg(c(look.term)).child(steps(look)).child(main_area(look)));
    shell(look, Side::at("Ledger schema migration"), middle, None)
}
