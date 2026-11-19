//! The To-do list (BOARDSIMPLE): one ranked list for every channel, an item
//! open in its drawer, the filters, quick add, the folded groups, a pull
//! request review item, a suggested item, and the list at three sizes. Drawn
//! from the signed-off mockups at their own 1600x950 size, 1x.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{prelude::*, *};

use crate::kit::*;

/// Type size: the mockups' px, unscaled (these frames are 1600x950).
fn f(size: f32) -> Pixels {
    px(size)
}

/// Geometry: the mockups' px, unscaled.
fn m(size: f32) -> Pixels {
    px(size)
}

fn tone(look: Look, light: u32, dark: u32) -> Rgba {
    c(if look.dark { dark } else { light })
}

fn soft(look: Look) -> Rgba {
    tone(look, 0x55555b, 0xc4c4ca)
}

/// Separators: the hairline colour of the mockups' lists and panes.
pub(crate) fn rule(look: Look) -> Rgba {
    tone(look, 0xe6e6ea, 0x343437)
}

/// Borders of buttons, inputs and chips.
pub(crate) fn edge(look: Look) -> Rgba {
    tone(look, 0xd4d4da, 0x48484c)
}

fn faint_line(look: Look) -> Rgba {
    tone(look, 0xf0f0f3, 0x2a2a2d)
}

fn amber(look: Look) -> Rgba {
    tone(look, 0xc76a00, 0xffb340)
}

fn red(look: Look) -> Rgba {
    tone(look, 0xa51d1d, 0xff9088)
}

fn teal(look: Look) -> Rgba {
    tone(look, 0x0f766e, 0x35c4b5)
}

fn selected_bg(look: Look) -> Rgba {
    tone(look, 0xeef4ff, 0x20304d)
}

// ---- Small parts

fn pill(look: Look, text: &'static str, bg: Rgba, fg: Rgba) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(m(7.))
        .py(px(1.))
        .rounded(m(5.))
        .text_size(f(12.))
        .bg(bg)
        .text_color(fg)
        .child(text)
        .when(false, |p| p.text_color(c(look.text)))
}

pub(crate) fn tag(look: Look, text: &'static str) -> Div {
    pill(look, text, c(look.fill), soft(look))
}

fn tag_on(look: Look, text: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .px(m(7.))
        .py(px(1.))
        .rounded(m(5.))
        .text_size(f(12.))
        .border_1()
        .border_color(c(look.accent))
        .text_color(c(look.accent))
        .child(text)
}

fn mini(look: Look, text: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(m(9.))
        .py(m(2.))
        .rounded(m(6.))
        .border_1()
        .border_color(edge(look))
        .bg(c(look.card))
        .text_size(f(12.))
        .font_weight(FontWeight::MEDIUM)
        .child(text)
}

pub(crate) fn btn(look: Look, text: &'static str, primary: bool) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(m(12.))
        .py(m(6.))
        .rounded(m(7.))
        .border_1()
        .text_size(f(13.))
        .font_weight(if primary { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
        .when(primary, |b| b.bg(tone(look, 0x0a66f5, 0x2f7cf6)).border_color(tone(look, 0x0a66f5, 0x2f7cf6)).text_color(c(0xffffff)))
        .when(!primary, |b| b.bg(c(look.card)).border_color(edge(look)))
        .child(text)
}

pub(crate) fn lnk(look: Look, text: &'static str) -> Div {
    div().flex_shrink_0().whitespace_nowrap().text_color(c(look.accent)).child(text)
}

fn kbd_box(look: Look, key: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .px(m(5.))
        .rounded(m(4.))
        .border_1()
        .border_color(edge(look))
        .bg(c(look.card))
        .text_size(f(11.5))
        .text_color(c(look.secondary))
        .child(key)
}

fn spacer() -> Div {
    div().flex_1().min_w_0()
}

/// A check circle: the one control on a row.
fn check_circle(look: Look) -> Div {
    let ring = tone(look, 0xc9c9cf, 0x55555a);
    div()
        .size(m(20.))
        .ml(m(1.))
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(ring)
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(IconName::Check).size(m(12.)).text_color(ring))
}

fn closed_circle(look: Look) -> Div {
    div()
        .size(m(20.))
        .ml(m(1.))
        .flex_shrink_0()
        .rounded_full()
        .bg(teal(look))
        .flex()
        .items_center()
        .justify_center()
        .child(Icon::new(IconName::Check).size(m(12.)).text_color(c(0xffffff)))
}

fn no_check() -> Div {
    div().size(m(20.)).ml(m(1.)).flex_shrink_0()
}

// ---- Rows

#[derive(Clone, Copy, PartialEq, Eq)]
enum Due {
    Plain,
    Over,
    Soon,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Why {
    Plain,
    Red,
    Blue,
}

#[derive(Clone, Copy)]
enum Second {
    /// The sessions working on it; `ended` adds "session ended" and Resume.
    Sessions(&'static [(St, &'static str)], bool),
    /// A reason chip and a plain remainder.
    Reason(Why, &'static str, &'static str),
    Suggested,
}

/// One item row, as the list draws it.
#[derive(Clone, Copy)]
struct Item {
    key: &'static str,
    title: &'static str,
    channel: &'static str,
    due: Option<(Due, &'static str)>,
    tags: &'static [&'static str],
    steps: Option<&'static str>,
    effort: &'static str,
    impact: &'static str,
    second: Second,
    pull_request: bool,
}

impl Item {
    const fn new(key: &'static str, title: &'static str, channel: &'static str) -> Self {
        Self {
            key,
            title,
            channel,
            due: None,
            tags: &[],
            steps: None,
            effort: "",
            impact: "",
            second: Second::Reason(Why::Plain, "", ""),
            pull_request: false,
        }
    }
    const fn due(mut self, kind: Due, text: &'static str) -> Self {
        self.due = Some((kind, text));
        self
    }
    const fn tags(mut self, tags: &'static [&'static str]) -> Self {
        self.tags = tags;
        self
    }
    const fn steps(mut self, steps: &'static str) -> Self {
        self.steps = Some(steps);
        self
    }
    const fn size(mut self, effort: &'static str, impact: &'static str) -> Self {
        self.effort = effort;
        self.impact = impact;
        self
    }
    const fn on(mut self, sessions: &'static [(St, &'static str)], ended: bool) -> Self {
        self.second = Second::Sessions(sessions, ended);
        self
    }
    const fn why(mut self, kind: Why, text: &'static str, more: &'static str) -> Self {
        self.second = Second::Reason(kind, text, more);
        self
    }
}

const PAY3: Item = Item::new("PAY-3", "Ledger schema v2", "payments-rewrite")
    .due(Due::Soon, "Due tomorrow")
    .tags(&["migration", "db"])
    .steps("4/5")
    .size("M", "high")
    .on(&[(St::NeedsYou, "Ledger entries")], false);
const INF2: Item = Item::new("INF-2", "Postgres 17 upgrade", "infra")
    .due(Due::Over, "Overdue · 2 days")
    .tags(&["db"])
    .steps("1/4")
    .size("L", "high")
    .on(&[(St::Offline, "Upgrade Postgres 17")], true);
const WEB8: Item = Item::new("WEB-8", "Checkout page shows the new ledger totals", "web")
    .due(Due::Plain, "Fri 27 Nov")
    .tags(&["checkout"])
    .steps("2/3")
    .size("M", "medium")
    .on(&[(St::Working, "Ledger totals"), (St::Working, "Totals, another way")], false);
const INF6: Item = Item::new("INF-6", "Rotate the staging database credentials", "infra")
    .due(Due::Soon, "Due today")
    .tags(&["security"])
    .size("S", "medium")
    .why(Why::Blue, "Pinned", "");
const PAY7: Item = Item::new("PAY-7", "Refund retry with backoff", "payments-rewrite")
    .due(Due::Over, "Overdue · 1 day")
    .tags(&["refunds", "bug"])
    .size("S", "high")
    .why(Why::Plain, "High impact for a small job", "PAY-9 waits on it");
const PR_REVIEW: Item = Item {
    pull_request: true,
    ..Item::new("", "Review: Ledger reads behind a flag", "pr-reviews").why(Why::Plain, "Waiting on you for 2 days", "dana-k asked for your review")
};
const PAY5: Item = Item::new("PAY-5", "API rate limiter", "payments-rewrite")
    .due(Due::Plain, "Mon 23 Nov")
    .tags(&["api"])
    .steps("0/3")
    .size("M", "high")
    .why(Why::Plain, "High impact", "");
const TD3: Item = Item::new("TD-3", "Renew the Apple developer certificate", "No channel")
    .due(Due::Plain, "Mon 14 Dec")
    .tags(&["admin"])
    .size("S", "high")
    .why(Why::Plain, "High impact for a small job", "");
const IDEMPOTENCY: Item = Item::new("", "Idempotency keys for refund retries", "payments-rewrite").tags(&["refunds"]).size("S", "high");

#[derive(Clone, Copy, Default)]
struct Mark {
    /// The "Needs attention · …" tag's question.
    attention: Option<&'static str>,
    rank: Option<u32>,
    selected: bool,
    /// Draw as a plain open item even with sessions: no check circle change.
    suggested: bool,
    /// First row under a header, or next to the selected row: no hairline.
    lead: bool,
}

fn second_line(look: Look, item: &Item, mark: Mark) -> Div {
    let line = div().flex().items_center().gap(m(6.)).mt(m(3.)).text_size(f(12.)).text_color(c(look.secondary)).overflow_hidden().whitespace_nowrap();
    if mark.suggested || mark.attention == Some("add?") {
        return line
            .child(why_chip(look, Why::Plain, "Suggested"))
            .child("by copilot in Refund retries · not in your list until you say yes");
    }
    match item.second {
        Second::Sessions(sessions, ended) => line
            .children(sessions.iter().map(|(state, name)| {
                div()
                    .flex()
                    .items_center()
                    .gap(m(6.))
                    .flex_shrink_0()
                    .px(m(8.))
                    .py(m(2.))
                    .rounded(m(6.))
                    .bg(tone(look, 0xf3f3f5, 0x2d2d30))
                    .text_color(tone(look, 0x7a7a80, 0x9a9aa1))
                    .child(status(*state))
                    .child(*name)
            }))
            .when(ended, |line| line.child("session ended").child(mini(look, "Resume"))),
        Second::Reason(kind, text, more) => line
            .when(!text.is_empty(), |line| line.child(why_chip(look, kind, text)))
            .when(!more.is_empty(), |line| line.child(more)),
        Second::Suggested => line.child(why_chip(look, Why::Plain, "Suggested")),
    }
}

fn why_chip(look: Look, kind: Why, text: &'static str) -> Div {
    let (border, fg) = match kind {
        Why::Plain => (look.line, soft(look)),
        Why::Red => (if look.dark { 0x7a3636 } else { 0xf0b6b6 }, red(look)),
        Why::Blue => (if look.dark { 0x2f5ea8 } else { 0xb9d0fb }, c(look.accent)),
    };
    div()
        .flex_shrink_0()
        .px(m(6.))
        .h(m(17.))
        .flex()
        .items_center()
        .rounded(m(5.))
        .border_1()
        .border_color(c(border))
        .text_size(f(11.5))
        .text_color(fg)
        .child(text)
}

/// One item row. `narrow` drops the effort, channel and key columns for a
/// list beside a drawer or panel.
fn item_row(look: Look, item: Item, mark: Mark, narrow: bool) -> Div {
    let title = div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(f(14.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(item.title);
    let first = div()
        .flex()
        .items_center()
        .gap(m(7.))
        .overflow_hidden()
        .whitespace_nowrap()
        .child(title)
        .children(mark.attention.map(|question| {
            div()
                .flex_shrink_0()
                .px(m(7.))
                .py(px(1.))
                .rounded(m(5.))
                .text_size(f(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .bg(tone(look, 0xffe9c2, 0x5b3f0c))
                .text_color(tone(look, 0x7a4300, 0xffd995))
                .child(format!("Needs attention · {question}"))
        }))
        .when(item.pull_request, |line| {
            line.child(pill(look, "Pull request review", tone(look, 0xece9fd, 0x322c5c), tone(look, 0x4a2fa8, 0xcdc4ff)).font_weight(FontWeight::MEDIUM))
                .child(tag(look, "acme/api#483"))
                .child(tag(look, "acme/web#210"))
                .child(div().flex_shrink_0().text_size(f(12.)).text_color(c(look.secondary)).child("by dana-k"))
        })
        .children(item.steps.map(|steps| div().flex_shrink_0().text_size(f(12.)).text_color(c(look.secondary)).child(steps)))
        .children(item.due.map(|(kind, text)| match kind {
            Due::Plain => pill(look, text, c(look.fill), soft(look)),
            Due::Over => pill(look, text, tone(look, 0xfde4e4, 0x4b1f1f), red(look)).font_weight(FontWeight::SEMIBOLD),
            Due::Soon => pill(look, text, c(look.attention_bg), tone(look, 0x8a4b00, 0xffcf85)).font_weight(FontWeight::MEDIUM),
        }))
        .children(item.tags.iter().map(|text| tag(look, text)));
    let lead = if item.pull_request || mark.suggested || (mark.attention == Some("add?")) { no_check() } else { check_circle(look) };
    div()
        .flex()
        .items_center()
        .gap(m(11.))
        .px(m(8.))
        .py(m(7.))
        .min_h(m(50.))
        .rounded(m(9.))
        .relative()
        .when(!mark.lead && !mark.selected, |row| row.child(div().absolute().top_0().left_0().right_0().h(px(1.)).bg(faint_line(look))))
        .when(mark.selected, |row| row.bg(selected_bg(look)))
        .child(lead)
        .child(
            div()
                .w(m(17.))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .text_size(f(12.))
                .text_color(c(look.tertiary))
                .children(mark.rank.map(|rank| rank.to_string())),
        )
        .child(div().flex_1().min_w_0().overflow_hidden().flex().flex_col().child(first).child(second_line(look, &item, mark)))
        .when(!narrow, |row| {
            row.child(
                div()
                    .w(m(110.))
                    .pl(px(2.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .text_size(f(12.))
                    .text_color(soft(look))
                    .when(!item.effort.is_empty(), |size| {
                        size.child(
                            div()
                                .size(m(18.))
                                .mr(m(6.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(m(5.))
                                .bg(c(look.fill))
                                .text_size(f(11.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(item.effort),
                        )
                        .child(item.impact)
                    }),
            )
            .child(div().w(m(128.)).flex_shrink_0().text_size(f(12.)).text_color(c(look.secondary)).child(item.channel))
        })
        .child(div().w(m(52.)).flex_shrink_0().flex().justify_end().text_size(f(12.)).text_color(c(look.tertiary)).child(item.key))
}

/// A closed item's row: teal tick, struck-through or plain title, Reopen.
fn closed_row(look: Look, key: &'static str, title: &'static str, how: &'static str, when: &'static str, reopen: bool, lead: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(m(11.))
        .px(m(8.))
        .py(m(7.))
        .min_h(m(50.))
        .rounded(m(9.))
        .relative()
        .when(!lead, |row| row.child(div().absolute().top_0().left_0().right_0().h(px(1.)).bg(faint_line(look))))
        .child(closed_circle(look))
        .child(div().w(m(17.)).flex_shrink_0())
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(div().text_size(f(14.)).font_weight(FontWeight::SEMIBOLD).text_color(soft(look)).child(title))
                .child(div().mt(m(3.)).text_size(f(12.)).text_color(c(look.secondary)).child(how)),
        )
        .when(reopen, |row| row.child(mini(look, "Reopen").min_w(m(64.)).flex().justify_center()))
        .child(div().w(m(110.)).flex_shrink_0().flex().justify_end().text_size(f(12.)).text_color(c(look.secondary)).child(when))
        .child(div().w(m(52.)).flex_shrink_0().flex().justify_end().text_size(f(12.)).text_color(c(look.tertiary)).child(key))
}

// ---- Sections, folds, add field

fn columns_head(look: Look) -> Div {
    div()
        .flex()
        .items_baseline()
        .text_size(f(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(c(look.secondary))
        .child(div().w(m(110.)).text_color(soft(look)).child("Effort · Impact"))
        .child(div().w(m(128.)).child("Channel"))
        .child(div().w(m(52.)))
}

fn section(look: Look, title: &'static str, note: Option<&'static str>, head: bool, open: Option<bool>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(m(8.))
        .px(m(8.))
        .pt(m(14.))
        .pb(m(4.))
        .text_size(f(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(look.secondary))
        .children(open.map(|open| div().w(px(5.)).flex_shrink_0().ml(px(-3.)).child(small_icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, look.secondary))))
        .child(title)
        .children(note.map(|note| div().font_weight(FontWeight::NORMAL).child(note)))
        .child(div().flex_1())
        .when(head, |s| s.child(columns_head(look)))
}

fn fold(look: Look, title: &'static str, text: &'static str, top_gap: bool) -> Div {
    div()
        .mx(m(8.))
        .mt(if top_gap { m(6.) } else { px(0.) })
        .py(m(9.))
        .border_t_1()
        .border_color(rule(look))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_size(f(13.))
        .text_color(c(look.secondary))
        .overflow_hidden()
        .whitespace_nowrap()
        .child(div().w(px(6.)).flex_shrink_0().child(small_icon(IconName::ChevronRight, look.secondary).ml(px(-4.))))
        .child(div().flex_shrink_0().font_weight(FontWeight::SEMIBOLD).text_color(soft(look)).child(title))
        .child(div().min_w_0().overflow_hidden().text_ellipsis().child(text))
}

fn add_field(look: Look, text: &'static str) -> Div {
    div()
        .mx(m(8.))
        .mt(m(8.))
        .mb(m(4.))
        .px(m(12.))
        .py(m(9.))
        .rounded(m(9.))
        .border_1()
        .border_color(rule(look))
        .flex()
        .items_center()
        .text_size(f(13.))
        .text_color(c(look.secondary))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().child(text))
        .child(kbd_box(look, "N"))
}

const ADD: &str = "Add an item… try “fix login redirect fri #bug”";

fn folds(look: Look) -> Div {
    div()
        .child(fold(look, "Waiting on another item · 2", "PAY-9 Backfill progress bar waits on PAY-7 · WEB-13 waits on PAY-3", true))
        .child(fold(look, "Hidden until a date · 3", "Next to come back: “Check the Stripe fee change” on Mon 23 Nov", false))
}

/// What the list rows pick out as open in the drawer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    None,
    Pay3,
    Inf2,
    Suggestion,
    PullRequest,
}

/// The main list: what an agent wants to change, what is in progress, then
/// everything else ranked.
fn main_list(look: Look, narrow: bool, open: Open, add: Div) -> Div {
    let attention = |item: Item, question, selected, lead| item_row(look, item, Mark { attention: Some(question), selected, lead, ..Mark::default() }, narrow);
    let ranked = |rank, item: Item, selected, lead| item_row(look, item, Mark { rank: Some(rank), selected, lead, ..Mark::default() }, narrow);
    div()
        .flex()
        .flex_col()
        .child(add)
        .child(section(look, "NEEDS ATTENTION · 3", (!narrow).then_some("an agent wants to change these; open one to say yes or no"), !narrow, None))
        .child(attention(PAY3, "close?", open == Open::Pay3, true))
        .child(attention(INF2, "new due date?", open == Open::Inf2, open == Open::Pay3))
        .child(attention(IDEMPOTENCY, "add?", open == Open::Suggestion, open == Open::Inf2))
        .child(section(look, "IN PROGRESS · 1", None, false, None))
        .child(item_row(look, WEB8, Mark { lead: true, ..Mark::default() }, narrow))
        .child(section(look, "NEXT · 5", (!narrow).then_some("ranked for you; the order holds still while you look"), false, None))
        .child(ranked(1, INF6, false, true))
        .child(ranked(2, PAY7, false, false))
        .child(ranked(3, PR_REVIEW, open == Open::PullRequest, false))
        .child(ranked(4, PAY5, false, open == Open::PullRequest))
        .child(ranked(5, TD3, false, false))
        .child(folds(look))
}

// ---- Page chrome

fn top_bar(look: Look, sub: &'static str, filters: Option<&'static str>, next: &'static str, next_dim: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(m(12.))
        .h(m(60.))
        .px(m(28.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rule(look))
        .child(div().text_size(f(17.)).font_weight(FontWeight::BOLD).child("To-do"))
        .child(div().text_size(f(13.)).text_color(c(look.secondary)).child(sub))
        .child(spacer())
        .child(
            div()
                .w(m(240.))
                .flex_shrink_0()
                .px(m(10.))
                .py(m(6.))
                .rounded(m(7.))
                .border_1()
                .border_color(rule(look))
                .flex()
                .items_center()
                .justify_between()
                .text_size(f(13.))
                .text_color(c(look.secondary))
                .child("Search")
                .child(kbd_box(look, "/")),
        )
        .child(btn(look, "Sort: Ranked ⌄", false))
        .child(match filters {
            Some(text) => btn(look, text, false).bg(selected_bg(look)).border_color(c(look.accent)).text_color(c(look.accent)),
            None => btn(look, "Filters", false),
        })
        .child(
            btn(look, "Do next", true)
                .flex()
                .items_baseline()
                .when(next_dim, |b| b.bg(Hsla::from(c(look.accent)).opacity(0.4)).border_color(Hsla::transparent_black()))
                .child(div().text_size(px(12.)).font_weight(FontWeight::NORMAL).opacity(0.8).child(format!("\u{00a0}·\u{00a0}{next}"))),
        )
}

pub(crate) fn filter_chip(look: Look, text: &'static str, count: &'static str, on: bool) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .flex()
        .px(m(9.))
        .py(m(3.))
        .rounded(m(6.))
        .text_size(f(12.5))
        .when(!on, |chip| chip.bg(c(look.fill)).text_color(soft(look)))
        .when(on, |chip| chip.bg(c(look.text)).text_color(c(look.content)).font_weight(FontWeight::MEDIUM).pr(px(match text {
            "payments-rewrite" => 12.,
            "pr-reviews" | "No channel" => 11.,
            _ => 10.,
        })))
        .child(text)
        .child(div().ml(px(3.)).opacity(0.65).font_weight(FontWeight::NORMAL).child(count))
}

const CHANNELS: &[(&str, &str)] = &[("All", "10"), ("payments-rewrite", "4"), ("infra", "2"), ("web", "2"), ("pr-reviews", "1"), ("No channel", "1")];

fn show_closed(look: Look, on: bool) -> Div {
    if on {
        return div()
            .flex_shrink_0()
            .px(m(9.))
            .py(m(3.))
            .rounded(m(6.))
            .text_size(f(12.5))
            .font_weight(FontWeight::MEDIUM)
            .bg(tone(look, 0xdff5f1, 0x153a36))
            .text_color(tone(look, 0x0f5a54, 0x35c4b5))
            .child("Closed shown ✕");
    }
    div()
        .flex_shrink_0()
        .px(m(9.))
        .py(m(2.))
        .rounded(m(6.))
        .border_1()
        .border_dashed()
        .border_color(tone(look, 0xcfcfd5, 0x55555a))
        .text_size(f(12.5))
        .text_color(c(look.secondary))
        .child("Show closed")
}

fn channel_bar(look: Look, channels: &[(&'static str, &'static str)], on: &str, extra: Option<Div>, closed: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(m(6.))
        .pl(px(27.))
        .pr(m(28.))
        .pt(m(12.))
        .pb(m(4.))
        .flex_shrink_0()
        .children(channels.iter().map(|(name, count)| filter_chip(look, name, count, *name == on)))
        .children(extra)
        .child(spacer())
        .child(show_closed(look, closed))
}

/// The whole page: the sidebar with To-do selected, the top bar, the channel
/// bar, then the list with an optional drawer, and an overlay (popover or
/// strip) over the main column.
pub fn list_page(look: Look, inbox: Option<u32>, top: Div, bar: Div, list: Div, drawer: Option<Div>, overlay: Option<Div>) -> Div {
    div()
        .size_full()
        .flex()
        .line_height(relative(1.35))
        .child(crate::mock_sidebar::sidebar(look, "To-do", inbox, true))
        .child(handle(look, false))
        .child(
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .bg(c(look.content))
                .child(top)
                .child(bar)
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .overflow_hidden()
                        .child(div().flex_1().min_w_0().overflow_hidden().px(m(20.)).pt(m(6.)).child(list))
                        .children(drawer),
                )
                .children(overlay),
        )
}

// ---- The drawer

fn field_row(look: Look, label: &'static str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .min_h(m(31.))
        .text_size(f(13.))
        .child(div().w(m(96.)).flex_shrink_0().text_color(c(look.secondary)).child(label))
        .child(div().flex().items_center().gap(m(4.)).min_w_0().child(value))
}

pub(crate) fn segs(look: Look, options: &[&'static str], on: &str) -> Div {
    div()
        .flex()
        .flex_shrink_0()
        .p(px(2.))
        .rounded(m(7.))
        .bg(c(look.fill))
        .children(options.iter().map(|option| {
            div()
                .px(m(10.))
                .py(m(3.))
                .rounded(m(5.))
                .text_size(f(12.5))
                .text_color(soft(look))
                .when(*option == on, |s| s.py(px(2.)).bg(c(look.card)).border_1().border_color(edge(look)).text_color(c(look.text)).font_weight(FontWeight::SEMIBOLD))
                .child(*option)
        }))
}

fn input(look: Look, text: &'static str, hex: u32) -> Div {
    div().px(m(10.)).py(m(4.)).min_w(m(150.)).rounded(m(7.)).border_1().border_color(edge(look)).text_color(c(hex)).child(text)
}

fn hint(look: Look, text: &'static str) -> Div {
    div().ml(m(10.)).text_size(f(12.)).text_color(c(look.secondary)).child(text)
}

fn add_link(look: Look) -> Div {
    hint(look, "+ Add").text_color(c(look.accent))
}

fn heading(look: Look, text: &'static str, right: Option<&'static str>) -> Div {
    div()
        .flex()
        .justify_between()
        .mt(m(13.))
        .mb(m(5.))
        .text_size(f(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(c(look.secondary))
        .child(text)
        .children(right.map(|right| div().font_weight(FontWeight::NORMAL).child(right)))
}

fn description(look: Look, text: &'static str) -> Div {
    div().text_size(f(13.)).text_color(soft(look)).line_height(relative(1.5)).child(text)
}

fn session_chip(look: Look, state: St, name: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(m(6.))
        .px(m(8.))
        .py(m(2.))
        .rounded(m(6.))
        .bg(c(look.fill))
        .text_size(f(12.))
        .child(status(state))
        .child(name)
}

fn effort_impact(look: Look, effort: &str, impact: &str) -> Div {
    div()
        .flex()
        .items_center()
        .child(segs(look, &["S", "M", "L"], effort))
        .child(div().w(m(96.)).ml(m(34.)).text_color(c(look.secondary)).child("Impact"))
        .child(segs(look, &["Low", "Medium", "High"], impact))
}

fn step(look: Look, text: &'static str, done: bool, dashed: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(m(9.))
        .py(m(3.5))
        .text_size(f(13.))
        .when(done, |s| s.text_color(c(look.secondary)))
        .when(dashed, |s| s.text_color(c(look.secondary)))
        .child(
            div()
                .size(m(15.))
                .flex_shrink_0()
                .rounded(m(4.))
                .border_1()
                .when(dashed, |b| b.border_dashed())
                .border_color(if done { teal(look) } else { tone(look, 0xb9b9c0, 0x6a6a70) })
                .when(done, |b| b.bg(teal(look)))
                .flex()
                .items_center()
                .justify_center()
                .when(done, |b| b.child(Icon::new(IconName::Check).size(m(10.)).text_color(c(0xffffff)))),
        )
        .child(text)
}

fn steps(look: Look, done: &[&'static str], todo: &[&'static str], by: &'static str) -> Div {
    let total = done.len() + todo.len();
    let head = format!("STEPS · {} OF {}", done.len(), total);
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .justify_between()
                .mt(m(13.))
                .mb(m(5.))
                .text_size(f(11.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c(look.secondary))
                .child(head)
                .child(div().font_weight(FontWeight::NORMAL).child(by)),
        )
        .children(done.iter().map(|text| step(look, text, true, false)))
        .children(todo.iter().map(|text| step(look, text, false, false)))
        .child(step(look, "Add a step…", false, true))
}

/// The ask card: a question, the agent's reason, a grid of facts, and the
/// buttons that answer it.
fn ask_card(look: Look, question: &'static str, reason: &'static str, facts: Vec<(&'static str, Div)>, extra: Vec<Div>) -> Div {
    div()
        .mt(m(8.))
        .mb(m(2.))
        .px(m(14.))
        .py(m(11.))
        .rounded(m(10.))
        .bg(c(look.attention_bg))
        .flex()
        .flex_col()
        .child(div().text_size(f(14.5)).font_weight(FontWeight::SEMIBOLD).child(question))
        .child(div().mt(m(5.)).mb(m(7.)).mr(px(-4.)).text_size(f(13.)).text_color(tone(look, 0x3a2c12, 0xe8d6b0)).line_height(relative(1.4)).child(reason))
        .child(
            div()
                .mb(m(8.))
                .px(m(11.))
                .py(m(8.))
                .rounded(m(8.))
                .bg(c(look.card))
                .flex()
                .flex_col()
                .gap(px(3.))
                .text_size(f(13.))
                .children(facts.into_iter().map(|(label, value)| {
                    div().flex().child(div().w(m(74.)).flex_shrink_0().text_color(c(look.secondary)).child(label)).child(value.min_w_0())
                })),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(m(8.))
                .child(btn(look, "Yes", true).min_w(m(64.)).flex().justify_center())
                .child(btn(look, "No", false).min_w(m(64.)).flex().justify_center())
                .child(spacer())
                .children(extra),
        )
}

fn fact(text: impl Into<SharedString>) -> Div {
    div().child(text.into())
}

/// Text that wraps with some phrases bold or in a colour.
fn rich(text: &'static str, marks: &[(&str, bool, Option<Rgba>)]) -> Div {
    let mut spans: Vec<_> = marks
        .iter()
        .filter_map(|(phrase, bold, color)| {
            let start = text.find(phrase)?;
            let style = HighlightStyle {
                font_weight: bold.then_some(FontWeight::SEMIBOLD),
                color: color.map(Into::into),
                ..Default::default()
            };
            Some((start..start + phrase.len(), style))
        })
        .collect();
    spans.sort_by_key(|(range, _)| range.start);
    div().child(StyledText::new(text).with_highlights(spans))
}

fn drawer(look: Look, meta: &'static str, title: &'static str, body: Div, foot: Div) -> Div {
    div()
        .w(m(540.))
        .h_full()
        .flex_shrink_0()
        .border_l_1()
        .border_color(rule(look))
        .px(m(26.))
        .pt(m(18.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            div()
                .flex()
                .justify_between()
                .text_size(f(12.))
                .text_color(c(look.secondary))
                .child(meta)
                .child(div().flex().gap(m(10.)).child(small_icon(IconName::Maximize2, look.secondary)).child(small_icon(IconName::X, look.secondary))),
        )
        .child(div().mt(m(4.)).mb(m(8.)).text_size(f(19.)).font_weight(FontWeight::BOLD).child(title))
        .child(body.flex_1().min_h_0().overflow_hidden())
        .child(
            div()
                .flex_shrink_0()
                .border_t_1()
                .border_color(rule(look))
                .pt(m(12.))
                .pb(px(14.))
                .flex()
                .items_center()
                .gap(m(8.))
                .text_size(f(13.))
                .child(foot),
        )
}

fn quiet(look: Look, text: &'static str) -> Div {
    div().ml(px(21.)).text_color(c(look.secondary)).child(text)
}

fn pin_hide(look: Look) -> Div {
    div()
        .flex()
        .items_center()
        .child(div().flex().items_center().gap(px(4.)).text_color(c(look.secondary)).child(small_icon(IconName::Pin, look.secondary)).child("Pin"))
        .child(quiet(look, "Hide until…"))
}

fn start_foot(look: Look, label: &'static str) -> Div {
    div()
        .flex()
        .flex_1()
        .items_center()
        .child(btn(look, label, false))
        .child(spacer())
        .child(div().text_color(c(look.secondary)).child("History"))
        .child(div().ml(px(21.)).child(pin_hide(look)))
}

fn column(body: Div) -> Div {
    body.flex().flex_col()
}

fn pay3_body(look: Look) -> Div {
    column(div())
        .child(ask_card(
            look,
            "Close Ledger schema v2?",
            "Migration and backfill pass on staging: 3.1M rows in 4m12s. Steps 1 to 4 are done; step 5, the production cut-over, is yours.",
            vec![
                ("Evidence", rich("3 commits · acme/ledger#92 · 212 tests passed", &[("3 commits", false, Some(c(look.accent))), ("acme/ledger#92", false, Some(c(look.accent))), ("212 tests passed", false, Some(c(look.accent)))])),
                ("Change", rich("State: In progress → Closed", &[("Closed", true, None)])),
                ("Also", fact("WEB-13 stops waiting · the session keeps running")),
                ("Asked by", fact("claude in Ledger entries · 20m ago")),
            ],
            vec![btn(look, "Ask why", false), btn(look, "Open session", false)],
        ))
        .child(field_row(look, "State", segs(look, &["Open", "In progress", "Closed"], "In progress")))
        .child(field_row(look, "Channel", "# payments-rewrite"))
        .child(field_row(look, "Sessions", div().flex().items_center().child(session_chip(look, St::NeedsYou, "Ledger entries")).child(add_link(look))))
        .child(field_row(look, "Due", div().flex().items_center().child(input(look, "Fri 20 Nov", look.text)).child(hint(look, "tomorrow · or type “next week”"))))
        .child(field_row(look, "Effort", effort_impact(look, "M", "High")))
        .child(field_row(look, "Tags", div().flex().items_center().gap(m(4.)).child(tag(look, "migration")).child(tag(look, "db")).child(add_link(look))))
        .child(field_row(
            look,
            "Waits on",
            div().flex().items_center().child(pill(look, "PAY-2 Currency codes table ✓", tone(look, 0xdff5f1, 0x153a36), teal(look))).child(add_link(look)),
        ))
        .child(steps(
            look,
            &["Add ledger_entries and ledger_balances", "Dual-write from the posting path", "Backfill the old table in batches", "Verify the balances match on staging"],
            &["Cut production reads over"],
            "ticked by claude in Ledger entries",
        ))
        .child(heading(look, "DESCRIPTION", None))
        .child(description(look, "Split postings into ledger_entries and ledger_balances, backfill from the old table in batches, and keep both in step until cut-over."))
}

fn frame_top(look: Look) -> Div {
    top_bar(look, "10 open across 4 channels", None, "INF-6", false)
}

fn frame_bar(look: Look) -> Div {
    channel_bar(look, CHANNELS, "All", None, false)
}

fn plain_add(look: Look) -> Div {
    add_field(look, ADD)
}

// ---- Frames

/// b1-1: the list.
pub fn list(look: Look) -> Div {
    list_page(look, Some(2), frame_top(look), frame_bar(look), main_list(look, false, Open::None, plain_add(look)), None, None)
}

/// b1-2: an item open, with the agent's ask on top.
pub fn item_open(look: Look) -> Div {
    let foot = start_foot(look, "Start another session on this");
    list_page(
        look,
        Some(2),
        frame_top(look),
        frame_bar(look),
        main_list(look, true, Open::Pay3, plain_add(look)),
        Some(drawer(look, "PAY-3 · made by you, Mon 9 Nov", "Ledger schema v2", pay3_body(look), foot)),
        None,
    )
}

fn filter_row(look: Look, label: &'static str, chips: &[&'static str], on: &[&'static str]) -> Div {
    div()
        .flex()
        .items_start()
        .py(m(5.))
        .text_size(f(12.5))
        .when(!label.is_empty(), |row| row.child(div().w(m(84.)).flex_shrink_0().pt(m(3.)).text_color(c(look.secondary)).child(label)))
        .child(div().flex().flex_wrap().gap(m(5.)).flex_1().min_w_0().children(chips.iter().map(|chip| {
            let selected = on.contains(chip);
            div()
                .px(m(9.))
                .py(if selected { px(2.5) } else { px(3.) })
                .rounded(m(6.))
                .text_size(f(12.5))
                .when(!selected, |c2| c2.bg(c(look.fill)).text_color(soft(look)))
                .when(selected, |c2| c2.bg(c(look.text)).text_color(c(look.content)).font_weight(FontWeight::MEDIUM))
                .child(*chip)
        })))
}

fn filter_popover(look: Look) -> Div {
    div()
        .absolute()
        .right(m(28.))
        .top(m(54.))
        .w(m(440.))
        .px(m(18.))
        .pt(m(16.))
        .pb(px(8.))
        .rounded(m(12.))
        .bg(c(look.card))
        .border_1()
        .border_color(rule(look))
        .shadow_lg()
        .flex()
        .flex_col()
        .child(heading(look, "DUE", None).mt(px(0.)))
        .child(filter_row(look, "", &["Overdue", "This week", "After this week", "No date"], &["Overdue", "This week"]))
        .child(heading(look, "TAGS", None))
        .child(filter_row(look, "", &["api", "bug", "db", "ledger", "migration", "refunds"], &[]))
        .child(heading(look, "MORE", None))
        .child(filter_row(look, "Session", &["Ledger entries", "Refund retries", "API rate limiter", "None yet"], &[]))
        .child(filter_row(look, "Effort", &["S", "M", "L"], &[]))
        .child(filter_row(look, "Impact", &["Low", "Medium", "High"], &[]))
        .child(filter_row(look, "Only", &["Needs attention", "Pull request reviews"], &[]))
        .child(
            div()
                .mt(m(12.))
                .pt(px(5.))
                .border_t_1()
                .border_color(rule(look))
                .flex()
                .items_center()
                .justify_between()
                .text_size(f(13.))
                .text_color(c(look.secondary))
                .child("3 of 4 items in payments-rewrite")
                .child(div().flex().items_center().gap(m(14.)).child(div().text_color(c(look.accent)).child("Reset")).child(btn(look, "Done", false).py(px(5.)))),
        )
}

/// b1-3: one channel filtered to overdue or this week, Filters open.
pub fn channel_due_soon(look: Look) -> Div {
    let extra = div()
        .ml(m(8.))
        .flex_shrink_0()
        .pl(px(8.))
        .pr(px(7.))
        .py(px(2.))
        .rounded(m(6.))
        .text_size(f(12.5))
        .border_1()
        .bg(c(look.card))
        .border_color(c(look.accent))
        .text_color(c(look.accent))
        .child("Due: overdue or this week ✕");
    let list = div()
        .flex()
        .flex_col()
        .child(add_field(look, "Add an item to payments-rewrite… try “fix login redirect fri #bug”"))
        .child(section(look, "NEEDS ATTENTION · 1", None, true, None))
        .child(item_row(look, PAY3, Mark { attention: Some("close?"), lead: true, ..Mark::default() }, false))
        .child(section(look, "NEXT · 2", Some("their places in the whole list are 2 and 4"), false, None))
        .child(item_row(look, PAY7, Mark { rank: Some(2), lead: true, ..Mark::default() }, false))
        .child(item_row(look, PAY5, Mark { rank: Some(4), ..Mark::default() }, false))
        .child(fold(
            look,
            "Left out by the due filter · 2",
            "PAY-9 Backfill progress bar · a suggested item with no date, “Idempotency keys for refund retries”",
            true,
        ));
    list_page(
        look,
        Some(2),
        top_bar(look, "4 open in payments-rewrite", Some("Filters · 1"), "PAY-7", false),
        channel_bar(look, CHANNELS, "payments-rewrite", Some(extra), false),
        list,
        None,
        Some(filter_popover(look)),
    )
}

/// b4-2: a field edit the agent asks for; the session that asked has ended.
pub fn field_edit_ask(look: Look) -> Div {
    let body = column(div())
        .child(ask_card(
            look,
            "Move the due date to Tue 24 Nov?",
            "The pgvector extension has to be rebuilt for Postgres 17 before the upgrade can run, and its 17 build isn't out until Monday. Steps 2 to 4 wait on it.",
            vec![
                ("Change", rich("Due: Tue 17 Nov → Tue 24 Nov", &[("Tue 24 Nov", true, None)])),
                ("Also", fact("It stops being overdue · nothing waits on this item")),
                ("Asked by", rich("claude in Upgrade Postgres 17 · 1h ago · session ended after asking", &[("session ended", true, None)])),
            ],
            vec![btn(look, "Resume the session to ask why", false)],
        ))
        .child(field_row(look, "State", segs(look, &["Open", "In progress", "Closed"], "In progress")))
        .child(field_row(look, "Channel", "# infra"))
        .child(field_row(
            look,
            "Sessions",
            div().flex().items_center().child(session_chip(look, St::Offline, "Upgrade Postgres 17")).child(hint(look, "session ended")).child(mini(look, "Resume").ml(m(8.))),
        ))
        .child(field_row(look, "Due", div().flex().items_center().child(input(look, "Tue 17 Nov", 0xa51d1d).text_color(red(look))).child(hint(look, "overdue by 2 days"))))
        .child(field_row(look, "Effort", effort_impact(look, "L", "High")))
        .child(field_row(look, "Tags", div().flex().items_center().gap(m(4.)).child(tag(look, "db")).child(add_link(look))))
        .child(field_row(look, "Waits on", div().flex().items_center().child(div().ml(m(0.)).text_size(f(12.)).text_color(c(look.secondary)).child("nothing")).child(add_link(look))))
        .child(steps(look, &["Dump and restore on a scratch box"], &["Rebuild pgvector for 17", "Upgrade staging", "Upgrade production"], "ticked by claude"))
        .child(heading(look, "DESCRIPTION", None))
        .child(description(look, "Move the staging and production clusters from Postgres 15 to 17 with pg_upgrade, extensions included."));
    list_page(
        look,
        Some(2),
        frame_top(look),
        frame_bar(look),
        main_list(look, true, Open::Inf2, plain_add(look)),
        Some(drawer(look, "INF-2 · made by you, Mon 2 Nov", "Postgres 17 upgrade", body, start_foot(look, "Start a session on this"))),
        None,
    )
}

/// b5-2: a line typed into the add field, read into its parts.
pub fn quick_add(look: Look) -> Div {
    let typed = div()
        .mx(m(8.))
        .mt(m(8.))
        .px(m(12.))
        .py(m(9.))
        .rounded_t(m(9.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.accent))
        .shadow(vec![BoxShadow { color: tone(look, 0xdbe8ff, 0x1d3358).into(), offset: point(px(0.), px(0.)), blur_radius: px(0.), spread_radius: px(3.), inset: false }])
        .flex()
        .items_center()
        .text_size(f(13.))
        .child(div().flex_1().child("fix login redirect loop fri #bug web s high").child(div().text_color(c(look.accent)).child("▏")).flex())
        .child(kbd_box(look, "↵"));
    let read = div()
        .mx(m(8.))
        .mb(m(6.))
        .px(m(12.))
        .py(m(9.))
        .rounded_b(m(9.))
        .border_1()
        .border_t_0()
        .border_color(rule(look))
        .bg(c(look.panel))
        .flex()
        .items_center()
        .gap(m(8.))
        .whitespace_nowrap()
        .overflow_hidden()
        .child(div().flex_shrink_0().text_size(f(14.)).font_weight(FontWeight::SEMIBOLD).child("Fix login redirect loop"))
        .child(pill(look, "Due tomorrow · Fri 20 Nov", c(look.attention_bg), tone(look, 0x8a4b00, 0xffcf85)).font_weight(FontWeight::MEDIUM))
        .child(tag(look, "bug"))
        .child(tag(look, "# web"))
        .child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .text_size(f(12.))
                .text_color(soft(look))
                .child(div().size(m(18.)).mr(m(6.)).flex().items_center().justify_center().rounded(m(5.)).bg(c(look.fill)).text_size(f(11.)).font_weight(FontWeight::SEMIBOLD).child("S"))
                .child("high"),
        )
        .child(spacer())
        .child(
            div()
                .flex()
                .flex_shrink_0()
                .text_size(f(12.5))
                .text_color(c(look.secondary))
                .child("It would be 2nd in your list · ")
                .child(div().font_weight(FontWeight::SEMIBOLD).text_color(soft(look)).child("Enter"))
                .child(" adds it · everything but the title is optional"),
        );
    list_page(look, Some(2), frame_top(look), frame_bar(look), main_list(look, false, Open::None, div().child(typed).child(read)), None, None)
}

const PAY9: Item = Item::new("PAY-9", "Backfill progress bar", "payments-rewrite")
    .due(Due::Plain, "Wed 2 Dec")
    .tags(&["ledger"])
    .size("M", "medium")
    .why(Why::Plain, "Waits on PAY-7 Refund retry with backoff", "");
const HIDDEN1: Item = Item::new("PAY-14", "Check the Stripe fee change", "payments-rewrite").tags(&["fees"]).size("S", "medium").why(Why::Plain, "Comes back Mon 23 Nov", "");
const HIDDEN2: Item = Item::new("PAY-11", "Re-run the ledger reconciliation after cut-over", "payments-rewrite")
    .tags(&["ledger"])
    .size("S", "high")
    .why(Why::Plain, "Comes back Tue 1 Dec", "");

/// b5-3: Waiting and Hidden unfolded, Show closed on.
pub fn waiting_hidden_closed(look: Look) -> Div {
    let open = |item, lead| item_row(look, item, Mark { lead, ..Mark::default() }, false);
    let list = div()
        .flex()
        .flex_col()
        .child(add_field(look, "Add an item to payments-rewrite… try “fix login redirect fri #bug”"))
        .child(section(look, "NEEDS ATTENTION · 2", None, true, None))
        .child(item_row(look, PAY3, Mark { attention: Some("close?"), lead: true, ..Mark::default() }, false))
        .child(item_row(look, IDEMPOTENCY, Mark { attention: Some("add?"), ..Mark::default() }, false))
        .child(section(look, "NEXT · 2", None, false, None))
        .child(item_row(look, PAY7, Mark { rank: Some(2), lead: true, ..Mark::default() }, false))
        .child(item_row(look, PAY5, Mark { rank: Some(4), ..Mark::default() }, false))
        .child(section(look, "WAITING ON ANOTHER ITEM · 1", Some("joins the list when what it waits on is closed"), false, Some(true)))
        .child(open(PAY9, true))
        .child(section(look, "HIDDEN UNTIL A DATE · 2", Some("each comes back on its date, without a notification"), false, Some(true)))
        .child(open(HIDDEN1, true))
        .child(open(HIDDEN2, false))
        .child(section(look, "CLOSED · 3 THIS WEEK", Some("reopen within a week; older ones are under History"), false, None))
        .child(closed_row(look, "PAY-2", "Currency codes table", "Closed by you", "Mon", true, true))
        .child(closed_row(look, "PAY-6", "Drop the legacy postings index", "You said yes to claude in Ledger entries", "Tue", true, false))
        .child(closed_row(look, "", "Review: Currency rounding fix", "Closed itself: you submitted your review of acme/api#471", "Tue", false, false));
    list_page(
        look,
        Some(2),
        top_bar(look, "4 open in payments-rewrite", None, "PAY-7", false),
        channel_bar(look, CHANNELS, "payments-rewrite", None, true),
        list,
        None,
        None,
    )
}

fn pr_line(look: Look, repo: &'static str, title: &'static str, stat: &'static str) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(m(10.))
        .py(m(5.))
        .text_size(f(13.))
        .child(tag(look, repo))
        .child(div().flex_1().min_w_0().child(title))
        .child(div().flex_shrink_0().text_size(f(12.5)).text_color(c(look.secondary)).child(stat))
}

/// b7-1: a pull request review item, opened.
pub fn pr_review_item(look: Look) -> Div {
    let card = div()
        .mt(m(8.))
        .mb(m(2.))
        .px(m(14.))
        .py(m(11.))
        .rounded(m(10.))
        .bg(tone(look, 0xf1effd, 0x2a2548))
        .flex()
        .flex_col()
        .child(div().text_size(f(14.5)).font_weight(FontWeight::SEMIBOLD).child("dana-k asked for your review 2 days ago"))
        .child(
            div()
                .mt(m(5.))
                .mb(m(9.))
                .text_size(f(13.))
                .text_color(tone(look, 0x2c2650, 0xd6d0f5))
                .line_height(relative(1.4))
                .child("“Reads go to the new ledger tables behind a flag, off by default. The web change only swaps the totals query. Second pair of eyes on the flag fallback, please.”"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .child(btn(look, "Start the review", true))
                .child(hint(look, "opens a session in pr-reviews with both pull requests laid out").ml(m(14.)).flex_1().min_w_0()),
        );
    let body = column(div())
        .child(card)
        .child(heading(look, "PULL REQUESTS · A STACK OF 2, REVIEWED TOGETHER", None))
        .child(pr_line(look, "acme/api#483", "Ledger reads behind a flag", "+205 −45 · 6 files · checks pass"))
        .child(pr_line(look, "acme/web#210", "Read totals from the ledger", "+71 −30 · 2 files · checks pass"))
        .child(div().h(m(6.)))
        .child(field_row(look, "Author", "dana-k"))
        .child(field_row(look, "Also asked", div().flex().items_center().child("sam-r").child(hint(look, "approved yesterday").ml(m(4.)))))
        .child(field_row(look, "Waiting", "2 days · asked Tue 17 Nov"))
        .child(field_row(look, "Channel", "# pr-reviews"))
        .child(field_row(look, "Rank", div().flex().items_center().child("3rd").child(hint(look, "it climbs the longer it waits on you"))))
        .child(heading(look, "THIS ITEM LOOKS AFTER ITSELF", None))
        .child(description(
            look,
            "It closes when you submit your review, when dana-k withdraws the request, or when both pull requests are merged or closed. Its title, author and pull requests come from GitHub, so there is nothing to edit here. You can pin it, hide it until a date, or hide it for good.",
        ));
    let foot = div()
        .flex()
        .flex_1()
        .items_center()
        .child(btn(look, "Open on GitHub ↗", false))
        .child(spacer())
        .child(pin_hide(look))
        .child(quiet(look, "Hide"));
    list_page(
        look,
        Some(2),
        frame_top(look),
        frame_bar(look),
        main_list(look, true, Open::PullRequest, plain_add(look)),
        Some(drawer(look, "Pull request review · from GitHub", "Review: Ledger reads behind a flag", body, foot)),
        None,
    )
}

/// b7-3: an agent-suggested item, opened.
pub fn suggested_item(look: Look) -> Div {
    let body = column(div())
        .child(ask_card(
            look,
            "Add this item to your list?",
            "While adding the retry I saw that a refund retried after a timeout can be sent twice: the provider may have accepted the first try. An idempotency key per refund stops that. It is separate from PAY-7, so I didn’t do it there.",
            vec![
                ("Evidence", rich("refunds/retry.rs:88 · a failing test that shows the double refund", &[("refunds/retry.rs:88", false, Some(c(look.accent))), ("a failing test that shows the double refund", false, Some(c(look.accent)))])),
                ("Change", rich("A new item in payments-rewrite · it would be 3rd in your list", &[("3rd", true, None)])),
                ("Asked by", fact("copilot in Refund retries · 35m ago")),
            ],
            vec![btn(look, "Ask why", false), btn(look, "Open session", false)],
        ))
        .child(field_row(look, "State", div().flex().items_center().child(segs(look, &["Open", "In progress", "Closed"], "Open")).child(hint(look, "once added"))))
        .child(field_row(look, "Channel", "# payments-rewrite"))
        .child(field_row(look, "Due", input(look, "No date", look.secondary)))
        .child(field_row(look, "Effort", effort_impact(look, "S", "High")))
        .child(field_row(look, "Tags", div().flex().items_center().gap(m(4.)).child(tag(look, "refunds")).child(add_link(look).ml(px(6.)))))
        .child(heading(look, "DESCRIPTION · AS THE AGENT WROTE IT", None))
        .child(description(look, "Send an idempotency key with every refund request, derived from the refund id, so that a retry after a timeout can’t pay out twice."))
        .child(
            div()
                .mt(m(14.))
                .text_size(f(12.5))
                .text_color(c(look.secondary))
                .line_height(relative(1.5))
                .child("You can change any field before saying yes. Until then it has no key and no place in the ranking, and Do next never picks it."),
        );
    let foot = div().text_color(c(look.secondary)).child("Say yes to add it, or no to drop it. The agent is told either way and isn’t waiting.");
    list_page(
        look,
        Some(2),
        frame_top(look),
        frame_bar(look),
        main_list(look, true, Open::Suggestion, plain_add(look)),
        Some(drawer(look, "Suggested · not in your list yet", "Idempotency keys for refund retries", body, foot)),
        None,
    )
}

/// b8-1: no items yet.
pub fn empty(look: Look) -> Div {
    let text = |size, hex, line: &'static str| div().max_w(m(520.)).text_size(f(size)).text_color(c(hex)).line_height(relative(1.5)).text_center().child(line);
    let body = div()
        .flex()
        .flex_col()
        .child(plain_add(look))
        .child(
            div()
                .pt(m(70.))
                .flex()
                .flex_col()
                .items_center()
                .gap(m(8.))
                .child(div().text_size(f(18.)).font_weight(FontWeight::BOLD).child("Nothing to do yet"))
                .child(text(13., if look.dark { 0xc4c4ca } else { 0x55555b }, "Type a title above and press Enter. A date, an effort and an impact are optional; the more an item has, the better the list can order itself."))
                .child(div().mt(m(8.)).child(text(13., look.secondary, "Agents can suggest items and changes. They show here marked Needs attention, and nothing joins your list until you say yes. Pull requests waiting for your review show up by themselves."))),
        );
    list_page(
        look,
        None,
        top_bar(look, "no items", None, "nothing yet", true),
        channel_bar(look, &[("All", "0")], "All", None, false),
        body,
        None,
        None,
    )
}

const BARE: &[(&str, &str, &str)] = &[
    ("TD-4", "Fix the login redirect loop", "Added 5 days ago"),
    ("TD-5", "Ask Dana about the ledger cut-over date", "Added 4 days ago"),
    ("TD-6", "Rate limiter: decide per-key or per-account", "Added 3 days ago"),
    ("TD-7", "Clean up the old staging buckets", "Added 2 days ago"),
    ("TD-8", "Write up the refund retry design", "Added yesterday"),
    ("TD-9", "Try the new pgbench flags", "Added today"),
];

/// b8-2: six title-only items, oldest first.
pub fn bare_list(look: Look) -> Div {
    let mut list = div().flex().flex_col().child(plain_add(look)).child(section(look, "NEXT · 6", None, true, None));
    for (index, (key, title, age)) in BARE.iter().enumerate() {
        let item = Item::new(key, title, "No channel").why(Why::Plain, age, "");
        list = list.child(item_row(look, item, Mark { rank: Some(index as u32 + 1), lead: index == 0, ..Mark::default() }, false));
    }
    let tip = div()
        .mx(m(8.))
        .mt(m(14.))
        .px(m(14.))
        .py(m(10.))
        .rounded(m(9.))
        .bg(c(look.panel))
        .flex()
        .gap(m(4.))
        .text_size(f(13.))
        .text_color(soft(look))
        .child("With titles only, the oldest comes first. Give any item a due date or an impact and it moves to where it belongs.")
        .child(lnk(look, "Dismiss"));
    list_page(
        look,
        None,
        top_bar(look, "6 open", None, "TD-4", false),
        channel_bar(look, &[("All", "6"), ("No channel", "6")], "All", None, false),
        list.child(tip),
        None,
        None,
    )
}

/// (title, channel, key, due, tags, effort, impact, reason)
type Big = (&'static str, &'static str, &'static str, Option<(Due, &'static str)>, &'static [&'static str], &'static str, &'static str, (Why, &'static str));

const SIXTY: &[Big] = &[
    ("Ledger balances drift check", "payments-rewrite", "PAY-20", Some((Due::Over, "Overdue · 3 days")), &["ledger"], "S", "high", (Why::Blue, "Pinned")),
    ("Refund webhook signature rotation", "infra", "INF-23", Some((Due::Soon, "Due today")), &["security"], "M", "high", (Why::Plain, "Open for 5 weeks")),
    ("Checkout totals rounding in JPY", "web", "WEB-26", Some((Due::Soon, "Due tomorrow")), &["checkout", "bug"], "L", "medium", (Why::Plain, "2 items wait on it")),
    ("Staging backup restore drill", "sdk", "SDK-29", Some((Due::Plain, "Mon 23 Nov")), &["ops"], "S", "high", (Why::Plain, "High impact")),
    ("Alert on backfill lag over 5 min", "billing", "BIL-32", Some((Due::Plain, "Tue 24 Nov")), &["ops"], "M", "medium", (Why::Plain, "Open for 3 weeks")),
    ("SDK: typed errors for refunds", "payments-rewrite", "PAY-35", Some((Due::Plain, "Fri 27 Nov")), &["sdk"], "L", "medium", (Why::Plain, "Small job")),
    ("Invoice table pagination", "payments-rewrite", "PAY-38", None, &["ui"], "S", "low", (Why::Plain, "High impact for a small job")),
    ("Move cron jobs to the scheduler", "infra", "INF-41", None, &["ops"], "M", "medium", (Why::Plain, "Open for 5 weeks")),
    ("Rate limiter metrics dashboard", "web", "WEB-44", None, &["api"], "L", "medium", (Why::Plain, "2 items wait on it")),
    ("Drop the legacy postings table", "sdk", "SDK-47", None, &["db"], "S", "high", (Why::Plain, "High impact")),
    ("Web: empty state for no invoices", "billing", "BIL-50", None, &["ui"], "M", "low", (Why::Plain, "Open for 3 weeks")),
    ("Vault policy for the ledger service", "payments-rewrite", "PAY-53", None, &["security"], "L", "medium", (Why::Plain, "Small job")),
];

/// b8-3: 60 items, the first groups folded to one line each.
pub fn sixty_items(look: Look) -> Div {
    let mut list = div()
        .flex()
        .flex_col()
        .child(fold(look, "Needs attention · 4", "2 to close · 1 new due date · 1 suggested item", false).mt(m(4.)).border_t_0())
        .child(fold(look, "In progress · 6", "5 sessions working · 1 session ended", false))
        .child(section(look, "NEXT · 48", Some("ranked for you"), true, None));
    for (index, (title, channel, key, due, tags, effort, impact, (kind, why))) in SIXTY.iter().enumerate() {
        let mut item = Item::new(key, title, channel).tags(tags).size(effort, impact).why(*kind, why, "");
        item.due = *due;
        list = list.child(item_row(look, item, Mark { rank: Some(index as u32 + 1), lead: index == 0, ..Mark::default() }, false));
    }
    let channels = [("All", "60"), ("payments-rewrite", "19"), ("infra", "14"), ("web", "11"), ("sdk", "7"), ("billing", "5"), ("pr-reviews", "3"), ("No channel", "1")];
    let strip = div()
        .absolute()
        .left_0()
        .right_0()
        .bottom_0()
        .h(m(40.))
        .px(m(28.))
        .bg(c(look.content))
        .border_t_1()
        .border_color(rule(look))
        .flex()
        .items_center()
        .gap(m(18.))
        .text_size(f(12.5))
        .text_color(c(look.secondary))
        .child(div().flex().child(div().font_weight(FontWeight::SEMIBOLD).text_color(soft(look)).child("1–12 of 48 ranked")).child(" · scroll for the rest"))
        .child("Needs attention 4 and In progress 6 are folded above")
        .child(spacer())
        .child("Waiting 4 · Hidden until a date 2")
        .child(div().flex().items_center().gap(px(4.)).child(kbd_box(look, "/")).child("to search"))
        .child(div().absolute().right(px(-24.)).top(m(-340.)).w(px(0.)));
    let thumb = div().absolute().right(px(4.)).top(m(118.)).w(px(6.)).h(m(150.)).rounded_full().bg(tone(look, 0xc9c9cf, 0x55555a));
    list_page(
        look,
        None,
        top_bar(look, "60 open across 7 channels", None, "PAY-20", false),
        channel_bar(look, &channels, "All", None, false),
        list,
        None,
        Some(div().absolute().size_full().child(strip).child(thumb)),
    )
}

/// b9-1: the day after migrating: old order, each saying where it was.
pub fn day_after_migration(look: Look) -> Div {
    const OLD: &[(&str, &str, &str, Why, &str)] = &[
        ("PAY-5", "API rate limiter", "payments-rewrite", Why::Blue, "Pinned · was VIP"),
        ("PAY-9", "Backfill progress bar", "payments-rewrite", Why::Plain, "Was 1st in To do"),
        ("PAY-7", "Refund retry with backoff", "payments-rewrite", Why::Plain, "Was 2nd in To do"),
        ("INF-6", "Rotate the staging database credentials", "infra", Why::Plain, "Was 1st in To do on infra"),
        ("WEB-11", "Fix the flaky cart end-to-end test", "web", Why::Plain, "Was 3rd in To do"),
        ("PAY-12", "Multi-currency ledgers", "payments-rewrite", Why::Plain, "Was 1st in Backlog"),
        ("INF-4", "Move CI to devbox runners", "infra", Why::Plain, "Was 2nd in Backlog"),
        ("WEB-12", "Dark mode for the invoices table", "web", Why::Plain, "Was 3rd in Backlog"),
    ];
    let banner = div()
        .mx(m(8.))
        .mt(px(4.))
        .mb(m(2.))
        .px(m(14.))
        .py(m(11.))
        .rounded(m(9.))
        .bg(selected_bg(look))
        .flex()
        .items_center()
        .gap(m(12.))
        .text_size(f(13.))
        .line_height(relative(1.5))
        .child(
            rich(
                "Your boards became this list yesterday. All 37 items are here, in the order you left them: In progress, then To do, then Backlog. Nothing has a date, effort or impact yet; add them where they matter and the order starts to follow.",
                &[("Your boards became this list yesterday.", true, None)],
            )
            .flex_1()
            .min_w_0(),
        )
        .child(lnk(look, "What changed"))
        .child(btn(look, "Got it", false));
    let first = Item::new("PAY-3", "Ledger schema v2", "payments-rewrite").on(&[(St::NeedsYou, "Ledger entries")], false);
    let second = Item::new("INF-2", "Postgres 17 upgrade", "infra").on(&[(St::Offline, "Upgrade Postgres 17")], true);
    let mut list = div()
        .flex()
        .flex_col()
        .child(plain_add(look))
        .child(banner)
        .child(section(look, "IN PROGRESS · 6", Some("2 of 6 shown"), true, None))
        .child(item_row(look, first, Mark { lead: true, ..Mark::default() }, false))
        .child(item_row(look, second, Mark::default(), false))
        .child(section(look, "NEXT · 20", Some("in your old order, until dates and impact say otherwise"), false, None));
    for (index, (key, title, channel, kind, why)) in OLD.iter().enumerate() {
        let item = Item::new(key, title, channel).why(*kind, why, "");
        list = list.child(item_row(look, item, Mark { rank: Some(index as u32 + 1), lead: index == 0, ..Mark::default() }, false));
    }
    let channels = [("All", "28"), ("payments-rewrite", "12"), ("infra", "8"), ("web", "6"), ("No channel", "2")];
    list_page(
        look,
        Some(1),
        top_bar(look, "28 open across 4 channels", None, "PAY-5", false),
        channel_bar(look, &channels, "All", None, false),
        list,
        None,
        None,
    )
}
