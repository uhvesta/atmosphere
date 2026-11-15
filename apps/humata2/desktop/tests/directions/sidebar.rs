//! The left sidebar: Home, Inbox, Board, the channels with their session
//! trees, Hosts and Settings (NV-2), with an optional Pinned section (NV-21).

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;

pub const WIDTH: f32 = 236.;

/// What the sidebar shows.
#[derive(Clone, Copy)]
pub struct Side {
    /// The selected row's label.
    pub selected: &'static str,
    /// Show the Pinned section at the top.
    pub pinned: bool,
    pub tree: Tree,
    /// Every channel folded to one row.
    pub folded: bool,
}

impl Side {
    pub const fn at(selected: &'static str) -> Self {
        Self { selected, pinned: false, tree: Tree::Indent, folded: false }
    }
}

/// How a channel's session tree is drawn: the options for the owner.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tree {
    /// Option A: forks indented under their parent with a fork glyph.
    Indent,
    /// Option B: connector lines, as in the channel's Sessions table.
    Lines,
}

fn row(look: Look, lead: Option<AnyElement>, text: &'static str, trail: Option<AnyElement>, indent: f32, on: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(7.))
        .h(px(26.))
        .mx(px(10.))
        .pl(px(8. + indent))
        .pr(px(8.))
        .rounded(px(6.))
        .when(on, |row| row.bg(c(look.selection)).text_color(c(look.selection_text)))
        .children(lead)
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(text))
        .children(trail)
}

fn nav(look: Look, side: Side, glyph: IconName, text: &'static str, count: Option<&'static str>) -> Div {
    let on = side.selected == text;
    row(
        look,
        Some(icon(glyph, if on { look.selection_text } else { look.accent }).into_any_element()),
        text,
        count.map(|count| {
            div()
                .text_size(px(11.))
                .text_color(c(if on { look.selection_text } else { look.secondary }))
                .child(count)
                .into_any_element()
        }),
        0.,
        on,
    )
}

fn session(look: Look, side: Side, state: St, text: &'static str, indent: f32, trail: Option<AnyElement>) -> Div {
    row(look, Some(status(state).into_any_element()), text, trail, indent, side.selected == text)
}

/// One session in a channel's tree (CH-16).
struct Node {
    depth: u8,
    state: St,
    title: &'static str,
    fork: bool,
    unread: bool,
    inbox: Option<&'static str>,
    /// The tree connectors drawn before it in the Lines style.
    lines: &'static str,
}

const PAYMENTS: &[Node] = &[
    Node { depth: 0, state: St::NeedsYou, title: "Ledger schema migration", fork: false, unread: true, inbox: Some("2"), lines: "├─" },
    Node { depth: 1, state: St::Working, title: "Try BRIN index", fork: true, unread: false, inbox: None, lines: "│ ├─" },
    Node { depth: 2, state: St::Working, title: "Partial BRIN on posted_at", fork: true, unread: false, inbox: None, lines: "│ │ └─" },
    Node { depth: 1, state: St::Idle, title: "Review the schema", fork: true, unread: false, inbox: None, lines: "│ └─" },
    Node { depth: 0, state: St::Working, title: "Refund retries", fork: false, unread: true, inbox: None, lines: "├─" },
    Node { depth: 0, state: St::Idle, title: "API rate limiter", fork: false, unread: false, inbox: None, lines: "└─" },
];

const INFRA: &[Node] = &[
    Node { depth: 0, state: St::Working, title: "Benchmark BRIN", fork: false, unread: false, inbox: None, lines: "├─" },
    Node { depth: 0, state: St::Exited, title: "Upgrade Postgres 17", fork: false, unread: true, inbox: None, lines: "└─" },
];

fn node(look: Look, side: Side, node: &Node) -> Div {
    let on = side.selected == node.title;
    let faint = if on { look.selection_text } else { look.tertiary };
    let lead = match side.tree {
        Tree::Indent => div()
            .flex()
            .items_center()
            .gap(px(5.))
            .when(node.fork, |lead| lead.child(small_icon(IconName::GitFork, faint)))
            .child(status(node.state)),
        Tree::Lines => div()
            .flex()
            .items_center()
            .gap(px(5.))
            .child(div().font_family(MONO).text_size(px(11.)).text_color(c(faint)).child(node.lines))
            .child(status(node.state)),
    };
    let indent = match side.tree {
        Tree::Indent => 10. + 16. * f32::from(node.depth) - if node.fork { 17. } else { 0. },
        Tree::Lines => 2.,
    };
    let trail = node.inbox.map(|count| {
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .text_size(px(11.))
            .text_color(c(if on { look.selection_text } else { RED }))
            .child(small_icon(IconName::Inbox, if on { look.selection_text } else { RED }))
            .child(count)
            .into_any_element()
    });
    row(look, Some(lead.into_any_element()), node.title, trail, indent.max(0.), on)
        .when(node.unread, |row| row.font_weight(FontWeight::SEMIBOLD))
}

fn channel(look: Look, side: Side, text: &'static str, open: bool, rollup: Option<(St, &'static str)>, unread: bool) -> Div {
    let on = side.selected == text;
    row(
        look,
        Some(
            small_icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, if on { look.selection_text } else { look.tertiary })
                .into_any_element(),
        ),
        text,
        rollup.filter(|_| !open).map(|(state, count)| {
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(status(state))
                .child(div().text_size(px(11.)).text_color(c(if on { look.selection_text } else { look.secondary })).child(count))
                .into_any_element()
        }),
        -4.,
        on,
    )
    .font_weight(if unread { FontWeight::BOLD } else { FontWeight::MEDIUM })
}

fn channels(look: Look, side: Side) -> Div {
    let open = !side.folded;
    let mut list = div().flex().flex_col();
    list = list.child(channel(look, side, "payments-rewrite", open, Some((St::NeedsYou, "6")), true));
    if open {
        list = list.children(PAYMENTS.iter().map(|n| node(look, side, n)));
    }
    list = list.child(channel(look, side, "infra", open, Some((St::Exited, "2")), true));
    if open {
        list = list.children(INFRA.iter().map(|n| node(look, side, n)));
    }
    list.child(channel(look, side, "pr-reviews", false, Some((St::Idle, "1")), false))
        .child(channel(look, side, "web", false, None, false))
}

fn section(look: Look, title: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .px(px(18.))
        .pt(px(12.))
        .pb(px(3.))
        .child(label(look, title).flex_1())
}

fn vip() -> AnyElement {
    div()
        .px(px(4.))
        .rounded(px(3.))
        .bg(c(PURPLE))
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(c(0xffffff))
        .child("VIP")
        .into_any_element()
}

pub fn sidebar(look: Look, side: Side) -> Div {
    div()
        .w(px(WIDTH))
        .h_full()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .bg(c(look.sidebar))
        .text_size(px(13.))
        .child(
            div()
                .flex()
                .items_center()
                .h(px(52.))
                .pl(px(20.))
                .pr(px(14.))
                .gap(px(12.))
                .child(traffic_lights())
                .child(div().flex_1())
                .child(icon(IconName::PanelLeft, look.secondary))
                .child(icon(IconName::SquarePen, look.secondary)),
        )
        .child(
            div()
                .mx(px(10.))
                .mb(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(6.))
                .bg(c(look.fill))
                .text_color(c(look.tertiary))
                .child(small_icon(IconName::Search, look.tertiary))
                .child(div().flex_1().child("Search"))
                .child(div().text_size(px(11.)).child("⌘K")),
        )
        .when(side.pinned, |bar| {
            bar.child(section(look, "Pinned"))
                .child(
                    session(look, side, St::NeedsYou, "Ledger schema migration", 0., Some(vip()))
                        .h(px(40.))
                        .items_start()
                        .pt(px(5.))
                        .child(div()),
                )
                .child(
                    div()
                        .ml(px(43.))
                        .mt(px(-18.))
                        .mb(px(2.))
                        .text_size(px(11.))
                        .text_color(c(ORANGE))
                        .child("Asks to move PAY-3 to Done · 2m"),
                )
                .child(row(
                    look,
                    Some(small_icon(IconName::GitPullRequest, look.accent).into_any_element()),
                    "acme/api #482",
                    Some(secondary(look, "3 new").into_any_element()),
                    0.,
                    false,
                ))
        })
        .child(div().h(px(4.)))
        .child(nav(look, side, IconName::House, "Home", None))
        .child(nav(look, side, IconName::Inbox, "Inbox", Some("7")))
        .child(nav(look, side, IconName::SquareKanban, "Board", None))
        .child(section(look, "Channels").child(small_icon(IconName::Plus, look.secondary)))
        .child(channels(look, side))
        .child(section(look, "Hosts"))
        .child(row(look, Some(status(St::Working).into_any_element()), "This Mac", None, 0., side.selected == "This Mac"))
        .child(row(look, Some(status(St::Working).into_any_element()), "devbox-1", None, 0., side.selected == "devbox-1"))
        .child(row(
            look,
            Some(status(St::Offline).into_any_element()),
            "gpu-box",
            Some(secondary(look, "offline").into_any_element()),
            0.,
            false,
        ))
        .child(div().flex_1())
        .child(div().pb(px(10.)).child(nav(look, side, IconName::Settings, "Settings", None)))
}

/// The sidebar collapsed to a thin strip that opens it again (NV-13).
pub fn strip(look: Look) -> Div {
    div()
        .w(px(14.))
        .h_full()
        .flex_shrink_0()
        .bg(c(look.sidebar))
        .border_r_1()
        .border_color(c(look.line))
        .flex()
        .items_center()
        .justify_center()
        .child(small_icon(IconName::ChevronRight, look.secondary))
}
