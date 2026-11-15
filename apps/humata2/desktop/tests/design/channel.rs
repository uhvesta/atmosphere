//! A channel's page, opened by clicking the channel itself (CH-11 to CH-15):
//! its tabs, the sessions tree table, Activity, and Repos and machines.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::page;
use crate::sidebar::Side;

/// The channel's tabs (CH-11).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Sessions,
    Inbox,
    Activity,
    Repos,
    Access,
}

fn header(look: Look, on: Tab) -> Div {
    let tab = |text: &'static str, count: Option<&'static str>, lit: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(5.))
            .h_full()
            .px(px(2.))
            .border_b_2()
            .border_color(c(if lit { look.accent } else { look.content }))
            .text_color(c(if lit { look.text } else { look.secondary }))
            .when(lit, |tab| tab.font_weight(FontWeight::MEDIUM))
            .child(text)
            .children(count.map(|count| badge(look, count, false)))
    };
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .h(px(52.))
                .px(px(20.))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child("# payments-rewrite"))
                .child(secondary(look, "acme/ledger, acme/api · This Mac, devbox-1"))
                .child(div().flex_1())
                .child(icon(IconName::Pin, look.secondary))
                .child(icon(IconName::Ellipsis, look.secondary))
                .child(button(look, Kind::Primary, "New session")),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(20.))
                .h(px(32.))
                .px(px(20.))
                .text_size(px(12.5))
                .child(tab("Sessions", Some("6"), on == Tab::Sessions))
                .child(tab("Inbox", Some("2"), on == Tab::Inbox))
                .child(tab("Board", None, false))
                .child(tab("Repos and machines", None, on == Tab::Repos))
                .child(tab("Agent access", None, on == Tab::Access))
                .child(tab("Activity", None, on == Tab::Activity)),
        )
}

/// The channel page on tab `on`, with `body` under the tabs.
pub fn frame(look: Look, on: Tab, body: Div) -> Div {
    page(
        look,
        Side::at("payments-rewrite"),
        header(look, on),
        body,
    )
}

struct Row {
    lines: &'static str,
    fold: Option<bool>,
    state: St,
    title: &'static str,
    kind: Option<&'static str>,
    inbox: Option<&'static str>,
    agent: &'static str,
    machine: &'static str,
    status: &'static str,
    started: &'static str,
}

const ROWS: &[Row] = &[
    Row { lines: "", fold: Some(true), state: St::NeedsYou, title: "Ledger schema migration", kind: None, inbox: Some("2"), agent: "Claude Code", machine: "devbox-1", status: "Needs you", started: "Today 11:40" },
    Row { lines: "├─ ", fold: Some(true), state: St::Working, title: "Try BRIN index", kind: Some("⑂ code + context"), inbox: None, agent: "Claude Code", machine: "devbox-1", status: "Working", started: "Today 12:05" },
    Row { lines: "│  └─ ", fold: None, state: St::Working, title: "Partial BRIN on posted_at", kind: Some("⑂ code only"), inbox: None, agent: "Codex", machine: "gpu-box", status: "Working", started: "Today 12:31" },
    Row { lines: "└─ ", fold: None, state: St::Idle, title: "Review the schema", kind: Some("⑂ context only"), inbox: None, agent: "Claude Code", machine: "This Mac", status: "Idle", started: "Today 12:50" },
    Row { lines: "", fold: Some(false), state: St::Working, title: "Refund retries", kind: None, inbox: None, agent: "Copilot", machine: "This Mac", status: "Working", started: "Yesterday" },
    Row { lines: "", fold: None, state: St::Idle, title: "API rate limiter", kind: None, inbox: None, agent: "Codex", machine: "devbox-1", status: "Idle · PR #491", started: "Mon" },
];

fn sessions_table(look: Look) -> Div {
    let head = |text: &'static str| div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(look.secondary)).child(text);
    let row = |row: &Row| {
        let status_hex = match row.state {
            St::NeedsYou => ORANGE,
            _ => look.secondary,
        };
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .h(px(34.))
            .border_b_1()
            .border_color(c(look.line))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().w(px(12.)).children(row.fold.map(|open| small_icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, look.tertiary))))
                    .child(div().font_family(MONO).text_size(px(11.5)).text_color(c(look.tertiary)).child(row.lines))
                    .child(status(row.state))
                    .child(div().font_weight(FontWeight::MEDIUM).whitespace_nowrap().child(row.title))
                    .children(row.kind.map(|kind| chip(look, kind, look.secondary)))
                    .children(row.inbox.map(|count| {
                        div().flex().items_center().gap(px(2.)).text_size(px(11.)).text_color(c(RED)).child(small_icon(IconName::Inbox, RED)).child(count)
                    })),
            )
            .child(div().w(px(100.)).child(row.agent))
            .child(div().w(px(80.)).text_color(c(if row.machine == "This Mac" { look.tertiary } else { look.text })).child(row.machine))
            .child(div().w(px(110.)).text_color(c(status_hex)).child(row.status))
            .child(div().w(px(90.)).text_color(c(look.secondary)).child(row.started))
    };
    card(look)
        .overflow_hidden()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .h(px(30.))
                .bg(c(look.panel))
                .border_b_1()
                .border_color(c(look.line))
                .child(head("Session").flex_1())
                .child(head("Agent").w(px(100.)))
                .child(head("Machine").w(px(80.)))
                .child(head("Status").w(px(110.)))
                .child(head("Started").w(px(90.))),
        )
        .children(ROWS.iter().map(row))
}

/// Clicking the channel opens its Sessions tab, the one tree
/// table of its sessions (CH-12).
pub fn sessions(look: Look) -> Div {
    frame(
        look,
        Tab::Sessions,
        div()
            .p(px(20.))
            .text_size(px(12.5))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(field(look, "Filter sessions", true).w(px(240.)))
                    .child(segmented(look, &["Open", "Archived", "All"], 0))
                    .child(div().flex_1())
                    .child(div().text_color(c(look.accent)).child("Expand all"))
                    .child(div().text_color(c(look.accent)).child("Collapse all")),
            )
            .child(sessions_table(look))
            .child(secondary(look, "Click a row to open the session. ✉ opens it with its Inbox, forks included.")),
    )
}

fn event(look: Look, glyph: IconName, hex: u32, text: &'static str, who: &'static str, when: &'static str, action: Option<&'static str>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(8.))
        .border_b_1()
        .border_color(c(look.line))
        .child(small_icon(glyph, hex))
        .child(div().flex_1().flex().flex_col().child(text).child(secondary(look, who)))
        .children(action.map(|action| button(look, Kind::Secondary, action)))
        .child(div().w(px(46.)).flex().justify_end().child(secondary(look, when)))
}

fn activity_list(look: Look) -> Div {
    card(look)
        .overflow_hidden()
        .child(event(look, IconName::Inbox, ORANGE, "Asks to move PAY-3 to Done", "Claude Code · Ledger schema migration", "2m", Some("Review")))
        .child(event(look, IconName::GitFork, look.accent, "Forked “Try BRIN index” → “Partial BRIN on posted_at” (code only)", "You", "14m", None))
        .child(event(look, IconName::Check, GREEN, "Finished a turn: +184 −24 across 2 repos", "Claude Code · Ledger schema migration", "20m", Some("Diff")))
        .child(event(look, IconName::FileText, look.secondary, "Created PAY-9 “Backfill progress bar”", "Claude Code · Ledger schema migration", "22m", None))
        .child(event(look, IconName::WifiOff, GREY, "gpu-box went offline", "Humata", "41m", None))
        .child(event(look, IconName::GitPullRequest, PURPLE, "Pushed rate-limiter and opened PR #491", "Codex · API rate limiter", "1h", Some("Open PR")))
}

pub fn activity(look: Look) -> Div {
    frame(look, Tab::Activity, div().p(px(20.)).text_size(px(12.5)).flex().flex_col().gap(px(10.)).child(activity_list(look)))
}

/// Repos and machines: clones per repo per machine (CH-9).
pub fn repos(look: Look) -> Div {
    let cell = |text: &'static str, faint: bool| {
        div().w(px(150.)).text_color(c(if faint { look.tertiary } else { look.accent })).child(text)
    };
    let head = |name: &'static str, note: &'static str, hex: u32| {
        div().w(px(150.)).flex().flex_col().child(div().font_weight(FontWeight::SEMIBOLD).text_color(c(look.accent)).child(name)).child(div().text_size(px(11.)).text_color(c(hex)).child(note))
    };
    let row = |repo: &'static str, cells: [(&'static str, bool); 3]| {
        div()
            .flex()
            .items_center()
            .px(px(14.))
            .h(px(38.))
            .border_b_1()
            .border_color(c(look.line))
            .child(div().flex_1().flex().items_center().gap(px(7.)).child(small_icon(IconName::FolderGit2, look.secondary)).child(repo))
            .children(cells.map(|(text, faint)| cell(text, faint)))
    };
    frame(
        look,
        Tab::Repos,
        div().p(px(20.)).text_size(px(12.5)).flex().flex_col().gap(px(12.)).child(
            card(look)
                .overflow_hidden()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(14.))
                        .h(px(48.))
                        .bg(c(look.panel))
                        .border_b_1()
                        .border_color(c(look.line))
                        .child(div().flex_1())
                        .child(head("This Mac", "3 sessions", look.secondary))
                        .child(head("devbox-1", "4 sessions", look.secondary))
                        .child(head("gpu-box", "1 session · offline", GREY)),
                )
                .child(row("acme/ledger", [("3 clones", false), ("4 clones", false), ("1 clone", false)]))
                .child(row("acme/api", [("2 clones", false), ("3 clones", false), ("—", true)])),
        )
        .child(div().flex().gap(px(8.)).child(button(look, Kind::Secondary, "Add a GitHub repo…")).child(button(look, Kind::Secondary, "Add a machine…"))),
    )
}
