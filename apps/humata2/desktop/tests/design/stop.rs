//! Stop all sessions on a machine (frames 72 to 76; D260, D264; drafts,
//! milestone STOP): the Stop confirm over the machine page, the page and
//! the Hosts list with devbox-1's sessions stopped, the Start confirm, and
//! a stop that hasn't reached the machine yet. "Sessions stopped" and
//! "Start sessions" never read like a provider VM's "stopped" and "Start".

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::machine_page;
use crate::overlays::scrim;
use crate::pages::{page, toolbar};
use crate::sidebar::{self, Side};

/// What devbox-1's machine page shows.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Machine {
    /// You stopped its sessions.
    Stopped,
    /// You asked to stop them, but devbox-1 can't be reached.
    Unreachable,
}

/// A dialog's card, `width` wide (as frame 71's).
fn dialog(look: Look, width: f32) -> Div {
    div()
        .w(px(width))
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(20.))
        .rounded(px(12.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.line))
        .shadow_lg()
        .text_size(px(12.5))
}

/// A dialog's title.
fn title(text: &'static str) -> Div {
    div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(text)
}

/// A button that destroys: red text and border (as frame 71's).
fn danger(look: Look, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .whitespace_nowrap()
        .border_1()
        .border_color(c(RED))
        .text_color(c(RED))
        .bg(c(look.fill))
        .child(text)
}

/// A dialog's buttons, at its bottom right.
fn buttons(cancel: Div, go: Div) -> Div {
    div().flex().items_center().gap(px(8.)).pt(px(8.)).child(div().flex_1()).child(cancel).child(go)
}

/// `under` with `dialog` laid over it, `top` from the window's top.
fn over(look: Look, under: Div, top: f32, dialog: Div) -> Div {
    under.relative().child(scrim(look)).child(div().absolute().top(px(top)).left_0().w_full().flex().justify_center().child(dialog))
}

/// The machine page's header in `machine`'s state: frame 59's, with the
/// stopped square or the offline dot, and the session buttons off (neither
/// works on a stopped or unreachable machine, D264).
fn header(look: Look, machine: Machine) -> Div {
    let tab = |text: &'static str, lit: bool| {
        div()
            .flex()
            .items_center()
            .h_full()
            .px(px(2.))
            .border_b_2()
            .border_color(c(if lit { look.accent } else { look.content }))
            .text_color(c(if lit { look.text } else { look.secondary }))
            .when(lit, |tab| tab.font_weight(FontWeight::MEDIUM))
            .child(text)
    };
    let stopped = machine == Machine::Stopped;
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
                .gap(px(8.))
                .h(px(48.))
                .px(px(20.))
                .child(div().text_size(px(15.)).text_color(c(look.secondary)).child("Hosts"))
                .child(small_icon(IconName::ChevronRight, look.tertiary))
                .child(if stopped { sidebar::stopped() } else { status(St::Offline) })
                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child("devbox-1"))
                .child(div().flex_1())
                .when(stopped, |bar| bar.child(div().text_size(px(11.5)).text_color(c(look.tertiary)).child("Start sessions first")))
                .child(button(look, Kind::Secondary, "Open a shell").opacity(0.5))
                .child(button(look, Kind::Primary, "New session on devbox-1").opacity(0.5))
                .child(icon(IconName::Ellipsis, look.secondary)),
        )
        .child(
            div()
                .px(px(20.))
                .pb(px(8.))
                .child(secondary(look, "Ubuntu 24.04 · x86_64 · devbox-1.lan (ssh) · humatad 2.0.3")),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap(px(20.))
                .h(px(31.))
                .px(px(20.))
                .text_size(px(12.5))
                .child(tab("Overview", true))
                .child(tab("Storage", false)),
        )
}

/// The banner at the top of the machine page's body.
fn banner(look: Look, machine: Machine) -> Div {
    let (glyph, hex, bg, text, actions) = match machine {
        Machine::Stopped => (
            IconName::Info,
            look.secondary,
            look.fill,
            "Sessions stopped by you at 14:02: agents and shells ended, everything else is kept, and they stay stopped after restarts.",
            vec![button(look, Kind::Primary, "Start sessions")],
        ),
        // Reconnect now is HO-4's, for any offline machine; here it also
        // delivers the stop.
        Machine::Unreachable => (
            IconName::WifiOff,
            ORANGE,
            look.attention_bg,
            "Stop not delivered yet: devbox-1 can't be reached, so its sessions may still be running. Humata stops them as soon as it connects again.",
            vec![button(look, Kind::Secondary, "Reconnect now"), button(look, Kind::Secondary, "Cancel stop")],
        ),
    };
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .bg(c(bg))
        .child(div().flex_shrink_0().child(small_icon(glyph, hex)))
        .child(div().flex_1().min_w_0().child(text))
        .children(actions)
}

/// The Overview body's left column in `machine`'s state: frame 59's
/// sessions, grey and "Stopped" when stopped, at their last known states
/// when unreachable; then the Humata folders.
fn left(look: Look, machine: Machine) -> Div {
    let stopped = machine == Machine::Stopped;
    let session = |state: St, name: &'static str, channel: &'static str, agent: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .h(px(36.))
            .border_b_1()
            .border_color(c(look.line))
            .child(if stopped { sidebar::stopped() } else { status(state) })
            .child(
                div()
                    .w(px(190.))
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .font_weight(FontWeight::MEDIUM)
                    .when(stopped, |name| name.text_color(c(look.secondary)))
                    .child(name),
            )
            .child(div().w(px(130.)).whitespace_nowrap().text_size(px(11.5)).text_color(c(look.secondary)).child(channel))
            .child(div().flex_1().whitespace_nowrap().text_color(c(look.secondary)).child(agent))
            .when(stopped, |row| row.child(div().flex_shrink_0().whitespace_nowrap().text_color(c(look.tertiary)).child("Stopped")))
    };
    let folder = |channel: &'static str, what: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .h(px(36.))
            .border_b_1()
            .border_color(c(look.line))
            .child(small_icon(IconName::FolderOpen, look.secondary))
            .child(div().w(px(220.)).whitespace_nowrap().child(channel))
            .child(div().flex_1().whitespace_nowrap().child(secondary(look, what)))
            .child(div().flex_shrink_0().whitespace_nowrap().text_color(c(look.accent)).child("Open in Files"))
    };
    let heading = if stopped { "Sessions on it · 6" } else { "Sessions on it · 6 · as last seen at 14:02" };
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(label(look, heading))
                .child(
                    card(look)
                        .overflow_hidden()
                        .child(session(St::NeedsYou, "Ledger schema migration", "# payments-rewrite", "Claude Code"))
                        .child(session(St::NeedsYou, "Try BRIN index", "# payments-rewrite", "Claude Code"))
                        .child(session(St::Idle, "API rate limiter", "# payments-rewrite", "Codex"))
                        .child(session(St::Working, "Partial BRIN on posted_at", "# payments-rewrite", "Claude Code"))
                        .child(session(St::Working, "Benchmark BRIN", "# infra", "Codex"))
                        .child(session(St::Exited, "Upgrade Postgres 17", "# infra", "Claude Code")),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(label(look, "Humata folders by channel"))
                .child(
                    card(look)
                        .overflow_hidden()
                        .child(folder("# payments-rewrite", "4 sessions · 3.1 GB"))
                        .child(folder("# infra", "2 sessions · 0.8 GB"))
                        .child(folder("# pr-reviews", "no sessions · 0.2 GB")),
                ),
        )
}

/// devbox-1's machine page, Overview tab, in `machine`'s state.
fn devbox_page(look: Look, machine: Machine) -> Div {
    let side = match machine {
        Machine::Stopped => Side { stopped: true, ..Side::at("devbox-1") },
        Machine::Unreachable => Side { unreachable: true, ..Side::at("devbox-1") },
    };
    page(
        look,
        side,
        header(look, machine),
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(20.))
            .text_size(px(12.5))
            .child(banner(look, machine))
            .child(div().flex().gap(px(16.)).child(left(look, machine)).child(machine_page::right(look, &machine_page::Machine::devbox_1()))),
    )
}

/// Frame 72: Stop all sessions… on devbox-1, confirmed over its page (D260).
pub fn stop_confirm(look: Look) -> Div {
    let part = |head: &'static str, text: &'static str| {
        div().flex().flex_col().gap(px(2.)).pt(px(4.)).child(label(look, head)).child(div().child(text))
    };
    over(
        look,
        machine_page::machine(look),
        120.,
        dialog(look, 520.)
            .child(title("Stop all sessions on devbox-1?"))
            .child(div().text_color(c(look.secondary)).child("Every agent and shell in devbox-1's 6 sessions gets 2 seconds to quit, then is killed."))
            .child(part("Kept", "Scrollback, conversations, todos, rewind points and open tabs."))
            .child(part("While stopped", "Agents can't type, call Humata or be answered. Their open requests end."))
            .child(
                div()
                    .mt(px(4.))
                    .p(px(10.))
                    .rounded(px(7.))
                    .bg(c(look.fill))
                    .text_color(c(look.secondary))
                    .child("devbox-1's sessions stay stopped, even after it restarts, until you choose Start sessions."),
            )
            .child(buttons(button(look, Kind::Secondary, "Cancel"), danger(look, "Stop all sessions"))),
    )
}

/// Frame 73: devbox-1's page with its sessions stopped (D260, D264).
pub fn machine_stopped(look: Look) -> Div {
    devbox_page(look, Machine::Stopped)
}

/// Frame 74: Hosts (frame 28) with devbox-1's sessions stopped; cloud-vm-3
/// keeps its provider "stopped" and "Start".
pub fn hosts_stopped(look: Look) -> Div {
    let meter = |used: f32, hex: u32| {
        div().w(px(70.)).h(px(5.)).rounded_full().bg(c(look.fill)).child(div().w(px(70. * used)).h_full().rounded_full().bg(c(hex)))
    };
    let row = |glyph: Div, name: &'static str, os: &'static str, sessions: Div, disk: f32, disk_text: &'static str, action: Option<Div>, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .h(px(44.))
            .border_b_1()
            .border_color(c(look.line))
            .when(on, |row| row.bg(c(look.fill)))
            .child(glyph)
            .child(div().w(px(150.)).flex().flex_col().child(div().font_weight(FontWeight::MEDIUM).child(name)).child(secondary(look, os)))
            .child(sessions.w(px(110.)).whitespace_nowrap())
            .child(div().w(px(150.)).flex().items_center().gap(px(8.)).child(meter(disk, if disk > 0.85 { RED } else { look.accent })).child(secondary(look, disk_text)))
            .child(div().flex_1())
            .children(action)
    };
    page(
        look,
        Side { stopped: true, ..Side::at("devbox-1") },
        toolbar(look, "Hosts", None, vec![button(look, Kind::Secondary, "Add a machine…")]),
        div().flex().gap(px(16.)).p(px(20.)).text_size(px(12.5)).child(
            card(look)
                .flex_1()
                .overflow_hidden()
                .child(row(status(St::Working), "This Mac", "macOS 26.7 · arm64", div().child("3 sessions"), 0.42, "210 GB free", None, false))
                .child(row(
                    sidebar::stopped(),
                    "devbox-1",
                    "Ubuntu 24.04 · ssh",
                    div().text_color(c(look.secondary)).child("Sessions stopped"),
                    0.88,
                    "11 GB free",
                    Some(button(look, Kind::Secondary, "Start sessions")),
                    true,
                ))
                .child(row(status(St::Offline), "gpu-box", "Debian 13 · ssh", div().child("none"), 0.3, "last seen 14:02", Some(button(look, Kind::Secondary, "Reconnect")), false))
                .child(row(status(St::Idle), "cloud-vm-3", "Ubuntu 24.04 · provider", div().child("none"), 0.1, "stopped", Some(button(look, Kind::Secondary, "Start")), false)),
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
                        .child(div().child("Sessions stopped by you at 14:02; everything is kept."))
                        .child(div().text_color(c(ORANGE)).child("11 GB free: new clones stop at 5 GB."))
                        .child(div().flex().gap(px(8.)).child(button(look, Kind::Primary, "Start sessions")).child(button(look, Kind::Secondary, "Free space…"))),
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

/// Frame 75: Start sessions on devbox-1, confirmed over frame 73 (D264).
pub fn start_confirm(look: Look) -> Div {
    over(
        look,
        devbox_page(look, Machine::Stopped),
        180.,
        dialog(look, 440.)
            .child(title("Start sessions on devbox-1?"))
            .child(div().text_color(c(look.secondary)).child("Humata brings back the 6 sessions stopped at 14:02 as after a restart: agents resume their conversations, idle, and shells open fresh in the same folders. A report shows what came back."))
            .child(buttons(button(look, Kind::Secondary, "Cancel"), button(look, Kind::Primary, "Start sessions"))),
    )
}

/// Frame 76: the stop hasn't reached devbox-1 yet; its sessions show their
/// last known states (D264).
pub fn stop_not_delivered(look: Look) -> Div {
    devbox_page(look, Machine::Unreachable)
}
