//! The Board (BD-1 to BD-9): the kanban, an item open in its drawer, a
//! table option, and a dashboard option where every number opens what it
//! counts (NV-7, NV-8).

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::{page, toolbar};
use crate::sidebar::Side;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum View {
    Kanban,
    Table,
    Dashboard,
}

struct Item {
    key: &'static str,
    title: &'static str,
    lane: usize,
    by: Option<&'static str>,
    channel: &'static str,
    state: Option<St>,
    asks: bool,
}

const LANES: [&str; 5] = ["Backlog", "To do", "In progress", "In review", "Done"];

const ITEMS: &[Item] = &[
    Item { key: "PAY-12", title: "Multi-currency ledgers", lane: 0, by: None, channel: "payments-rewrite", state: None, asks: false },
    Item { key: "INF-4", title: "Move CI to devbox runners", lane: 0, by: None, channel: "infra", state: None, asks: false },
    Item { key: "PAY-9", title: "Backfill progress bar", lane: 1, by: Some("Ledger schema migration"), channel: "payments-rewrite", state: Some(St::NeedsYou), asks: false },
    Item { key: "PAY-7", title: "Refund retry with backoff", lane: 1, by: None, channel: "payments-rewrite", state: None, asks: false },
    Item { key: "PAY-3", title: "Ledger schema v2", lane: 2, by: Some("Ledger schema migration"), channel: "payments-rewrite", state: Some(St::NeedsYou), asks: true },
    Item { key: "INF-2", title: "Postgres 17 upgrade", lane: 2, by: Some("Upgrade Postgres 17"), channel: "infra", state: Some(St::Exited), asks: false },
    Item { key: "PAY-5", title: "API rate limiter", lane: 3, by: Some("API rate limiter"), channel: "payments-rewrite", state: Some(St::Idle), asks: false },
    Item { key: "PAY-2", title: "Currency codes table", lane: 4, by: Some("Currency codes"), channel: "payments-rewrite", state: None, asks: false },
    Item { key: "INF-1", title: "BRIN benchmark harness", lane: 4, by: Some("Benchmark BRIN"), channel: "infra", state: None, asks: false },
];

fn bar(look: Look, view: View) -> Div {
    let on = match view {
        View::Kanban => 0,
        View::Table => 1,
        View::Dashboard => 2,
    };
    toolbar(
        look,
        "Board",
        Some("All channels"),
        vec![
            popup(look, "Scope: All channels"),
            segmented(look, &["Board", "Table", "Dashboard"], on),
            field(look, "Filter", true).w(px(150.)),
            button(look, Kind::Primary, "New item"),
        ],
    )
}

fn by(look: Look, item: &Item) -> Option<Div> {
    item.by.map(|session| {
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(6.))
            .py(px(1.))
            .rounded(px(4.))
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .bg(c(look.fill))
            .text_size(px(11.))
            .text_color(c(look.secondary))
            .children(item.state.map(status))
            .child(session)
    })
}

fn item_card(look: Look, item: &Item, on: bool) -> Div {
    card(look)
        .min_w_0()
        .overflow_hidden()
        .p(px(9.))
        .gap(px(6.))
        .when(on, |card| card.border_2().border_color(c(look.accent)))
        .when(item.asks, |card| card.bg(c(look.attention_bg)))
        .child(div().flex().items_center().gap(px(6.)).child(secondary(look, item.key)).child(div().flex_1()).child(secondary(look, item.channel).min_w_0().overflow_hidden().whitespace_nowrap()))
        .child(div().font_weight(FontWeight::MEDIUM).child(item.title))
        .children(by(look, item))
        .when(item.asks, |card| {
            card.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.))
                    .text_color(c(ORANGE))
                    .child(small_icon(IconName::Inbox, ORANGE))
                    .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child("Asks to move to Done"))
                    .child(button(look, Kind::Secondary, "Review")),
            )
        })
}

/// The lanes side by side, for every channel or for one.
pub fn lanes(look: Look, channel: Option<&str>, selected: Option<&str>) -> Div {
    div()
        .flex()
        .gap(px(10.))
        .p(px(14.))
        .size_full()
        .text_size(px(12.5))
        .children(LANES.iter().enumerate().map(|(index, lane)| {
            let items: Vec<&Item> = ITEMS.iter().filter(|item| item.lane == index && channel.is_none_or(|channel| item.channel == channel)).collect();
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(8.))
                .rounded(px(9.))
                .bg(c(look.panel))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(*lane))
                        .child(secondary(look, items.len().to_string()))
                        .child(div().flex_1())
                        .child(small_icon(IconName::Plus, look.tertiary)),
                )
                .children(items.into_iter().map(|item| item_card(look, item, selected == Some(item.key))))
        }))
}

/// The kanban across every channel, with the sessions working each item.
pub fn kanban(look: Look) -> Div {
    page(look, Side::at("Board"), bar(look, View::Kanban), lanes(look, None, None))
}

/// An item open in its drawer over the board (BD-3).
pub fn drawer(look: Look) -> Div {
    let line = |name: &'static str, value: Div| div().flex().items_center().gap(px(10.)).child(div().w(px(80.)).text_color(c(look.secondary)).child(name)).child(value);
    let drawer = div()
        .w(px(380.))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(12.))
        .p(px(16.))
        .bg(c(look.content))
        .border_l_1()
        .border_color(c(look.line))
        .text_size(px(12.5))
        .child(div().flex().items_center().gap(px(8.)).child(secondary(look, "PAY-3")).child(div().flex_1()).child(icon(IconName::Maximize2, look.secondary)).child(icon(IconName::X, look.secondary)))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Ledger schema v2"))
        .child(line("Status", popup(look, "In progress")))
        .child(line("Channel", div().child("# payments-rewrite")))
        .child(line("Sessions", div().flex().flex_col().gap(px(4.)).child(div().flex().items_center().gap(px(6.)).child(status(St::NeedsYou)).child("Ledger schema migration")).child(div().flex().items_center().gap(px(6.)).child(status(St::Working)).child("Try BRIN index"))))
        .child(line("Depends on", chip(look, "PAY-2 Currency codes ✓", look.secondary)))
        .child(hline(look))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(c(look.attention_bg))
                .child(div().flex().items_center().gap(px(6.)).child(status(St::NeedsYou)).child(div().font_weight(FontWeight::SEMIBOLD).child("Claude Code asks to move this to Done")))
                .child(secondary(look, "“Migration and backfill pass on staging; 3.1M rows in 4m12s.”"))
                .child(div().flex().gap(px(6.)).child(button(look, Kind::Primary, "Move to Done")).child(button(look, Kind::Secondary, "Keep in progress")).child(button(look, Kind::Secondary, "Open session"))),
        )
        .child(label(look, "Description"))
        .child(div().text_color(c(look.secondary)).child("Split postings into ledger_entries and ledger_balances, backfill from the old table in batches, and keep both in step until cut-over."))
        .child(label(look, "History"))
        .child(secondary(look, "Moved to In progress by Ledger schema migration · 1h"))
        .child(secondary(look, "Created by you · Mon"));
    page(
        look,
        Side::at("Board"),
        bar(look, View::Kanban),
        div().flex().size_full().child(div().flex_1().min_w_0().overflow_hidden().child(lanes(look, None, Some("PAY-3")))).child(drawer),
    )
}

/// Option: the same items as a table, grouped by status.
pub fn table(look: Look) -> Div {
    let head = |text: &'static str, width: Option<f32>| {
        let cell = div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(look.secondary)).child(text);
        match width {
            Some(width) => cell.w(px(width)),
            None => cell.flex_1(),
        }
    };
    let row = |item: &Item| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .h(px(32.))
            .border_b_1()
            .border_color(c(look.line))
            .when(item.asks, |row| row.bg(c(look.attention_bg)))
            .child(div().w(px(60.)).text_color(c(look.secondary)).child(item.key))
            .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(item.title))
            .child(div().w(px(220.)).children(by(look, item)))
            .child(div().w(px(130.)).text_color(c(look.secondary)).child(item.channel))
            .child(div().w(px(90.)).children(item.asks.then(|| button(look, Kind::Secondary, "Review"))))
    };
    let mut body = card(look).overflow_hidden().child(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .h(px(30.))
            .bg(c(look.panel))
            .border_b_1()
            .border_color(c(look.line))
            .child(head("Key", Some(60.)))
            .child(head("Title", None))
            .child(head("Worked on by", Some(220.)))
            .child(head("Channel", Some(130.)))
            .child(head("", Some(90.))),
    );
    for (index, lane) in LANES.iter().enumerate() {
        let items: Vec<&Item> = ITEMS.iter().filter(|item| item.lane == index).collect();
        body = body
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(12.))
                    .h(px(28.))
                    .bg(c(look.fill))
                    .border_b_1()
                    .border_color(c(look.line))
                    .child(small_icon(IconName::ChevronDown, look.secondary))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(*lane))
                    .child(secondary(look, items.len().to_string())),
            )
            .children(items.into_iter().map(row));
    }
    page(look, Side::at("Board"), bar(look, View::Table), div().p(px(14.)).text_size(px(12.5)).child(body))
}

/// Option: a dashboard. Every tile and bar is a link to the items it counts.
pub fn dashboard(look: Look) -> Div {
    let tile = |title: &'static str, value: &'static str, note: &'static str, hex: u32| {
        card(look)
            .flex_1()
            .p(px(12.))
            .gap(px(2.))
            .child(secondary(look, title))
            .child(div().text_size(px(24.)).font_weight(FontWeight::SEMIBOLD).text_color(c(hex)).child(value))
            .child(div().text_size(px(11.)).text_color(c(look.accent)).child(note))
    };
    // Items per status per channel, as stacked bars.
    let lane_hex = [look.tertiary, GREY, look.accent, PURPLE, GREEN];
    let stack = |name: &'static str, counts: [u32; 5]| {
        let total: u32 = counts.iter().sum();
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(div().w(px(130.)).child(name))
            .child(
                div().flex_1().flex().h(px(16.)).rounded(px(4.)).overflow_hidden().gap(px(2.)).children(counts.iter().enumerate().filter(|(_, n)| **n > 0).map(|(lane, n)| {
                    div().flex_grow(*n as f32).flex_basis(px(0.)).h_full().bg(c(lane_hex[lane]))
                })),
            )
            .child(div().w(px(30.)).flex().justify_end().child(secondary(look, total.to_string())))
    };
    let legend = div().flex().gap(px(12.)).children(LANES.iter().enumerate().map(|(lane, name)| {
        div().flex().items_center().gap(px(5.)).child(div().size(px(8.)).rounded(px(2.)).bg(c(lane_hex[lane]))).child(secondary(look, *name))
    }));
    // Items done per day over two weeks.
    let done = [1u32, 0, 2, 1, 3, 0, 0, 2, 4, 1, 3, 2, 0, 5];
    let throughput = div().flex().items_end().gap(px(6.)).h(px(90.)).children(done.iter().enumerate().map(|(day, n)| {
        div()
            .flex_1()
            .h(px(4. + 16. * *n as f32))
            .rounded_t(px(3.))
            .bg(c(if day == done.len() - 1 { look.accent } else { look.fill }))
    }));
    let session_row = |state: St, name: &'static str, items: &'static str, open: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(30.))
            .border_b_1()
            .border_color(c(look.line))
            .child(status(state))
            .child(div().flex_1().child(name))
            .child(div().w(px(80.)).text_color(c(look.secondary)).child(items))
            .child(div().w(px(70.)).text_color(c(look.accent)).child(open))
    };
    page(
        look,
        Side::at("Board"),
        bar(look, View::Dashboard),
        div()
            .p(px(16.))
            .text_size(px(12.5))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(tile("Waiting on you", "2", "Review the asks →", ORANGE))
                    .child(tile("In progress", "2", "1 session exited →", look.text))
                    .child(tile("Done this week", "11", "+4 on last week", look.text))
                    .child(tile("Unassigned", "3", "Start a session →", look.text)),
            )
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        card(look)
                            .flex_1()
                            .p(px(12.))
                            .gap(px(10.))
                            .child(label(look, "By channel"))
                            .child(stack("payments-rewrite", [1, 2, 1, 1, 1]))
                            .child(stack("infra", [1, 0, 1, 0, 1]))
                            .child(stack("pr-reviews", [0, 1, 0, 2, 3]))
                            .child(legend),
                    )
                    .child(
                        card(look)
                            .w(px(330.))
                            .p(px(12.))
                            .gap(px(8.))
                            .child(div().flex().child(label(look, "Done per day").flex_1()).child(secondary(look, "14 days")))
                            .child(throughput),
                    ),
            )
            .child(
                card(look)
                    .p(px(12.))
                    .gap(px(2.))
                    .child(label(look, "By session"))
                    .child(session_row(St::NeedsYou, "Ledger schema migration", "2 items", "Open"))
                    .child(session_row(St::Exited, "Upgrade Postgres 17", "1 item", "Resume"))
                    .child(session_row(St::Idle, "API rate limiter", "1 item", "Open")),
            ),
    )
}
