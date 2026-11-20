//! MASSDELETE (D875): multi-select mode in the sidebar, Home and machine page,
//! the delete confirmation sheet (picked, delete-all, stale re-plan, dark),
//! and the post-delete toast. Drawn from the signed-off mockups in
//! `workflow/milestones/mockups/massdelete/` (`m1-1`..`m1-4`, `m2-1`, `m2-2`,
//! `m2-5`, `m2-6`).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::overlays::scrim;
use crate::pages;
use crate::sidebar::{self, Side};

const RED_FILL: u32 = 0xe5392d;
const KEY_HINT: &str =
    "\u{2318}A all \u{b7} \u{21e7}-click a range \u{b7} space ticks \u{b7} esc cancels";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tick {
    Off,
    On,
    Along,
    Dash,
    Barred,
}

fn tick_box(look: Look, size: f32, tick: Tick) -> Div {
    let base = div()
        .size(px(size))
        .flex_shrink_0()
        .rounded(px(if size > 13.5 { 4. } else { 3.5 }))
        .flex()
        .items_center()
        .justify_center();
    match tick {
        Tick::Off => base
            .border_1()
            .border_color(c(look.tertiary))
            .bg(c(look.card)),
        Tick::On => base
            .bg(c(look.accent))
            .child(Icon::new(IconName::Check).size(px(size - 4.)).text_color(c(0xffffff))),
        Tick::Along => base
            .bg(c(look.accent))
            .opacity(0.5)
            .child(Icon::new(IconName::Check).size(px(size - 4.)).text_color(c(0xffffff))),
        Tick::Dash => base.bg(c(look.accent)).child(
            div()
                .w(px(size - 6.))
                .h(px(1.5))
                .rounded_full()
                .bg(c(0xffffff)),
        ),
        Tick::Barred => base
            .border_1()
            .border_dashed()
            .border_color(c(look.tertiary))
            .opacity(0.6),
    }
}

fn tint(look: Look) -> Rgba {
    let accent = c(look.accent);
    Rgba {
        a: if look.dark { 0.22 } else { 0.16 },
        ..accent
    }
}

fn red_outline_button(look: Look, label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(24.))
        .px(px(10.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(RED))
        .bg(c(look.card))
        .text_color(c(RED))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .whitespace_nowrap()
        .child(label)
}

fn red_filled_button(label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.))
        .px(px(12.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(RED_FILL))
        .bg(c(RED_FILL))
        .text_color(c(0xffffff))
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .whitespace_nowrap()
        .child(label)
}

fn disabled_button(look: Look, label: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .h(px(26.))
        .px(px(12.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(look.line))
        .bg(c(look.fill))
        .text_color(c(look.tertiary))
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .whitespace_nowrap()
        .child(label)
}

fn quiet(look: Look, text: &'static str) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(c(look.secondary))
        .child(text)
}

fn lined(glyph: IconName, color: u32, size: f32, text: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(7.))
        .child(
            div()
                .mt(px(2.))
                .flex_shrink_0()
                .child(Icon::new(glyph).size(px(size)).text_color(c(color))),
        )
        .child(div().flex_1().min_w_0().text_color(c(color)).child(text))
}

fn work_check(look: Look, on: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .py(px(2.))
        .child(tick_box(look, 16., if on { Tick::On } else { Tick::Off }))
        .child(div().text_size(px(13.)).child("Delete that work too"))
}

fn changed_tag() -> Div {
    div()
        .flex_shrink_0()
        .px(px(6.))
        .rounded(px(4.))
        .border_1()
        .border_color(c(ORANGE))
        .text_size(px(11.))
        .text_color(c(ORANGE))
        .child("changed")
}

// ---- Sidebar in select mode (m1-1)

fn select_sidebar(look: Look) -> Div {
    let nav = |glyph: IconName, text: &'static str, count: Option<&'static str>, on: bool| {
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .h(px(28.))
            .mx(px(8.))
            .px(px(8.))
            .rounded(px(6.))
            .when(on, |r| r.bg(c(look.selection)).text_color(c(look.selection_text)))
            .child(icon(glyph, if on { look.selection_text } else { look.accent }))
            .child(div().flex_1().child(text))
            .children(count.map(|n| {
                div()
                    .text_size(px(11.5))
                    .text_color(c(if on { look.selection_text } else { look.secondary }))
                    .child(n)
            }))
    };
    let channel = |tick: Tick, name: &'static str, right: Option<&'static str>| {
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .h(px(26.))
            .mx(px(8.))
            .px(px(6.))
            .child(tick_box(look, 13., tick))
            .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).child(name))
            .children(right.map(|r| secondary(look, r)))
    };
    let sess = |tick: Tick, indent: f32, fork: bool, st: St, title: &'static str, bold: bool| {
        let tinted = matches!(tick, Tick::On | Tick::Along);
        div()
            .flex()
            .items_center()
            .gap(px(6.))
            .h(px(26.))
            .mx(px(8.))
            .px(px(6.))
            .rounded(px(6.))
            .when(tinted, |r| r.bg(tint(look)))
            .child(tick_box(look, 13., tick))
            .when(indent > 0., |r| r.child(div().w(px(indent))))
            .when(fork, |r| r.child(small_icon(IconName::GitFork, look.tertiary)))
            .child(status(st))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .when(bold, |t| t.font_weight(FontWeight::SEMIBOLD))
                    .child(title),
            )
    };
    let host = |tick: Tick, st: St, name: &'static str, note: Option<&'static str>| {
        div()
            .flex()
            .items_center()
            .gap(px(7.))
            .h(px(25.))
            .mx(px(8.))
            .px(px(6.))
            .child(tick_box(look, 13., tick))
            .child(status(st))
            .child(div().flex_1().child(name))
            .children(note.map(|n| secondary(look, n)))
    };

    div()
        .w(px(250.))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .bg(c(look.sidebar))
        .text_size(px(12.5))
        .child(
            div()
                .flex()
                .items_center()
                .h(px(38.))
                .px(px(14.))
                .gap(px(10.))
                .child(traffic_lights())
                .child(div().flex_1())
                .child(icon(IconName::PanelLeft, look.secondary))
                .child(icon(IconName::SquarePen, look.secondary)),
        )
        .child(
            div()
                .mx(px(10.))
                .mb(px(8.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(6.))
                .bg(c(look.fill))
                .flex()
                .items_center()
                .gap(px(6.))
                .child(small_icon(IconName::Search, look.tertiary))
                .child(div().flex_1().text_size(px(12.)).text_color(c(look.tertiary)).child("Search"))
                .child(div().text_size(px(11.)).text_color(c(look.tertiary)).child("\u{2318}K")),
        )
        .child(nav(IconName::House, "Home", None, true))
        .child(nav(IconName::Inbox, "Agent inbox", Some("7"), false))
        .child(nav(IconName::SquareKanban, "To-do", None, false))
        .child(nav(IconName::SquareTerminal, "Desk", None, false))
        .child(
            div()
                .flex()
                .items_center()
                .px(px(14.))
                .pt(px(12.))
                .pb(px(4.))
                .child(label(look, "Channels"))
                .child(div().flex_1())
                .child(small_icon(IconName::Plus, look.secondary)),
        )
        .child(channel(Tick::Dash, "payments-rewrite", None))
        .child(sess(Tick::On, 0., false, St::NeedsYou, "Ledger schema migration", true))
        .child(sess(Tick::Along, 6., true, St::NeedsYou, "Try BRIN index", false))
        .child(sess(Tick::Off, 18., true, St::Working, "Partial BRIN on posted", false))
        .child(sess(Tick::Off, 6., true, St::Idle, "Review the schema", false))
        .child(sess(Tick::On, 0., false, St::Working, "Refund retries", true))
        .child(sess(Tick::Off, 0., false, St::Idle, "API rate limiter", false))
        .child(channel(Tick::Dash, "infra", None))
        .child(sess(Tick::Off, 0., false, St::Working, "Benchmark BRIN", false))
        .child(sess(Tick::On, 0., false, St::Exited, "Upgrade Postgres 17", true))
        .child(channel(Tick::Off, "pr-reviews", Some("1")))
        .child(channel(Tick::Barred, "web", None))
        .child(div().px(px(14.)).pt(px(12.)).pb(px(4.)).child(label(look, "Hosts")))
        .child(host(Tick::Dash, St::Working, "This Mac", None))
        .child(host(Tick::Dash, St::Working, "devbox-1", None))
        .child(host(Tick::Barred, St::Offline, "gpu-box", Some("offline")))
        .child(div().flex_1())
        .child(
            div().px(px(10.)).pb(px(6.)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .p(px(12.))
                    .rounded(px(10.))
                    .bg(c(look.card))
                    .border_1()
                    .border_color(c(look.line))
                    .shadow_sm()
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child("4 selected"))
                    .child(quiet(look, "3 picked \u{b7} 1 fork goes along"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .mt(px(8.))
                            .child(button(look, Kind::Secondary, "Cancel"))
                            .child(red_outline_button(look, "Delete 4 sessions\u{2026}").flex_1()),
                    ),
            ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(34.))
                .px(px(14.))
                .border_t_1()
                .border_color(c(look.line))
                .child(icon(IconName::Settings, look.accent))
                .child("Settings"),
        )
}

fn home_body(look: Look, selecting: bool) -> Div {
    let row = |tick: Option<Tick>, st: St, title: &'static str, place: &'static str, what: &'static str, when: &'static str, act: Option<Div>| {
        let tinted = matches!(tick, Some(Tick::On | Tick::Along));
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(9.))
            .border_b_1()
            .border_color(c(look.line))
            .when(tinted, |r| r.bg(tint(look)))
            .children(tick.map(|t| tick_box(look, 14., t)))
            .child(status(st))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(div().font_weight(FontWeight::MEDIUM).child(title))
                            .child(secondary(look, place)),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(if st == St::NeedsYou { ORANGE } else { look.secondary }))
                            .child(what),
                    ),
            )
            .child(secondary(look, when))
            .children(if selecting { None } else { act })
    };
    let tk = |on: bool| selecting.then_some(if on { Tick::On } else { Tick::Off });
    div().flex().justify_center().p(px(24.)).text_size(px(13.)).child(
        div()
            .w(px(720.))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(pages::group(
                look,
                "Needs you \u{b7} 3",
                vec![
                    row(tk(true), St::NeedsYou, "Ledger schema migration", "# payments-rewrite \u{b7} devbox-1", "Asks to move PAY-3 to Done", "2m", Some(button(look, Kind::Primary, "Review"))),
                    row(tk(true), St::NeedsYou, "Try BRIN index", "fork of Ledger schema", "Asks to push try-brin to origin", "9m", Some(button(look, Kind::Primary, "Review"))),
                    row(tk(false), St::Exited, "Upgrade Postgres 17", "# infra \u{b7} devbox-1", "Exited with code 1 after the dump step", "40m", Some(button(look, Kind::Secondary, "Resume"))),
                ],
            ))
            .child(pages::group(
                look,
                "Working \u{b7} 3",
                vec![
                    row(tk(true), St::Working, "Partial BRIN on posted_at", "fork of Try BRIN index", "Comparing partial BRIN on posted_at", "now", None),
                    row(tk(true), St::Working, "Refund retries", "# payments-rewrite", "Running cargo test -p refunds", "now", None),
                    row(tk(false), St::Working, "Benchmark BRIN", "# infra \u{b7} devbox-1", "pgbench run 2 of 4", "1m", None),
                ],
            ))
            .child(pages::group(
                look,
                "Recent",
                vec![
                    row(tk(true), St::Idle, "API rate limiter", "# payments-rewrite \u{b7} devbox-1", "Finished: pushed and opened PR #491", "1h", Some(button(look, Kind::Secondary, "Open PR"))),
                    row(tk(false), St::Idle, "acme/api #482", "# pr-reviews", "3 new commits since your review", "2h", Some(button(look, Kind::Secondary, "Review"))),
                ],
            )),
    )
}

fn bottom_bar(
    look: Look,
    selected: &'static str,
    note: &'static str,
    delete_label: &'static str,
    menu_open: bool,
) -> Div {
    let select_btn = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(9.))
        .pr(px(5.))
        .rounded(px(6.))
        .bg(c(look.fill))
        .border_1()
        .border_color(c(if menu_open { look.accent } else { look.line }))
        .text_size(px(12.))
        .text_color(c(if menu_open { look.accent } else { look.text }))
        .child("Select")
        .child(small_icon(IconName::ChevronsUpDown, look.secondary));

    let bar = div()
        .relative()
        .flex()
        .items_center()
        .h(px(48.))
        .px(px(20.))
        .gap(px(12.))
        .flex_shrink_0()
        .border_t_1()
        .border_color(c(look.line))
        .bg(c(look.panel))
        .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).flex_shrink_0().child(selected))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_size(px(12.)).text_color(c(look.secondary)).child(note))
        .child(select_btn)
        .child(div().text_size(px(11.5)).text_color(c(look.tertiary)).whitespace_nowrap().flex_shrink_0().child(KEY_HINT))
        .child(button(look, Kind::Secondary, "Cancel"))
        .child(red_outline_button(look, delete_label));

    if !menu_open {
        return bar;
    }

    let head = |text: &'static str| {
        div()
            .px(px(8.))
            .pt(px(5.))
            .pb(px(2.))
            .text_size(px(11.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(look.secondary))
            .child(text)
    };
    let entry = |checked: bool, text: &'static str, right: &'static str, dim: bool| {
        div()
            .flex()
            .items_center()
            .h(px(25.))
            .px(px(8.))
            .gap(px(6.))
            .rounded(px(5.))
            .text_size(px(12.5))
            .text_color(c(if dim { look.tertiary } else { look.text }))
            .child(div().w(px(14.)).flex_shrink_0().children(checked.then(|| small_icon(IconName::Check, look.text))))
            .child(div().flex_1().whitespace_nowrap().child(text))
            .child(div().text_size(px(12.)).text_color(c(look.secondary)).child(right))
    };
    let menu = div()
        .absolute()
        .bottom(px(52.))
        .right(px(380.))
        .w(px(236.))
        .p(px(5.))
        .flex()
        .flex_col()
        .gap(px(1.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.line))
        .rounded(px(7.))
        .shadow_lg()
        .child(head("Every session in a channel"))
        .child(entry(true, "# payments-rewrite", "6", false))
        .child(entry(false, "# infra", "2", false))
        .child(entry(false, "# pr-reviews", "1", false))
        .child(hline(look).my(px(4.)))
        .child(head("Every session on a machine"))
        .child(entry(false, "This Mac", "3", false))
        .child(entry(false, "devbox-1", "6", false))
        .child(entry(false, "gpu-box", "offline", true))
        .child(hline(look).my(px(4.)))
        .child(entry(false, "Everything on Home", "\u{2318}A", false))
        .child(entry(false, "Nothing", "", false));
    bar.child(menu)
}

// ---- Sheet wrapper

fn sheet_frame(look: Look, under: Div, body: Div) -> Div {
    under.relative().child(scrim(look)).child(
        div()
            .absolute()
            .top(px(24.))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(700.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .p(px(20.))
                    .rounded(px(12.))
                    .bg(c(look.card))
                    .border_1()
                    .border_color(c(look.line))
                    .shadow_lg()
                    .text_size(px(13.))
                    .child(body),
            ),
    )
}

fn trash_box(look: Look) -> Div {
    div()
        .p(px(12.))
        .rounded(px(8.))
        .bg(c(look.fill))
        .text_size(px(12.5))
        .text_color(c(look.secondary))
        .child(
            "The terminals end for good. The sessions go to the trash on This Mac and devbox-1 \
             for 30 days: restore one from the machine\u{2019}s Storage > Trash and it comes back \
             archived, with its work. There is no Undo.",
        )
}

fn session_row_head(
    look: Look,
    rides: bool,
    title: &'static str,
    changed: bool,
    ran: &'static str,
    state: &'static str,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(small_icon(
            if rides { IconName::GitFork } else { IconName::SquareTerminal },
            look.secondary,
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().font_weight(FontWeight::MEDIUM).child(title))
                .when(changed, |r| r.child(changed_tag())),
        )
        .child(quiet(look, ran).w(px(170.)).flex_shrink_0())
        .child(quiet(look, state).w(px(56.)).flex_shrink_0().flex().justify_end())
}

// ---- Public design frames

/// `m1-1`: Sidebar in select mode with 3 picked + 1 context fork going along.
pub fn sidebar_select(look: Look) -> Div {
    div()
        .size_full()
        .flex()
        .child(select_sidebar(look))
        .child(handle(look, false))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .bg(c(look.content))
                .child(pages::toolbar(
                    look,
                    "Home",
                    None,
                    vec![
                        button(look, Kind::Secondary, "Select"),
                        button(look, Kind::Primary, "New session"),
                    ],
                ))
                .child(home_body(look, false).flex_1().min_h_0().overflow_hidden()),
        )
}

/// `m1-2`: Confirm sheet for 4 picked sessions (3 picked + 1 fork along).
pub fn plan_picked(look: Look) -> Div {
    plan_picked_inner(look, false)
}

/// `m1-4`: Stale plan notice with `changed` tags and updated items.
pub fn plan_stale(look: Look) -> Div {
    plan_picked_inner(look, true)
}

fn plan_picked_inner(look: Look, stale: bool) -> Div {
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Delete 4 sessions?"));
    if stale {
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .p(px(14.))
                .rounded(px(8.))
                .bg(c(look.attention_bg))
                .text_color(c(ORANGE))
                .text_size(px(12.5))
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Something changed since this plan. Nothing was deleted."))
                .child("Refund retries: 1 new commit on uhvesta/refund-retries.")
                .child("Ledger schema migration: a new terminal started (zsh).")
                .child("This is the new plan. Check it and delete again."),
        );
    }
    body = body
        .child(quiet(look, "In 2 channels, on This Mac and devbox-1.").text_size(px(12.5)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .text_size(px(12.5))
                .child(lined(
                    IconName::SquareTerminal,
                    ORANGE,
                    13.,
                    if stale {
                        "5 running terminals end, in 3 sessions."
                    } else {
                        "4 running terminals end, in 3 sessions."
                    },
                ))
                .child(lined(IconName::GitBranch, ORANGE, 13., "2 sessions hold work that was never pushed."))
                .child(lined(IconName::GitFork, look.text, 13., "1 fork goes with its parent. 2 forks stay.")),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .py(px(10.))
                .border_t_1()
                .border_color(c(look.line))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("# payments-rewrite"))
                                .child(quiet(look, "3 of 6 sessions")),
                        )
                        .child(session_row_head(look, false, "Ledger schema migration", stale, "Claude Code \u{b7} devbox-1", "live"))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .pl(px(21.))
                                .text_size(px(12.))
                                .child(lined(
                                    IconName::SquareTerminal,
                                    ORANGE,
                                    11.,
                                    if stale {
                                        "Ends 3 running terminals: claude (waiting for you), zsh, zsh (new)."
                                    } else {
                                        "Ends 2 running terminals: claude (waiting for you), zsh."
                                    },
                                ))
                                .child(lined(IconName::GitBranch, ORANGE, 11., "ledger: 3 uncommitted files on uhvesta/ledger-schema, never pushed."))
                                .child(work_check(look, true)),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .pl(px(24.))
                                .child(session_row_head(look, true, "Try BRIN index", false, "Claude Code \u{b7} devbox-1", "live"))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(px(3.))
                                        .pl(px(21.))
                                        .text_size(px(12.))
                                        .child(quiet(look, "Goes with Ledger schema migration: this fork works in the same folder."))
                                        .child(lined(IconName::SquareTerminal, ORANGE, 11., "Ends 1 running terminal: claude (waiting for you).")),
                                ),
                        )
                        .child(quiet(look, "Partial BRIN on posted_at and Review the schema stay. These forks have their own folders; they move up in the tree.").pl(px(21.)))
                        .child(session_row_head(look, false, "Refund retries", stale, "Claude Code \u{b7} This Mac", "live"))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .pl(px(21.))
                                .text_size(px(12.))
                                .child(lined(IconName::SquareTerminal, ORANGE, 11., "Ends 1 running terminal: claude (working)."))
                                .child(lined(
                                    IconName::GitBranch,
                                    ORANGE,
                                    11.,
                                    if stale {
                                        "api: 3 commits on uhvesta/refund-retries never pushed (1 new)."
                                    } else {
                                        "api: 2 commits on uhvesta/refund-retries never pushed."
                                    },
                                ))
                                .child(work_check(look, false)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("# infra"))
                                .child(quiet(look, "1 of 2 sessions")),
                        )
                        .child(session_row_head(look, false, "Upgrade Postgres 17", false, "Claude Code \u{b7} devbox-1", "live"))
                        .child(quiet(look, "Nothing running. Nothing unpushed.").pl(px(21.))),
                ),
        )
        .child(trash_box(look))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().flex_1().text_size(px(11.5)).text_color(c(look.tertiary)).child("Tick \u{201c}Delete that work too\u{201d} for Refund retries to continue"))
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(disabled_button(look, "Delete 4 sessions")),
        );
    sheet_frame(look, sidebar_select(look), body)
}

/// `m1-3`: Delete all sessions sheet with empty count field and unticked work blockers.
pub fn plan_all_empty(look: Look) -> Div {
    let body = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Delete all 23 sessions?"))
        .child(quiet(look, "Every session, live and archived: 23 in 4 channels, on This Mac and devbox-1.").text_size(px(12.5)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .text_size(px(12.5))
                .child(lined(IconName::SquareTerminal, ORANGE, 13., "9 running terminals end, in 8 sessions."))
                .child(lined(IconName::GitBranch, ORANGE, 13., "3 sessions hold work that was never pushed."))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().ml(px(3.)).size(px(8.)).rounded_full().bg(c(look.tertiary)))
                        .child(quiet(look, "gpu-box can\u{2019}t be reached. Its 4 sessions are left alone.").text_size(px(12.5))),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .py(px(10.))
                .border_t_1()
                .border_color(c(look.line))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("# payments-rewrite"))
                                .child(quiet(look, "9 sessions")),
                        )
                        .child(session_row_head(look, false, "Ledger schema migration", false, "Claude Code \u{b7} devbox-1", "live"))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .pl(px(21.))
                                .text_size(px(12.))
                                .child(lined(IconName::SquareTerminal, ORANGE, 11., "Ends 2 running terminals: claude (waiting for you), zsh."))
                                .child(lined(IconName::GitBranch, ORANGE, 11., "ledger: 3 uncommitted files on uhvesta/ledger-schema, never pushed."))
                                .child(work_check(look, true)),
                        )
                        .child(session_row_head(look, false, "Refund retries", false, "Claude Code \u{b7} This Mac", "live"))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .pl(px(21.))
                                .text_size(px(12.))
                                .child(lined(IconName::SquareTerminal, ORANGE, 11., "Ends 1 running terminal: claude (working)."))
                                .child(lined(IconName::GitBranch, ORANGE, 11., "api: 2 commits on uhvesta/refund-retries never pushed."))
                                .child(work_check(look, false)),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(4.))
                                .child(quiet(look, "5 more, nothing running and nothing unpushed \u{b7}"))
                                .child(div().text_size(px(11.5)).text_color(c(look.accent)).child("Show")),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("# infra"))
                                .child(quiet(look, "4 sessions")),
                        )
                        .child(session_row_head(look, false, "Benchmark BRIN", false, "Codex \u{b7} devbox-1", "live"))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .pl(px(21.))
                                .text_size(px(12.))
                                .child(lined(IconName::GitBranch, ORANGE, 11., "infra: 1 commit on uhvesta/bench-brin not pushed."))
                                .child(work_check(look, false)),
                        ),
                ),
        )
        .child(div().text_size(px(11.5)).text_color(c(look.tertiary)).child("Desk sessions are not part of this. The Desk has its own Clean up."))
        .child(trash_box(look))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child("Type the number of sessions, 23, to confirm")
                .child(field(look, "", true).w(px(108.))),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().flex_1().text_size(px(11.5)).text_color(c(look.tertiary)).child("Tick \u{201c}Delete that work too\u{201d} for 2 sessions and type 23 to continue"))
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(disabled_button(look, "Delete all 23 sessions")),
        );
    sheet_frame(look, pages::home_list(look), body)
}

/// `m2-1`: Home in select mode with `# payments-rewrite` picked from the Select menu.
pub fn home_select_channel(look: Look) -> Div {
    let done = div()
        .flex()
        .items_center()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .border_1()
        .border_color(c(look.accent))
        .bg(c(look.card))
        .text_color(c(look.accent))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child("Done");
    div()
        .size_full()
        .flex()
        .child(sidebar::sidebar(look, Side::at("Home")))
        .child(handle(look, false))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .bg(c(look.content))
                .child(pages::toolbar(
                    look,
                    "Home",
                    None,
                    vec![done, button(look, Kind::Primary, "New session").opacity(0.45)],
                ))
                .child(home_body(look, true).flex_1().min_h_0().overflow_hidden())
                .child(bottom_bar(
                    look,
                    "6 selected",
                    "All of # payments-rewrite \u{b7} Review the schema is not on Home",
                    "Delete 6 sessions\u{2026}",
                    true,
                )),
        )
}

/// `m2-2`: Machine page (`devbox-1`) in select mode with all 6 sessions ticked.
pub fn machine_select_all(look: Look) -> Div {
    let row = |st: St, title: &'static str, chan: &'static str, agent: &'static str| {
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .px(px(12.))
            .py(px(9.))
            .border_b_1()
            .border_color(c(look.line))
            .bg(tint(look))
            .child(tick_box(look, 14., Tick::On))
            .child(status(st))
            .child(div().w(px(200.)).font_weight(FontWeight::MEDIUM).child(title))
            .child(secondary(look, chan).w(px(150.)))
            .child(secondary(look, agent).flex_1())
    };
    let sessions = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(tick_box(look, 14., Tick::On))
                .child(div().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).child("Select all on devbox-1"))
                .child(quiet(look, "6 of 6"))
                .child(div().flex_1())
                .child(div().text_size(px(12.)).text_color(c(look.accent)).child("Done")),
        )
        .child(
            card(look)
                .overflow_hidden()
                .child(row(St::NeedsYou, "Ledger schema migration", "# payments-rewrite", "Claude Code"))
                .child(row(St::NeedsYou, "Try BRIN index", "# payments-rewrite", "Claude Code"))
                .child(row(St::Idle, "API rate limiter", "# payments-rewrite", "Codex"))
                .child(row(St::Working, "Partial BRIN on posted_at", "# payments-rewrite", "Claude Code"))
                .child(row(St::Working, "Benchmark BRIN", "# infra", "Codex"))
                .child(row(St::Exited, "Upgrade Postgres 17", "# infra", "Claude Code")),
        );

    div()
        .size_full()
        .flex()
        .child(sidebar::sidebar(look, Side::at("devbox-1")))
        .child(handle(look, false))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .bg(c(look.content))
                .child(pages::toolbar(
                    look,
                    "Hosts \u{203a} devbox-1",
                    Some("Ubuntu 24.04 \u{b7} x86_64 \u{b7} devbox-1.lan (ssh) \u{b7} humatad 2.0.3"),
                    vec![
                        button(look, Kind::Secondary, "Open a shell"),
                        button(look, Kind::Primary, "New session on devbox-1"),
                    ],
                ))
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_hidden()
                        .p(px(24.))
                        .child(sessions),
                )
                .child(bottom_bar(
                    look,
                    "6 selected",
                    "Every session on devbox-1",
                    "Delete 6 sessions\u{2026}",
                    false,
                )),
        )
}

/// `m2-5`: Delete all sessions sheet in dark mode, scrolled to the unreachable
/// machine, count typed (`23`) and filled red button active.
pub fn plan_all_typed_dark(look: Look) -> Div {
    let body = div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Delete all 23 sessions?"))
        .child(quiet(look, "Every session, live and archived: 23 in 4 channels, on This Mac and devbox-1.").text_size(px(12.5)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .text_size(px(12.5))
                .child(lined(IconName::SquareTerminal, ORANGE, 13., "9 running terminals end, in 8 sessions."))
                .child(lined(IconName::GitBranch, ORANGE, 13., "3 sessions hold work that was never pushed."))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(div().ml(px(3.)).size(px(8.)).rounded_full().bg(c(look.tertiary)))
                        .child(quiet(look, "gpu-box can\u{2019}t be reached. Its 4 sessions are left alone.").text_size(px(12.5))),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .py(px(10.))
                .border_t_1()
                .border_color(c(look.line))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .pl(px(21.))
                        .text_size(px(12.))
                        .child(lined(IconName::SquareTerminal, ORANGE, 11., "Ends 1 running terminal: codex (working)."))
                        .child(lined(IconName::GitBranch, ORANGE, 11., "infra: 1 commit on uhvesta/bench-brin not pushed."))
                        .child(work_check(look, true)),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(4.))
                        .child(quiet(look, "4 more, nothing running and nothing unpushed: Upgrade Postgres 17 and 3 archived \u{b7}"))
                        .child(div().text_size(px(11.5)).text_color(c(look.accent)).child("Show")),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("# pr-reviews"))
                                .child(quiet(look, "4 sessions")),
                        )
                        .child(session_row_head(look, false, "acme/api #482", false, "Claude Code \u{b7} This Mac", "live"))
                        .child(lined(IconName::SquareTerminal, ORANGE, 11., "Ends 1 running terminal: claude (idle).").pl(px(21.)).text_size(px(12.)))
                        .child(quiet(look, "3 archived, nothing unpushed: acme/api #455, acme/api #461, acme/ledger #88")),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("# web"))
                                .child(quiet(look, "6 sessions")),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(4.))
                                .child(quiet(look, "6 archived, nothing unpushed \u{b7}"))
                                .child(div().text_size(px(11.5)).text_color(c(look.accent)).child("Show")),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().size(px(8.)).rounded_full().bg(c(look.tertiary)))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("gpu-box"))
                                .child(quiet(look, "can\u{2019}t reach, left alone \u{b7} 4 sessions")),
                        )
                        .child(
                            quiet(
                                look,
                                "Not deleted: Train reranker, Eval sweep (# infra), Render farm probe, Font subsetting (# web). Delete them when gpu-box is back.",
                            )
                            .pl(px(16.)),
                        ),
                ),
        )
        .child(div().text_size(px(11.5)).text_color(c(look.tertiary)).child("Desk sessions are not part of this. The Desk has its own Clean up."))
        .child(trash_box(look))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child("Type the number of sessions, 23, to confirm")
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(28.))
                        .w(px(108.))
                        .px(px(8.))
                        .rounded(px(6.))
                        .bg(c(look.content))
                        .border_1()
                        .border_color(c(look.accent))
                        .child("23"),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().flex_1().text_size(px(11.5)).text_color(c(look.tertiary)).child("Return deletes"))
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(red_filled_button("Delete all 23 sessions")),
        );
    sheet_frame(look, pages::settings(look), body)
}

/// `m2-6`: After delete — toast with no Undo action.
pub fn after_delete(look: Look) -> Div {
    pages::home_list(look).relative().child(
        div()
            .absolute()
            .bottom(px(24.))
            .left_0()
            .w_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(14.))
                    .py(px(9.))
                    .rounded(px(10.))
                    .bg(c(if look.dark { 0x3a3a3d } else { 0x2c2c2e }))
                    .text_color(c(0xffffff))
                    .text_size(px(12.5))
                    .shadow_lg()
                    .child("Deleted 4 sessions. They are in the trash on This Mac and devbox-1 for 30 days."),
            ),
    )
}
