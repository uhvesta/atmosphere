//! A machine's page, opened by clicking the machine in the sidebar's Hosts
//! section (HO-3 to HO-5): the Overview tab with its sessions, Humata folders
//! (FM-11), health and what its agents can reach, and the Storage tab
//! (LD-13 to LD-15).

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::page;
use crate::sidebar::Side;

/// The machine's header and its sub-tabs; `storage` lights Storage, else Overview.
fn header(look: Look, storage: bool) -> Div {
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
                .child(status(St::Working))
                .child(div().font_weight(FontWeight::SEMIBOLD).text_size(px(15.)).child("devbox-1"))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Open a shell"))
                .child(button(look, Kind::Primary, "New session on devbox-1"))
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
                .h(px(32.))
                .px(px(20.))
                .text_size(px(12.5))
                .child(tab("Overview", !storage))
                .child(tab("Storage", storage)),
        )
}

/// A small card title.
fn title(text: &'static str) -> Div {
    div().font_weight(FontWeight::SEMIBOLD).child(text)
}

/// The Overview body's left column: sessions on the machine, then its
/// Humata folders by channel (FM-11).
fn left(look: Look) -> Div {
    let session = |state: St, name: &'static str, channel: &'static str, agent: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .h(px(36.))
            .border_b_1()
            .border_color(c(look.line))
            .child(status(state))
            .child(div().w(px(190.)).whitespace_nowrap().overflow_hidden().font_weight(FontWeight::MEDIUM).child(name))
            .child(div().w(px(130.)).whitespace_nowrap().text_size(px(11.5)).text_color(c(look.secondary)).child(channel))
            .child(div().flex_1().whitespace_nowrap().text_color(c(look.secondary)).child(agent))
    };
    let folder = |path: &'static str, what: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .h(px(36.))
            .border_b_1()
            .border_color(c(look.line))
            .child(small_icon(IconName::FolderOpen, look.secondary))
            .child(div().w(px(220.)).whitespace_nowrap().font_family(MONO).text_size(px(11.5)).child(path))
            .child(div().flex_1().whitespace_nowrap().child(secondary(look, what)))
            .child(div().flex_shrink_0().whitespace_nowrap().text_color(c(look.accent)).child("Open in Files"))
    };
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
                .child(label(look, "Sessions on it · 6"))
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
                        .child(folder("~/Humata/payments-rewrite/", "4 sessions · 3.1 GB"))
                        .child(folder("~/Humata/infra/", "2 sessions · 0.8 GB"))
                        .child(folder("~/Humata/pr-reviews/", "no sessions · 0.2 GB")),
                ),
        )
}

/// The Overview body's right column: the health card, then the host rules
/// its agents run under.
fn right(look: Look) -> Div {
    let kv = |key: &'static str, value: Div| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_h(px(22.))
            .child(div().w(px(84.)).flex_shrink_0().text_color(c(look.secondary)).child(key))
            .child(value.flex_1().min_w_0())
    };
    let rule = |text: &'static str, value: &'static str, hex: u32| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .h(px(32.))
            .border_b_1()
            .border_color(c(look.line))
            .child(div().flex_1().min_w_0().whitespace_nowrap().overflow_hidden().child(text))
            .child(div().w(px(46.)).flex_shrink_0().whitespace_nowrap().font_weight(FontWeight::MEDIUM).text_color(c(hex)).child(value))
    };
    div()
        .w(px(400.))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            card(look)
                .p(px(12.))
                .gap(px(8.))
                .child(title("Health"))
                .child(kv(
                    "GitHub",
                    div().flex().items_center().gap(px(6.)).child(small_icon(IconName::Check, GREEN)).child("signed in as uhvesta").child(chip(look, "repo", look.secondary)).child(chip(look, "read:org", look.secondary)),
                ))
                .child(kv("Agents", div().flex().gap(px(6.)).child(chip(look, "claude 2.4", look.secondary)).child(chip(look, "codex 0.98", look.secondary))))
                .child(kv(
                    "Disk",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().h(px(5.)).w_full().rounded_full().bg(c(look.fill)).child(div().w(px(250. * 0.88)).h_full().rounded_full().bg(c(RED))))
                        .child(div().whitespace_nowrap().child("11 GB free of 500 GB"))
                        .child(div().whitespace_nowrap().text_size(px(11.5)).text_color(c(ORANGE)).child("New clones stop at 5 GB.")),
                ))
                .child(kv(
                    "Variables",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(div().flex().gap(px(6.)).child(chip(look, "DATABASE_URL", look.secondary)).child(chip(look, "STRIPE_KEY", look.secondary)))
                        .child(secondary(look, "Names only; values stay on the host.")),
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(label(look, "What its agents can reach through the host"))
                .child(
                    card(look)
                        .overflow_hidden()
                        .child(rule("Read files on other machines", "Allow", GREEN))
                        .child(rule("Push its branch or open a pull request", "Ask me", ORANGE))
                        .child(rule("Read the global board or other channels", "Allow", GREEN))
                        .child(rule("Change the global board or other channels", "Ask me", ORANGE))
                        .child(div().px(px(12.)).h(px(30.)).flex().items_center().text_color(c(look.accent)).child("Change in Agent access")),
                ),
        )
}

/// The machine page for devbox-1, Overview tab: sessions on it, its Humata
/// folders, health, and what its agents can reach (HO-4, FM-11).
pub fn machine(look: Look) -> Div {
    page(
        look,
        Side::at("devbox-1"),
        header(look, false),
        div().flex().gap(px(16.)).p(px(20.)).text_size(px(12.5)).child(left(look)).child(right(look)),
    )
}

/// One row of the Storage table: the kind (blank under a group), what it
/// is, an optional note, its size, last use and the one action.
fn storage_row(look: Look, kind: &'static str, what: &'static str, note: Option<&'static str>, size: &'static str, used: &'static str, action: Option<&'static str>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(12.))
        .min_h(px(38.))
        .py(px(4.))
        .border_b_1()
        .border_color(c(look.line))
        .child(div().w(px(120.)).flex_shrink_0().whitespace_nowrap().text_color(c(look.secondary)).child(kind))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().whitespace_nowrap().overflow_hidden().child(what))
                .children(note.map(|note| div().whitespace_nowrap().text_size(px(11.)).text_color(c(ORANGE)).child(note))),
        )
        .child(div().w(px(70.)).flex_shrink_0().whitespace_nowrap().child(size))
        .child(div().w(px(60.)).flex_shrink_0().whitespace_nowrap().text_color(c(look.secondary)).child(used))
        .child(
            div()
                .w(px(80.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_color(c(if action.is_some() { look.accent } else { look.tertiary }))
                .child(action.unwrap_or("—")),
        )
}

/// The machine page's Storage tab: what Humata keeps on devbox-1, with
/// sizes, last use and the one action that frees each (LD-13 to LD-15).
pub fn storage(look: Look) -> Div {
    let head = |text: &'static str| div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(look.secondary)).child(text);
    page(
        look,
        Side::at("devbox-1"),
        header(look, true),
        div()
            .p(px(20.))
            .text_size(px(12.5))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("212 GB free of 500 GB"))
                    .child(secondary(look, "measured just now"))
                    .child(div().flex_1())
                    .child(button(look, Kind::Secondary, "Measure again")),
            )
            .child(
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
                            .child(head("Kind").w(px(120.)).flex_shrink_0())
                            .child(head("What").flex_1())
                            .child(head("Size").w(px(70.)).flex_shrink_0())
                            .child(head("Used").w(px(60.)).flex_shrink_0())
                            .child(head("Action").w(px(80.)).flex_shrink_0()),
                    )
                    .child(storage_row(look, "Session clones", "# payments-rewrite › API rate limiter", None, "1.2 GB", "2h", Some("Archive…")))
                    .child(storage_row(look, "", "# payments-rewrite › Refund retries", None, "0.9 GB", "9d", Some("Archive…")))
                    .child(storage_row(look, "", "# payments-rewrite › Ledger schema migration", None, "2.1 GB", "now", Some("Archive…")))
                    .child(storage_row(look, "Main clones", "# payments-rewrite: api, ledger", Some("in use by # payments-rewrite"), "0.6 GB", "—", None))
                    .child(storage_row(look, "Build caches", "API rate limiter: target/", None, "14 GB", "2h", Some("Clear")))
                    .child(storage_row(look, "", "Ledger schema migration: target/", Some("running: asks first, its next build is slower"), "6.0 GB", "now", Some("Clear")))
                    .child(storage_row(look, "Trash", "41 items · oldest 12 days", None, "3.1 GB", "—", Some("Empty…")))
                    .child(storage_row(look, "Logs", "humatad, holders", None, "0.4 GB", "—", Some("Trim…"))),
            )
            .child(secondary(look, "Sizes are measured when this page opens. Archive… runs the same checks as archiving from a session.")),
    )
}
