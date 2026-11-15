//! The other screens: Home, Inbox, a channel, Board, Hosts, Settings and
//! first run.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::session::decision_card;
use crate::sidebar::{self, Side};

pub fn toolbar(look: Look, title: &'static str, note: Option<&'static str>, actions: Vec<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(52.))
        .px(px(20.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child(title))
        .children(note.map(|note| secondary(look, note)))
        .child(div().flex_1())
        .children(actions)
}

/// A page: the sidebar, then a toolbar over the body.
pub fn page(look: Look, side: Side, bar: Div, body: Div) -> Div {
    div()
        .size_full()
        .flex()
        .child(sidebar::sidebar(look, side))
        .child(handle(look, false))
        .child(div().flex_1().min_w_0().flex().flex_col().bg(c(look.content)).child(bar).child(body.flex_1().min_h_0().overflow_hidden()))
}

pub fn session_row(look: Look, state: St, title: &'static str, channel: &'static str, what: &'static str, when: &'static str, action: Option<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .border_b_1()
        .border_color(c(look.line))
        .child(status(state))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().flex().gap(px(8.)).child(div().font_weight(FontWeight::MEDIUM).child(title)).child(secondary(look, channel)))
                .child(div().text_size(px(12.)).text_color(c(if state == St::NeedsYou { ORANGE } else { look.secondary })).child(what)),
        )
        .child(secondary(look, when))
        .children(action)
}

pub fn group(look: Look, title: &'static str, rows: Vec<Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(label(look, title))
        .child(card(look).overflow_hidden().children(rows))
}

/// Home: one column of grouped lists, what needs you at the top.
pub fn home_list(look: Look) -> Div {
    page(
        look,
        Side { pinned: true, ..Side::at("Home") },
        toolbar(look, "Home", None, vec![button(look, Kind::Primary, "New session")]),
        div().flex().justify_center().p(px(24.)).text_size(px(13.)).child(
            div()
                .w(px(720.))
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(group(
                    look,
                    "Needs you · 2",
                    vec![
                        session_row(look, St::NeedsYou, "Ledger schema migration", "# payments-rewrite", "Asks to move PAY-3 to Done", "2m", Some(button(look, Kind::Primary, "Review"))),
                        session_row(look, St::Exited, "Upgrade Postgres 17", "# infra", "Exited with code 1 after the dump step", "40m", Some(button(look, Kind::Secondary, "Resume"))),
                    ],
                ))
                .child(group(
                    look,
                    "Working · 3",
                    vec![
                        session_row(look, St::Working, "Refund retries", "# payments-rewrite", "Running cargo test -p refunds", "now", None),
                        session_row(look, St::Working, "Try BRIN index", "fork of Ledger schema", "Writing migrations/0043_brin.sql", "now", None),
                        session_row(look, St::Working, "Benchmark BRIN", "# infra · gpu-box", "pgbench run 2 of 4", "1m", None),
                    ],
                ))
                .child(group(
                    look,
                    "Recent",
                    vec![
                        session_row(look, St::Idle, "API rate limiter", "# payments-rewrite", "Finished: pushed and opened PR #491", "1h", Some(button(look, Kind::Secondary, "Open PR"))),
                        session_row(look, St::Idle, "acme/api #482", "# pr-reviews", "3 new commits since your review", "2h", Some(button(look, Kind::Secondary, "Review"))),
                    ],
                )),
        ),
    )
}

pub fn inbox_row(look: Look, title: &'static str, from: &'static str, when: &'static str, on: bool, vip: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(14.))
        .py(px(9.))
        .border_b_1()
        .border_color(c(look.line))
        .when(on, |row| row.bg(c(look.selection)).text_color(c(look.selection_text)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .when(vip, |row| row.child(div().px(px(4.)).rounded(px(3.)).bg(c(PURPLE)).text_size(px(9.5)).font_weight(FontWeight::BOLD).text_color(c(0xffffff)).child("VIP")))
                .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_size(px(11.)).text_color(c(if on { look.selection_text } else { look.secondary })).child(when)),
        )
        .child(div().text_size(px(11.5)).text_color(c(if on { look.selection_text } else { look.secondary })).child(from))
}

/// Inbox: the list, newest needing you first, beside the open request.
pub fn inbox_split(look: Look) -> Div {
    page(
        look,
        Side::at("Inbox"),
        toolbar(look, "Inbox", Some("7 waiting"), vec![segmented(look, &["Waiting", "Answered", "All"], 0)]),
        div()
            .flex()
            .text_size(px(12.5))
            .child(
                div()
                    .w(px(330.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(c(look.line))
                    .child(div().px(px(14.)).pt(px(10.)).pb(px(4.)).child(label(look, "# payments-rewrite")))
                    .child(inbox_row(look, "Move PAY-3 to Done", "Claude Code · Ledger schema migration", "2m", true, true))
                    .child(inbox_row(look, "Push try-brin to origin", "Claude Code · Try BRIN index", "9m", false, false))
                    .child(inbox_row(look, "Which currency for refunds?", "Copilot · Refund retries", "15m", false, false))
                    .child(div().px(px(14.)).pt(px(10.)).pb(px(4.)).child(label(look, "# infra")))
                    .child(inbox_row(look, "Run a command on gpu-box", "Codex · Benchmark BRIN", "22m", false, false))
                    .child(inbox_row(look, "Delete the old snapshots", "Claude Code · Upgrade Postgres 17", "1h", false, false)),
            )
            .child(div().flex_1().p(px(20.)).flex().flex_col().gap(px(12.)).child(decision_card(look, true).max_w(px(560.)))),
    )
}

/// Hosts: every machine, its health and what runs on it.
pub fn hosts(look: Look) -> Div {
    let meter = |used: f32, hex: u32| {
        div().w(px(70.)).h(px(5.)).rounded_full().bg(c(look.fill)).child(div().w(px(70. * used)).h_full().rounded_full().bg(c(hex)))
    };
    let row = |state: St, name: &'static str, os: &'static str, sessions: &'static str, disk: f32, disk_text: &'static str, action: Option<Div>, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .h(px(44.))
            .border_b_1()
            .border_color(c(look.line))
            .when(on, |row| row.bg(c(look.fill)))
            .child(status(state))
            .child(div().w(px(150.)).flex().flex_col().child(div().font_weight(FontWeight::MEDIUM).child(name)).child(secondary(look, os)))
            .child(div().w(px(110.)).child(sessions))
            .child(div().w(px(150.)).flex().items_center().gap(px(8.)).child(meter(disk, if disk > 0.85 { RED } else { look.accent })).child(secondary(look, disk_text)))
            .child(div().flex_1())
            .children(action)
    };
    page(
        look,
        Side::at("devbox-1"),
        toolbar(look, "Hosts", None, vec![button(look, Kind::Secondary, "Add a machine…")]),
        div().flex().gap(px(16.)).p(px(20.)).text_size(px(12.5)).child(
            card(look)
                .flex_1()
                .overflow_hidden()
                .child(row(St::Working, "This Mac", "macOS 26.7 · arm64", "2 sessions", 0.42, "210 GB free", None, false))
                .child(row(St::Working, "devbox-1", "Ubuntu 24.04 · ssh", "4 sessions", 0.88, "11 GB free", Some(button(look, Kind::Secondary, "Free space…")), true))
                .child(row(St::Offline, "gpu-box", "Debian 13 · ssh", "1 session", 0.3, "last seen 14:02", Some(button(look, Kind::Secondary, "Reconnect")), false))
                .child(row(St::Idle, "cloud-vm-3", "Ubuntu 24.04 · provider", "none", 0.1, "stopped", Some(button(look, Kind::Secondary, "Start")), false)),
        )
        .child(
            div()
                .w(px(300.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    card(look)
                        .p(px(12.))
                        .gap(px(6.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("devbox-1"))
                        .child(secondary(look, "humatad 2.0.3 · up 6 days · linked over ssh"))
                        .child(div().flex().gap(px(6.)).child(chip(look, "gh 2.83", look.secondary)).child(chip(look, "claude 2.4", look.secondary)).child(chip(look, "codex 0.98", look.secondary)))
                        .child(div().text_color(c(ORANGE)).child("11 GB free: new clones stop at 5 GB."))
                        .child(div().flex().gap(px(8.)).child(button(look, Kind::Secondary, "Free space…")).child(button(look, Kind::Secondary, "Open a shell"))),
                )
                .child(
                    card(look)
                        .p(px(12.))
                        .gap(px(6.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child("Bridge"))
                        .child(secondary(look, "This Mac ⇄ devbox-1 (ssh, 18 ms)"))
                        .child(secondary(look, "This Mac ⇢ gpu-box (offline since 14:02; retrying)")),
                ),
        ),
    )
}

/// Settings, showing the Layout and appearance page.
pub fn settings(look: Look) -> Div {
    let nav = |glyph: IconName, text: &'static str, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .px(px(10.))
            .rounded(px(6.))
            .when(on, |row| row.bg(c(look.selection)).text_color(c(look.selection_text)))
            .child(icon(glyph, if on { look.selection_text } else { look.accent }))
            .child(text)
    };
    let setting = |title: &'static str, note: &'static str, control: Div| {
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(14.))
            .py(px(10.))
            .border_b_1()
            .border_color(c(look.line))
            .child(div().flex_1().flex().flex_col().child(title).child(secondary(look, note)))
            .child(control)
    };
    page(
        look,
        Side::at("Settings"),
        toolbar(look, "Settings", None, vec![]),
        div()
            .flex()
            .text_size(px(12.5))
            .child(
                div()
                    .w(px(200.))
                    .h_full()
                    .p(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .border_r_1()
                    .border_color(c(look.line))
                    .child(nav(IconName::Settings, "General", false))
                    .child(nav(IconName::Bot, "Agents", false))
                    .child(nav(IconName::Server, "Machines", false))
                    .child(nav(IconName::Columns2, "Layout", true))
                    .child(nav(IconName::ShieldCheck, "Agent access", false))
                    .child(nav(IconName::HardDrive, "Storage", false))
                    .child(nav(IconName::Bell, "Notifications", false))
                    .child(nav(IconName::Keyboard, "Keyboard", false)),
            )
            .child(
                div()
                    .flex_1()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        card(look)
                            .child(setting("Appearance", "Follows the system unless you pick one.", segmented(look, &["System", "Light", "Dark"], 0)))
                            .child(setting("Terminal font", "Used by every terminal and the diff.", popup(look, "Menlo 12")))
                            .child(setting("Sidebar shows Pinned", "Pinned sessions, reviews and machines at the top.", switch(look, true))),
                    )
                    .child(label(look, "Panes"))
                    .child(
                        card(look)
                            .p(px(14.))
                            .gap(px(10.))
                            .child(radio(look, false, "Fixed", "Default layouts only: sidebar, terminal, one panel."))
                            .child(radio(look, true, "Simple", "Up to 4 panes, two levels deep, moved by dragging."))
                            .child(radio(look, false, "Free", "Any number and depth.")),
                    )
                    .child(secondary(look, "Narrowing this never closes anything: layouts that no longer fit show their default until you widen it again.")),
            ),
    )
}

/// First run, with no channels yet (NV-11).
pub fn first_run(look: Look) -> Div {
    div().size_full().flex().child(
        div()
            .w(px(236.))
            .h_full()
            .bg(c(look.sidebar))
            .flex()
            .flex_col()
            .child(div().h(px(52.)).pl(px(20.)).flex().items_center().child(traffic_lights())),
    )
    .child(handle(look, false))
    .child(
        div().flex_1().flex().items_center().justify_center().text_size(px(13.)).child(
            div()
                .w(px(460.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(12.))
                .child(icon(IconName::SquareTerminal, look.accent).size(px(40.)))
                .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).child("Start with a channel"))
                .child(
                    div()
                        .text_color(c(look.secondary))
                        .text_center()
                        .child("A channel is a project: its repos, the machines that work on it, its board and its sessions."),
                )
                .child(div().flex().gap(px(10.)).pt(px(6.)).child(button(look, Kind::Primary, "Create your first channel")).child(button(look, Kind::Secondary, "Add a machine over SSH")))
                .child(div().pt(px(8.)).child(secondary(look, "GitHub access comes from the gh CLI. There is no sign-in in Humata."))),
        ),
    )
}
