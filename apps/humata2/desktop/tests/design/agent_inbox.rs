//! The Agent inbox (INBOXHOOKS): one column of full cards for the agents
//! stopped until the owner answers, drawn from the signed-off mockups
//! (`milestones/mockups/boardsimple/b2-*`, `b3-1`, `b3-2`) at their own
//! 1600x950 size, 1x.

use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::mock_sidebar::sidebar;

fn tone(look: Look, light: u32, dark: u32) -> Rgba {
    c(if look.dark { dark } else { light })
}

fn soft(look: Look) -> Rgba {
    tone(look, 0x55555b, 0xc4c4ca)
}

fn amber_ring(look: Look) -> Rgba {
    tone(look, 0xf0b35a, 0xb9812f)
}

fn panel_bg(look: Look) -> Rgba {
    tone(look, 0xf5f5f8, 0x2a2a2d)
}

/// Text with the given phrases in bold; it wraps like any text.
fn rich(text: &'static str, bold: &[&str]) -> StyledText {
    let mut spans: Vec<_> = bold
        .iter()
        .filter_map(|phrase| text.find(phrase).map(|start| (start..start + phrase.len(), HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() })))
        .collect();
    spans.sort_by_key(|(range, _)| range.start);
    StyledText::new(text).with_highlights(spans)
}

fn spacer() -> Div {
    div().flex_1().min_w_0()
}

fn lnk(look: Look, text: &'static str) -> Div {
    div().flex_shrink_0().whitespace_nowrap().text_color(c(look.accent)).child(text)
}

fn btn(look: Look, text: &'static str, primary: bool) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(12.))
        .py(px(6.))
        .rounded(px(7.))
        .border_1()
        .text_size(px(13.))
        .font_weight(if primary { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
        .when(primary, |b| b.bg(c(look.accent)).border_color(c(look.accent)).text_color(c(0xffffff)))
        .when(!primary, |b| b.bg(c(look.card)).border_color(tone(look, 0xd4d4da, 0x48484d)))
        .child(text)
}

// ---- Page chrome

fn top_bar(look: Look, sub: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .h(px(60.))
        .px(px(28.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .child(div().text_size(px(17.)).font_weight(FontWeight::BOLD).child("Agent inbox"))
        .child(div().text_size(px(13.)).text_color(c(look.secondary)).child(sub))
        .child(spacer())
        .child(lnk(look, "History"))
}

fn chip(look: Look, text: &'static str, count: &'static str, on: bool) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .flex()
        .px(px(9.))
        .py(px(3.))
        .rounded(px(6.))
        .text_size(px(12.5))
        .when(!on, |chip| chip.bg(c(look.fill)).text_color(soft(look)))
        .when(on, |chip| chip.bg(c(look.text)).text_color(c(look.content)).font_weight(FontWeight::MEDIUM))
        .child(text)
        .when(!count.is_empty(), |chip| chip.child(div().ml(px(4.)).opacity(0.65).font_weight(FontWeight::NORMAL).child(count)))
}

fn channel_bar(look: Look, chans: &[(&'static str, &'static str)]) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(28.))
        .pt(px(12.))
        .pb(px(4.))
        .flex_shrink_0()
        .whitespace_nowrap()
        .children(chans.iter().enumerate().map(|(index, (name, count))| chip(look, name, count, index == 0)))
        .child(spacer())
        .child(
            div()
                .mx(px(2.))
                .text_size(px(12.))
                .text_color(c(look.secondary))
                .child("Agents that are stopped until you answer. Changes to your to-dos wait in To-do instead."),
        )
}

fn page(look: Look, inbox: Option<u32>, ledger_orange: bool, top: Div, body: Div) -> Div {
    div()
        .size_full()
        .flex()
        .line_height(relative(1.35))
        .child(sidebar(look, "Agent inbox", inbox, ledger_orange))
        .child(handle(look, false))
        .child(div().flex_1().min_w_0().flex().flex_col().bg(c(look.content)).child(top).child(body))
}

// ---- The card

fn cl(look: Look, title: &'static str, small: Option<&'static str>, first: bool) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(px(4.))
        .mt(if first { px(0.) } else { px(10.) })
        .mb(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(look.secondary))
        .child(title)
        .children(small.map(|small| div().text_size(px(11.5)).font_weight(FontWeight::NORMAL).child(small)))
}

fn facts(look: Look, rows: &[(&'static str, &'static str, &'static str)]) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(8.))
        .bg(panel_bg(look))
        .children(rows.iter().map(|(label, text, bold)| {
            div()
                .flex()
                .child(div().w(px(66.)).flex_shrink_0().text_color(c(look.secondary)).child(*label))
                .child(div().flex_1().min_w_0().child(rich(text, &[*bold])))
        }))
}

fn why(look: Look, text: &'static str, item: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .child(cl(look, "WHY", Some("the agent's reason"), false))
        .child(div().text_color(tone(look, 0x2c2c30, 0xe2e2e6)).line_height(relative(1.5)).child(text))
        .child(div().text_color(c(look.secondary)).line_height(relative(1.5)).child(item))
}

fn mark(look: Look, glyph: &'static str, warn: bool, text: &'static str, bold: &'static str) -> Div {
    let hue = if warn { tone(look, 0xb25000, 0xffb340) } else { tone(look, 0x0f766e, 0x35c4b5) };
    div()
        .flex()
        .gap(px(8.))
        .py(px(1.5))
        .line_height(relative(1.45))
        .child(div().w(px(14.)).flex_shrink_0().flex().justify_center().font_weight(FontWeight::SEMIBOLD).text_color(hue).child(glyph))
        .child(div().flex_1().min_w_0().child(rich(text, &[bold])))
}

fn undone(look: Look, text: &'static str) -> Div {
    div().flex().flex_col().child(cl(look, "CAN IT BE UNDONE?", None, false)).child(div().child(text))
}

fn always(look: Look, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .ml(px(10.))
        .text_size(px(12.5))
        .text_color(soft(look))
        .whitespace_nowrap()
        .child(div().size(px(15.)).flex_shrink_0().rounded(px(4.)).border_1().border_color(tone(look, 0xb9b9c0, 0x6a6a70)).bg(c(look.card)))
        .child(text)
}

fn rule(look: Look, name: &'static str) -> Div {
    div()
        .flex()
        .whitespace_nowrap()
        .text_size(px(12.5))
        .text_color(c(look.secondary))
        .child("Asked because ")
        .child(div().font_weight(FontWeight::MEDIUM).text_color(soft(look)).child(name))
        .child(" is set to Ask me · ")
        .child(div().text_color(c(look.accent)).child("Change this rule"))
}

fn yes_no(look: Look) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(btn(look, "Yes", true).min_w(px(64.)).flex().justify_center())
        .child(btn(look, "No", false).min_w(px(64.)).flex().justify_center())
}

/// The full card: dot, title, who and the link, two columns, the extra band,
/// and the footer.
fn card(look: Look, title: &'static str, who: &'static str, left: Div, right: Div, extra: Option<Div>, foot: Div) -> Div {
    div().mx(px(-1.)).p(px(1.)).rounded(px(13.)).bg(amber_ring(look)).child(
        div()
            .rounded(px(12.))
            .border_1()
            .border_color(amber_ring(look))
            .bg(c(look.card))
            .px(px(18.))
            .pt(px(16.))
            .pb(px(14.))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().size(px(8.)).flex_shrink_0().rounded_full().bg(c(ORANGE)))
                    .child(div().text_size(px(15.5)).font_weight(FontWeight::BOLD).child(title))
                    .child(spacer())
                    .child(
                        div()
                            .flex()
                            .gap(px(4.))
                            .whitespace_nowrap()
                            .text_size(px(12.5))
                            .text_color(c(look.secondary))
                            .child(who)
                            .child("  ")
                            .child(div().text_color(c(look.accent)).child("Open session ↗")),
                    ),
            )
            .child(div().flex().gap(px(28.)).mt(px(10.)).child(left.flex_1().min_w_0()).child(right.flex_1().min_w_0()))
            .children(extra)
            .child(div().flex().items_center().gap(px(8.)).mt(px(14.)).pt(px(12.)).border_t_1().border_color(c(look.line)).child(foot)),
    )
}

fn card_foot(look: Look, buttons: Div, always_text: &'static str, rule_name: &'static str) -> Div {
    div()
        .flex()
        .flex_1()
        .items_center()
        .gap(px(8.))
        .child(buttons)
        .child(always(look, always_text))
        .child(spacer())
        .child(rule(look, rule_name))
}

fn yes_no_why(look: Look) -> Div {
    yes_no(look).gap(px(8.)).child(btn(look, "Ask why", false).ml(px(8.)))
}

fn body(card: Div) -> Div {
    div().flex_1().px(px(28.)).py(px(16.)).flex().flex_col().gap(px(14.)).min_h_0().child(card)
}

fn page_with_card(look: Look, working: bool, chans: &[(&'static str, &'static str)], card: Div) -> Div {
    page(look, Some(1), !working, div().flex().flex_col().child(top_bar(look, "1 waiting on you")).child(channel_bar(look, chans)), body(card))
}

// ---- Frames

/// b2-1: nothing waiting.
pub fn empty(look: Look) -> Div {
    let teal = tone(look, 0x0f766e, 0x35c4b5);
    let center = div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .pb(px(80.))
        .child(
            div()
                .size(px(46.))
                .rounded_full()
                .bg(tone(look, 0xdff5f1, 0x153a36))
                .text_color(teal)
                .text_size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .child("✓"),
        )
        .child(div().mt(px(6.)).text_size(px(18.)).font_weight(FontWeight::BOLD).child("Nothing is waiting on you"))
        .child(
            div()
                .max_w(px(520.))
                .text_center()
                .text_color(soft(look))
                .line_height(relative(1.5))
                .child("5 agents are working. An agent stops and asks here before it moves or copies files or sessions, needs new access, or has a question for you."),
        )
        .child(
            div()
                .mt(px(10.))
                .flex()
                .gap(px(4.))
                .text_color(c(look.secondary))
                .child("Today: 2 answered · 31 actions followed your rules without asking ·")
                .child(div().text_color(c(look.accent)).child("History")),
        )
        .child(
            div()
                .flex()
                .gap(px(4.))
                .text_color(c(look.secondary))
                .child("3 to-dos need attention when you get to them ·")
                .child(div().text_color(c(look.accent)).child("Open To-do")),
        );
    page(look, None, false, top_bar(look, "nothing is waiting on you"), center)
}

fn move_card(
    look: Look,
    title: &'static str,
    who: &'static str,
    grid: &[(&'static str, &'static str, &'static str)],
    why_text: &'static str,
    item: &'static str,
    affects: Vec<Div>,
    undo: &'static str,
    always_text: &'static str,
    rule_name: &'static str,
) -> Div {
    let left = div().flex().flex_col().child(facts(look, grid)).child(why(look, why_text, item));
    let right = div()
        .flex()
        .flex_col()
        .child(cl(look, "WHAT ELSE IT AFFECTS", Some("worked out by Humata"), true))
        .children(affects)
        .child(undone(look, undo));
    card(look, title, who, left, right, None, card_foot(look, yes_no_why(look), always_text, rule_name))
}

/// b2-2: a file move.
pub fn file_move(look: Look) -> Div {
    let card = move_card(
        look,
        "Move the folder ledger-2026 from This Mac to devbox-1?",
        "claude in Ledger entries · payments-rewrite · 2m ago",
        &[
            ("From", "This Mac · Ledger entries · api/fixtures/ledger-2026/", "This Mac"),
            ("To", "devbox-1 · Ledger entries · api/fixtures/ledger-2026/", "devbox-1"),
            ("Size", "412 files · 1.8 GB", ""),
        ],
        "“The backfill runs on devbox-1 and reads these fixture rows. Nothing on This Mac uses them after today's run.”",
        "Working on PAY-3 Ledger schema v2",
        vec![
            mark(look, "!", true, "2 files on devbox-1 are replaced: seed.sql (the one there is 3 days newer) and accounts.csv (same content)", "2 files on devbox-1 are replaced:"),
            mark(look, "!", true, "The folder leaves This Mac: this is a move, not a copy", ""),
            mark(look, "✓", false, "410 files are new on devbox-1", ""),
            mark(look, "✓", false, "No running terminal is inside this folder", ""),
        ],
        "Yes, for 7 days. Undo moves the folder back and restores the 2 replaced files.",
        "Always allow for this session",
        "Move files to another machine",
    );
    page_with_card(look, false, &[("All", "1"), ("payments-rewrite", "1"), ("infra", "")], card)
}

/// b2-3: a session move.
pub fn session_move(look: Look) -> Div {
    let card = move_card(
        look,
        "Move the session Benchmark BRIN from devbox-1 to gpu-box?",
        "codex in Benchmark BRIN · infra · 4m ago",
        &[
            ("From", "devbox-1 · infra · Benchmark BRIN", "devbox-1"),
            ("To", "gpu-box · infra · Benchmark BRIN", "gpu-box"),
            ("Carries", "2 repos (infra, ledger) · 2.1 GB · 3 uncommitted files, as they are", ""),
        ],
        "“pgbench at 30M rows needs more memory than devbox-1 has; it started swapping at 22M. gpu-box has 64 GB free.”",
        "Working on INF-9 Benchmark BRIN on real data",
        vec![
            mark(look, "✓", false, "2 terminals move along and keep running: codex, and zsh running pgbench (14 min in)", "2 terminals move along and keep running:"),
            mark(look, "✓", false, "INF-9 stays in progress with this session", ""),
            mark(look, "!", true, "gpu-box has no clone of acme/ledger yet; Humata clones it there first (about 2 min)", ""),
            mark(look, "!", true, "Its fork, Partial BRIN, stays on devbox-1", ""),
        ],
        "Yes. Move it back at any time; nothing is removed from devbox-1 until the move has finished.",
        "Always allow for this session",
        "Move a session to another machine",
    );
    page_with_card(look, true, &[("All", "1"), ("payments-rewrite", ""), ("infra", "1")], card)
}

/// b3-1: a copy whose reason the owner asked for.
pub fn copy_ask_why(look: Look) -> Div {
    let left = div()
        .flex()
        .flex_col()
        .child(facts(
            look,
            &[
                ("From", "Refund retries (copilot's session) · devbox-1 · api/src/retry/", "Refund retries"),
                ("To", "Ledger entries · devbox-1 · api/src/retry/", "Ledger entries"),
                ("Files", "backoff.rs (4 KB) · policy.rs (2 KB) · client.rs (11 KB)", ""),
            ],
        ))
        .child(why(
            look,
            "“Refund retries already wrote the backoff policy. The ledger client should use the same one instead of a second copy that drifts.”",
            "Working on PAY-3 Ledger schema v2",
        ));
    let right = div()
        .flex()
        .flex_col()
        .child(cl(look, "WHAT ELSE IT AFFECTS", Some("worked out by Humata"), true))
        .child(mark(look, "!", true, "client.rs is overwritten, and yours has 2 uncommitted changes that would be lost (+62 −21)", "client.rs is overwritten"))
        .child(mark(look, "✓", false, "backoff.rs and policy.rs are new in Ledger entries", ""))
        .child(mark(look, "✓", false, "Nothing changes in Refund retries; copilot isn't interrupted", ""))
        .child(mark(look, "!", true, "The copies are uncommitted work from another session's branch, refund-retries", ""))
        .child(undone(look, "Yes, for 7 days. Undo removes the 2 new files and restores your client.rs with its changes."));
    let asked = div()
        .mt(px(12.))
        .px(px(13.))
        .py(px(10.))
        .rounded(px(9.))
        .bg(tone(look, 0xeef4ff, 0x20304d))
        .flex()
        .flex_col()
        .child(div().mb(px(3.)).font_weight(FontWeight::SEMIBOLD).child("You asked why · the copy is on hold"))
        .child({
            let text = "claude, 40s ago: “I need backoff.rs and policy.rs. client.rs only came along because it imports them. If you’d rather keep your client.rs, say no and I’ll copy the two files and add the imports to yours by hand.”";
            let dim = HighlightStyle { color: Some(c(look.secondary).into()), ..Default::default() };
            div().line_height(relative(1.5)).text_color(tone(look, 0x2c2c30, 0xe2e2e6)).child(StyledText::new(text).with_highlights([(0.."claude, 40s ago:".len(), dim)]))
        });
    let buttons = yes_no(look).child(btn(look, "Asked why ✓", false).ml(px(8.)).text_color(c(look.secondary)));
    let card = card(
        look,
        "Copy 3 files from Refund retries into Ledger entries?",
        "claude in Ledger entries · payments-rewrite · 1m ago",
        left,
        right,
        Some(asked),
        card_foot(look, buttons, "Always allow between these two sessions", "Copy files from another session"),
    );
    page_with_card(look, false, &[("All", "1"), ("payments-rewrite", "1"), ("infra", "")], card)
}

/// b3-2: an agent's question.
pub fn question(look: Look) -> Div {
    let left = div()
        .flex()
        .flex_col()
        .child(cl(look, "WHAT IT NEEDS TO KNOW", None, true))
        .child(
            div()
                .text_color(tone(look, 0x2c2c30, 0xe2e2e6))
                .line_height(relative(1.5))
                .child("“The charge stores two currencies: the one the customer paid in and the one you were paid in. Today a refund goes out in yours, so the customer gets less back when the rate has moved. Which should a retried refund use?”"),
        )
        .child(div().text_color(c(look.secondary)).line_height(relative(1.5)).child("Working on PAY-7 Refund retry with backoff"));
    let right = div()
        .flex()
        .flex_col()
        .child(cl(look, "YOUR ANSWER", None, true))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .items_start()
                .child(btn(look, "The currency the customer paid in", false))
                .child(btn(look, "The currency I was paid in", false))
                .child(btn(look, "Leave it as it is and add a to-do", false))
                .child(
                    div()
                        .w_full()
                        .mt(px(4.))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(7.))
                        .border_1()
                        .border_color(tone(look, 0xd9d9de, 0x48484d))
                        .flex()
                        .justify_between()
                        .text_color(c(look.secondary))
                        .child("Or type an answer…")
                        .child("↵"),
                ),
        );
    let foot = div()
        .flex()
        .flex_1()
        .items_center()
        .gap(px(8.))
        .child(div().text_color(c(look.secondary)).child("A question changes nothing by itself; the answer goes to the agent as you give it."))
        .child(spacer())
        .child(div().whitespace_nowrap().text_size(px(12.5)).text_color(c(look.secondary)).child("copilot is stopped until you answer"));
    let card = card(look, "Which currency should a retried refund use?", "copilot in Refund retries · payments-rewrite · 6m ago", left, right, None, foot);
    page_with_card(look, true, &[("All", "1"), ("payments-rewrite", "1"), ("infra", "")], card)
}
