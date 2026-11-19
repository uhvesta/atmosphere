//! The sidebar the signed-off Agent inbox mockups (`boardsimple/b2` to
//! `b9`) draw: plain text rows, no icons, a fixed tree of sessions.

use gpui_kit::{prelude::*, *};

use crate::kit::*;

const BLUE: u32 = 0x0a66f5;
const GREEN_HEX: u32 = 0x28c840;
const ORANGE_HEX: u32 = 0xff9500;
const GREY_HEX: u32 = 0xb0b0b6;

fn tone(look: Look, light: u32, dark: u32) -> Rgba {
    c(if look.dark { dark } else { light })
}

fn dot(hex: u32) -> Div {
    div().size(px(8.)).rounded_full().flex_shrink_0().bg(c(hex))
}

/// The sidebar as the mockups draw it. `selected` names the lit row,
/// `inbox_count` is the Agent inbox badge and `ledger_orange` turns the
/// Ledger entries dot from green to orange.
pub(crate) fn sidebar(look: Look, selected: &str, inbox_count: Option<u32>, ledger_orange: bool) -> Div {
    sidebar_with(look, selected, inbox_count, ledger_orange, false)
}

/// The same sidebar with pr-reviews unfolded to its one session, "Ledger
/// reads behind a flag" (the pull request review session of frame b7-2).
pub(crate) fn sidebar_pr_reviews_open(look: Look, selected: &str, inbox_count: Option<u32>, ledger_orange: bool) -> Div {
    sidebar_with(look, selected, inbox_count, ledger_orange, true)
}

fn sidebar_with(look: Look, selected: &str, inbox_count: Option<u32>, ledger_orange: bool, pr_open: bool) -> Div {
    let muted = tone(look, 0x7a7a80, 0x9a9aa1);
    let blue_bg = if look.dark { 0x2f7cf6 } else { BLUE };
    let row = |text: &'static str, count: Option<u32>| {
        let on = selected == text;
        div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(10.))
            .py(px(6.))
            .mb(px(2.))
            .rounded(px(7.))
            .line_height(px(17.55))
            .when(on, |r| r.bg(c(blue_bg)).text_color(c(0xffffff)).font_weight(FontWeight::SEMIBOLD))
            .child(text)
            .children(count.map(|count| div().text_size(px(12.)).text_color(if on { c(0xdbe8ff) } else { muted }).child(count.to_string())))
    };
    let heading = |text: &'static str| div().px(px(10.)).pt(px(16.)).pb(px(6.)).text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(muted).child(text);
    let channel = |text: &'static str, open: bool| {
        div()
            .flex()
            .gap(px(8.))
            .px(px(10.))
            .py(px(5.))
            .line_height(px(17.55))
            .when(open, |r| r.font_weight(FontWeight::SEMIBOLD))
            .child(div().w(px(8.)).text_color(muted).font_weight(FontWeight::NORMAL).child(if open { "⌄" } else { "›" }))
            .child(text)
    };
    let lead = |hex: Option<u32>| match hex {
        Some(hex) => dot(hex).into_any_element(),
        None => div().w(px(8.)).text_size(px(11.)).font_weight(FontWeight::BOLD).text_color(c(0xe0342b)).flex().justify_center().child("✕").into_any_element(),
    };
    let session = |hex: Option<u32>, text: &'static str| {
        let on = selected == text;
        div()
            .flex()
            .items_center()
            .gap(px(9.))
            .py(px(5.))
            .line_height(px(17.55))
            .when(!on, |r| r.pl(px(30.)).pr(px(10.)))
            .when(on, |r| r.ml(px(20.)).pl(px(10.)).rounded(px(7.)).bg(c(blue_bg)).text_color(c(0xffffff)).font_weight(FontWeight::SEMIBOLD))
            .child(lead(hex))
            .child(text)
    };
    let host = |text: &'static str| div().flex().items_center().gap(px(9.)).px(px(10.)).py(px(5.)).line_height(px(17.55)).child(dot(GREEN_HEX)).child(text);
    let ledger = if ledger_orange { ORANGE_HEX } else { GREEN_HEX };
    div()
        .w(px(280.))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .px(px(12.))
        .pt(px(14.))
        .pb(px(14.))
        .bg(tone(look, 0xf3f3f6, 0x27272a))
        .border_r_1()
        .border_color(tone(look, 0xe6e6ea, 0x343437))
        .text_size(px(13.))
        .child(traffic_lights().px(px(8.)).pt(px(6.)).pb(px(18.)))
        .child(div().flex().justify_between().px(px(10.)).py(px(6.)).mb(px(14.)).rounded(px(7.)).bg(tone(look, 0xe6e6ea, 0x3a3a3d)).line_height(px(17.55)).text_color(muted).child("Search").child("⌘K"))
        .child(row("Home", None))
        .child(row("Agent inbox", inbox_count))
        .child(row("To-do", None))
        .child(row("Desk", None))
        .child(heading("Channels"))
        .child(channel("payments-rewrite", true))
        .child(session(Some(ledger), "Ledger entries"))
        .child(session(Some(GREEN_HEX), "Refund retries"))
        .child(session(Some(GREY_HEX), "API rate limiter"))
        .child(channel("infra", true))
        .child(session(None, "Upgrade Postgres 17"))
        .child(session(Some(GREEN_HEX), "Benchmark BRIN"))
        .child(channel("web", true))
        .child(session(Some(GREEN_HEX), "Ledger totals"))
        .child(session(Some(GREEN_HEX), "Totals, another way"))
        .child(channel("pr-reviews", pr_open))
        .children(pr_open.then(|| session(Some(GREEN_HEX), "Ledger reads behind a flag")))
        .child(heading("Hosts"))
        .child(host("This Mac"))
        .child(host("devbox-1"))
        .child(host("gpu-box"))
        .child(div().flex_1())
        .child(row("Settings", None))
}
