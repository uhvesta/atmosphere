//! The Agent inbox elsewhere (INBOXHOOKS): a session's own inbox panel,
//! Home, Settings › Agent access and the notification, drawn from the
//! signed-off mockups (`milestones/mockups/boardsimple/b3-3`, `b4-1`,
//! `b5-1`, `b6-1`, `b6-2`, `b9-2`) at their own 1600x950 size, 1x.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::mock_sidebar::sidebar;

// ---- Colours the mockups use

fn tone(look: Look, light: u32, dark: u32) -> Rgba {
    c(if look.dark { dark } else { light })
}

fn soft(look: Look) -> Rgba {
    tone(look, 0x55555b, 0xc4c4ca)
}

fn muted(look: Look) -> Rgba {
    tone(look, 0x7a7a80, 0x9a9aa1)
}

fn hair(look: Look) -> Rgba {
    tone(look, 0xe6e6ea, 0x343437)
}

fn sunken(look: Look) -> Rgba {
    tone(look, 0xf3f3f5, 0x2d2d30)
}

fn amber(look: Look) -> Rgba {
    tone(look, 0xc76a00, 0xffb340)
}

fn teal(look: Look) -> Rgba {
    tone(look, 0x0f766e, 0x35c4b5)
}

fn red(look: Look) -> Rgba {
    tone(look, 0xa51d1d, 0xff9088)
}

const BLUE: u32 = 0x0a66f5;
const GREEN_HEX: u32 = 0x28c840;
const GREY_HEX: u32 = 0xb0b0b6;

fn blue(look: Look) -> Rgba {
    tone(look, BLUE, 0x4a93ff)
}

// ---- Small parts

fn spacer() -> Div {
    div().flex_1().min_w_0()
}

fn dot(hex: u32) -> Div {
    div().size(px(8.)).rounded_full().flex_shrink_0().bg(c(hex))
}

fn mbtn(look: Look, text: &'static str, primary: bool) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(12.))
        .py(px(6.))
        .rounded(px(7.))
        .border_1()
        .text_size(px(13.))
        .font_weight(if primary { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
        .when(primary, |b| b.bg(c(BLUE)).border_color(c(BLUE)).text_color(c(0xffffff)))
        .when(!primary, |b| b.bg(tone(look, 0xffffff, 0x2d2d30)).border_color(tone(look, 0xd4d4da, 0x48484c)))
        .child(text)
}

fn mini(look: Look, text: &'static str) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(9.))
        .py(px(2.))
        .rounded(px(6.))
        .border_1()
        .border_color(tone(look, 0xd4d4da, 0x48484c))
        .bg(c(look.card))
        .text_color(c(look.text))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child(text)
}

fn tag(look: Look, text: &'static str) -> Div {
    div().flex_shrink_0().whitespace_nowrap().px(px(7.)).py(px(1.)).rounded(px(5.)).text_size(px(12.)).bg(sunken(look)).text_color(soft(look)).child(text)
}

fn chip(text: &'static str, bg: Rgba, fg: Rgba, weight: FontWeight) -> Div {
    div().flex_shrink_0().whitespace_nowrap().px(px(7.)).py(px(1.)).rounded(px(5.)).text_size(px(12.)).bg(bg).text_color(fg).font_weight(weight).child(text)
}

/// Text with phrases picked out: bold, or a colour.
fn rich(text: &'static str, marks: &[(&str, Option<FontWeight>, Option<Rgba>)]) -> StyledText {
    let mut spans: Vec<_> = marks
        .iter()
        .filter_map(|(phrase, weight, color)| {
            text.find(phrase).map(|start| (start..start + phrase.len(), HighlightStyle { font_weight: *weight, color: color.map(Into::into), ..Default::default() }))
        })
        .collect();
    spans.sort_by_key(|(range, _)| range.start);
    StyledText::new(text).with_highlights(spans)
}

/// Which row the sidebar lights, its Agent inbox count and whether the
/// Ledger entries dot is orange.
#[derive(Clone, Copy)]
struct Side(&'static str, Option<u32>, bool);

impl Side {
    fn draw(self, look: Look) -> Div {
        sidebar(look, self.0, self.1, self.2)
    }
}

// ---- Terminal lines

#[derive(Clone, Copy)]
enum T {
    N,
    M,
    G,
    A,
    R,
}

fn tc(look: Look, t: T) -> Rgba {
    match t {
        T::N => c(look.term_text),
        T::M => c(look.term_dim),
        T::G => c(0x0f8a3c),
        T::A => c(0xb86a00),
        T::R => c(0xc0392b),
    }
}

fn ln(look: Look, parts: &[(T, &'static str)]) -> Div {
    div()
        .h(px(20.6))
        .flex()
        .whitespace_nowrap()
        .children(parts.iter().map(|(t, text)| div().text_color(tc(look, *t)).child(*text)))
}

fn plain(look: Look, text: &'static str) -> Div {
    ln(look, &[(T::N, text)])
}

fn dim(look: Look, text: &'static str) -> Div {
    ln(look, &[(T::M, text)])
}

fn blank(look: Look) -> Div {
    ln(look, &[])
}

// ---- The session screen

fn rail(look: Look, on: &str, badge: Option<(&str, &'static str)>) -> Div {
    let items: [(&str, &[&'static str]); 6] = [
        ("Agent inbox", &["Agent", "inbox"]),
        ("Diff", &["Diff"]),
        ("Files", &["Files"]),
        ("To-do", &["To-do"]),
        ("Journal", &["Journal"]),
        ("Info", &["Info"]),
    ];
    div()
        .w(px(76.))
        .h_full()
        .flex_shrink_0()
        .border_l_1()
        .border_color(hair(look))
        .pt(px(62.))
        .flex()
        .flex_col()
        .gap(px(18.))
        .items_center()
        .children(items.into_iter().map(|(key, lines)| {
            let lit = key == on;
            let ink = if lit { blue(look) } else { soft(look) };
            div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(5.))
                .text_size(px(11.))
                .line_height(px(14.85))
                .text_color(ink)
                .child(div().w(px(18.)).h(px(16.)).rounded(px(4.)).border_1().border_color(if lit { blue(look) } else { tone(look, 0x6a6a70, 0x8a8a90) }))
                .child(div().flex().flex_col().items_center().children(lines.iter().map(|line| div().child(*line))))
                .children(badge.filter(|(name, _)| *name == key).map(|(_, count)| {
                    div()
                        .absolute()
                        .top(px(-7.))
                        .right(px(2.))
                        .px(px(5.))
                        .rounded(px(8.))
                        .bg(c(BLUE))
                        .text_color(c(0xffffff))
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(count)
                }))
        }))
}

fn header(look: Look, crumb: &'static str, state: u32, name: &'static str, agent: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .h(px(60.))
        .px(px(18.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(hair(look))
        .child(
            div()
                .flex()
                .flex_col()
                .child(div().text_size(px(12.)).text_color(blue(look)).child(crumb))
                .child(div().flex().items_center().gap(px(8.)).text_size(px(15.)).font_weight(FontWeight::BOLD).child(dot(state)).child(name)),
        )
        .child(spacer())
        .child(div().text_size(px(13.)).text_color(muted(look)).child(format!("{agent}\u{a0} \u{2442}\u{a0} \u{a0}\u{2922}\u{a0} \u{a0}\u{22ef}")))
}

fn tabs(look: Look, other: &'static str, first: &'static str) -> Div {
    let tab = |lead: Option<AnyElement>, text: String, on: bool| {
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(14.))
            .py(px(9.))
            .border_r_1()
            .border_color(hair(look))
            .text_size(px(13.))
            .when(on, |t| t.bg(c(look.content)))
            .children(lead)
            .child(text)
            .when(on, |t| t.child(div().absolute().left(px(0.)).right(px(0.)).bottom(px(0.)).h(px(2.)).bg(blue(look))))
    };
    div()
        .flex()
        .flex_shrink_0()
        .bg(tone(look, 0xfafafb, 0x252527))
        .border_b_1()
        .border_color(hair(look))
        .child(tab(Some(dot(GREEN_HEX).into_any_element()), format!("{first} \u{2715}"), true))
        .child(tab(None, format!(">_ {other} \u{2715}"), false))
        .child(tab(None, "+".to_string(), false))
}

fn terminal(look: Look, lines: Vec<Div>) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(c(look.term))
        .font_family(MONO)
        .text_size(px(12.5))
        .child(div().flex_1().min_h_0().overflow_hidden().px(px(18.)).pt(px(16.)).flex().flex_col().children(lines.into_iter().enumerate().map(|(i, l)| {
            // 12.5px x 1.65 = 20.625px a line; whole device pixels would round each line down.
            let edge = |n: f32| (n * 20.625 * 2.).round() / 2.;
            l.h(px(edge(i as f32 + 1.) - edge(i as f32)))
        })))
        .child(
            div()
                .mx(px(18.))
                .mb(px(8.))
                .px(px(10.))
                .py(px(7.))
                .rounded(px(6.))
                .border_1()
                .border_color(tone(look, 0xcfcfd5, 0x48484c))
                .line_height(px(15.))
                .child("> ▌"),
        )
        .child(div().mx(px(18.)).mb(px(12.)).text_size(px(12.)).text_color(muted(look)).child("▸▸ accept edits on (shift+tab to cycle)"))
}

/// A panel's header: bold `title`, then `note`, then the expand and close marks.
fn panel_head(look: Look, title: Div, note: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(44.))
        .px(px(14.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(hair(look))
        .child(title)
        .child(div().text_size(px(13.)).text_color(muted(look)).child(note))
        .child(spacer())
        .child(div().text_color(muted(look)).child("\u{2922}\u{a0} \u{2715}"))
}

fn bold_title(text: &'static str) -> Div {
    div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(text)
}

#[allow(clippy::too_many_arguments)]
fn session_screen(
    look: Look,
    side: Side,
    crumb: &'static str,
    name: &'static str,
    agent: &'static str,
    state: u32,
    tab_names: (&'static str, &'static str),
    lines: Vec<Div>,
    head: Div,
    body: Div,
    on: &str,
    badge: Option<(&str, &'static str)>,
) -> Div {
    div()
        .size_full()
        .flex()
        .line_height(relative(1.35))
        .bg(c(look.content))
        .child(side.draw(look))
        .child(div().flex_1().min_w_0().h_full().flex().flex_col().child(header(look, crumb, state, name, agent)).child(tabs(look, tab_names.1, tab_names.0)).child(terminal(look, lines)))
        .child(div().w(px(520.)).h_full().flex_shrink_0().flex().flex_col().border_l_1().border_color(hair(look)).text_size(px(13.)).child(head).child(body))
        .child(rail(look, on, badge))
}

// ---- b3-3

fn cl(look: Look, text: &'static str, top: f32) -> Div {
    div().mt(px(top)).mb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(muted(look)).child(text)
}

/// b3-3: a session with its Agent inbox panel.
pub fn session_panel(look: Look) -> Div {
    let lines = vec![
        ln(look, &[(T::M, "~/src/api (refund-retries) $"), (T::N, " copilot")]),
        plain(look, "> Make refunds retry with backoff (PAY-7)."),
        blank(look),
        plain(look, "● Read(api/src/refunds/retry.rs)"),
        dim(look, "  ⎿ Read 148 lines"),
        plain(look, "● Edit(api/src/refunds/retry.rs)"),
        dim(look, "  ⎿ Added backoff with jitter, 5 attempts"),
        plain(look, "● Bash(cargo test -p api refunds)"),
        ln(look, &[(T::G, "  ⎿ test result: ok. 31 passed; 0 failed")]),
        blank(look),
        plain(look, "● The retry path picks a currency for the refund."),
        plain(look, "  The charge stores two, and they differ when"),
        plain(look, "  the rate has moved. I need you to choose."),
        blank(look),
        ln(look, &[(T::A, "◐ Asked you in the Agent inbox · waiting 6m")]),
        dim(look, "  You can also answer here."),
    ];
    let ink = tone(look, 0x2c2c30, 0xe0e0e4);
    let say = div()
        .line_height(relative(1.5))
        .text_color(ink)
        .child("“The charge stores two currencies: the one the customer paid in and the one you were paid in. Today a refund goes out in yours, so the customer gets less back when the rate has moved. Which should a retried refund use?”")
        .child(div().text_color(muted(look)).child("Working on PAY-7 Refund retry with backoff"));
    let field = div()
        .w_full()
        .mt(px(4.))
        .flex()
        .justify_between()
        .px(px(10.))
        .py(px(6.))
        .rounded(px(7.))
        .border_1()
        .border_color(tone(look, 0xd9d9de, 0x48484c))
        .text_color(muted(look))
        .child("Or type an answer…")
        .child("↵");
    let card = div()
        .px(px(14.))
        .py(px(13.))
        .rounded(px(12.))
        .border_2()
        .border_color(c(0xf0b35a))
        .bg(c(look.card))
        .flex()
        .flex_col()
        .child(div().flex().items_center().gap(px(10.)).child(dot(0xff9500)).child(div().text_size(px(14.5)).font_weight(FontWeight::BOLD).child("Which currency should a retried refund use?")))
        .child(div().mt(px(10.)).flex().flex_col().child(cl(look, "WHAT IT NEEDS TO KNOW", 0.)).child(say))
        .child(div().h(px(10.)))
        .child(cl(look, "YOUR ANSWER", 10.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .items_start()
                .child(mbtn(look, "The currency the customer paid in", false))
                .child(mbtn(look, "The currency I was paid in", false))
                .child(mbtn(look, "Leave it as it is and add a to-do", false))
                .child(field),
        )
        .child(
            div()
                .mt(px(14.))
                .pt(px(12.))
                .border_t_1()
                .border_color(hair(look))
                .text_size(px(12.5))
                .text_color(muted(look))
                .child("copilot is stopped until you answer · asked 6m ago"),
        );
    let foot = div()
        .mt(px(14.))
        .text_size(px(12.5))
        .line_height(relative(1.5))
        .text_color(muted(look))
        .child(rich("Only this session and its forks are listed here. Everything waiting on you is in Agent inbox.", &[("Agent inbox", None, Some(blue(look)))]));
    let head = panel_head(look, bold_title("Agent inbox"), "this session · 1 waiting on you");
    let body = div().p(px(14.)).child(card).child(foot);
    session_screen(
        look,
        Side("Refund retries", Some(1), false),
        "# payments-rewrite ›",
        "Refund retries",
        "Copilot · devbox-1",
        0xff9500,
        ("copilot", "zsh"),
        lines,
        head,
        body,
        "Agent inbox",
        Some(("Agent inbox", "1")),
    )
}

// ---- b4-1

fn sec(look: Look, text: &'static str) -> Div {
    div().px(px(8.)).pt(px(14.)).pb(px(4.)).text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(muted(look)).child(text)
}

fn check_circle(look: Look) -> Div {
    let ring = tone(look, 0xc9c9cf, 0x55555a);
    div().size(px(22.)).flex_shrink_0().rounded_full().border_1().border_color(ring).flex().items_center().justify_center().child(Icon::new(IconName::Check).size(px(12.)).text_color(ring))
}

fn key(look: Look, text: &'static str) -> Div {
    div().w(px(52.)).flex_shrink_0().flex().justify_end().text_size(px(12.)).text_color(tone(look, 0xa0a0a6, 0x77777e)).child(text)
}

/// One row of the session's To-do panel.
fn todo_row(look: Look, lead: Div, line1: Vec<Div>, line2: Vec<Div>, id: &'static str, first: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .py(px(7.))
        .min_h(px(50.))
        .relative()
        .when(!first, |r| r.child(div().absolute().top(px(-1.)).left(px(0.)).right(px(0.)).h(px(1.)).bg(tone(look, 0xf0f0f3, 0x2a2a2d))))
        .child(lead)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().flex().items_center().gap(px(7.)).whitespace_nowrap().overflow_hidden().children(line1))
                .child(div().mt(px(3.)).flex().items_center().gap(px(6.)).whitespace_nowrap().overflow_hidden().text_size(px(12.)).text_color(muted(look)).children(line2)),
        )
        .when(!id.is_empty(), |r| r.child(key(look, id)))
}

fn title(text: &'static str) -> Div {
    div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).whitespace_nowrap().overflow_hidden().text_ellipsis().child(text)
}

fn why(look: Look, text: &'static str) -> Div {
    div().flex_shrink_0().px(px(6.)).h(px(17.)).flex().items_center().rounded(px(5.)).text_size(px(11.5)).border_1().border_color(tone(look, 0xdcdce2, 0x4c4c51)).text_color(soft(look)).child(text)
}

/// b4-1: a session after the owner said yes.
pub fn after_yes(look: Look) -> Div {
    let lines = vec![
        ln(look, &[(T::M, "~/src/ledger (ledger-schema) $"), (T::N, " claude")]),
        plain(look, "> Design the ledger schema from PAY-3 and move the reads over."),
        blank(look),
        plain(look, "● Bash(cargo test -p api ledger)"),
        ln(look, &[(T::G, "  ⎿ test result: ok. 212 passed; 0 failed")]),
        plain(look, "● Bash(psql -f backfill.sql)"),
        dim(look, "  ⎿ INSERT 0 3104882 · 4m12s"),
        plain(look, "● humata todo close PAY-3"),
        plain(look, "    --reason \"Migration and backfill pass on staging;"),
        plain(look, "              steps 1 to 4 are done.\""),
        plain(look, "    --evidence 3 commits, acme/ledger#92, 212 tests"),
        dim(look, "  ⎿ Asked the owner. Not waiting; carrying on."),
        blank(look),
        plain(look, "● Writing the cut-over runbook while that waits."),
        plain(look, "● Write(ledger/docs/cutover.md)"),
        dim(look, "  ⎿ Wrote 64 lines"),
        blank(look),
        ln(look, &[(T::G, "✓ The owner said yes · PAY-3 Ledger schema v2 is closed")]),
        dim(look, "  WEB-13 no longer waits on it."),
        blank(look),
        plain(look, "● PAY-3 is closed. The runbook for step 5 is in"),
        plain(look, "  docs/cutover.md for when you do the cut-over."),
    ];
    let add = div()
        .mx(px(8.))
        .mt(px(8.))
        .mb(px(4.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(9.))
        .border_1()
        .border_color(hair(look))
        .flex()
        .justify_between()
        .text_color(muted(look))
        .child("Add an item to payments-rewrite…")
        .child(div().px(px(5.)).rounded(px(4.)).border_1().border_color(tone(look, 0xd9d9de, 0x48484c)).bg(tone(look, 0xffffff, 0x2d2d30)).text_size(px(11.5)).child("N"));
    let closed = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(8.))
        .py(px(7.))
        .min_h(px(50.))
        .rounded(px(9.))
        .bg(tone(look, 0xeaf7ee, 0x17301f))
        .child(div().size(px(22.)).flex_shrink_0().rounded_full().bg(teal(look)).flex().items_center().justify_center().child(Icon::new(IconName::Check).size(px(12.)).text_color(c(0xffffff))))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().text_size(px(14.)).text_color(soft(look)).font_weight(FontWeight::BOLD).line_through().child("Ledger schema v2"))
                .child(
                    div()
                        .mt(px(3.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(12.))
                        .text_color(muted(look))
                        .child("Closed just now · you said yes to claude")
                        .child(mini(look, "Reopen")),
                ),
        )
        .child(key(look, "PAY-3"));
    let attn = todo_row(
        look,
        div().size(px(22.)).flex_shrink_0(),
        vec![title("Idempotency keys for re…").flex_shrink(1.), chip("Needs attention · add?", tone(look, 0xffe9c2, 0x5b3f0c), tone(look, 0x7a4300, 0xffd995), FontWeight::SEMIBOLD), tag(look, "refunds")],
        vec![why(look, "Suggested"), div().child("by copilot in Refund retries · not in your list until you say yes")],
        "",
        true,
    );
    let over = chip("Overdue · 1 day", tone(look, 0xfde4e4, 0x4b1f1f), red(look), FontWeight::SEMIBOLD);
    let pay7 = todo_row(
        look,
        check_circle(look),
        vec![title("Refund retry with backoff"), over, tag(look, "refunds"), tag(look, "bug")],
        vec![why(look, "High impact for a small job"), div().child("PAY-9 waits on it")],
        "PAY-7",
        true,
    );
    let pay5 = todo_row(
        look,
        check_circle(look),
        vec![title("API rate limiter"), div().text_size(px(12.)).text_color(muted(look)).child("0/3"), tag(look, "Mon 23 Nov"), tag(look, "api")],
        vec![why(look, "High impact")],
        "PAY-5",
        false,
    );
    let fold = div()
        .mx(px(8.))
        .mt(px(3.5))
        .py(px(9.))
        .border_t_1()
        .border_color(hair(look))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_color(muted(look))
        .child("›")
        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(soft(look)).child("Waiting on another item · 1"))
        .child("PAY-9 waits on PAY-7");
    let list = div()
        .px(px(10.))
        .py(px(4.))
        .flex()
        .flex_col()
        .child(add)
        .child(sec(look, "THIS SESSION"))
        .child(closed)
        .child(sec(look, "NEEDS ATTENTION · 1"))
        .child(attn)
        .child(sec(look, "NEXT IN PAYMENTS-REWRITE · 2"))
        .child(pay7)
        .child(pay5)
        .child(fold)
        .child(div().px(px(8.)).py(px(10.)).text_size(px(12.5)).text_color(muted(look)).child(rich("Everything in every channel is in To-do.", &[("To-do", None, Some(blue(look)))])));
    let head = panel_head(look, bold_title("To-do"), "payments-rewrite · this session first");
    session_screen(
        look,
        Side("Ledger entries", None, false),
        "# payments-rewrite ›",
        "Ledger entries",
        "Claude Code · devbox-1",
        GREEN_HEX,
        ("claude", "zsh"),
        lines,
        head,
        list,
        "To-do",
        None,
    )
}

// ---- b5-1

fn page_top(look: Look, name: &'static str, sub: &'static str, right: Option<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .h(px(60.))
        .px(px(28.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(hair(look))
        .child(div().text_size(px(17.)).font_weight(FontWeight::BOLD).child(name))
        .child(div().text_size(px(13.)).text_color(muted(look)).child(sub))
        .child(spacer())
        .children(right)
}

fn page(look: Look, side: Side, top: Div, body: Div) -> Div {
    div()
        .size_full()
        .flex()
        .bg(c(look.content))
        .text_size(px(13.))
        .line_height(relative(1.35))
        .child(side.draw(look))
        .child(div().relative().flex_1().min_w_0().h_full().flex().flex_col().child(top).child(body.flex_1().min_h_0().overflow_hidden()))
}

#[derive(Clone, Copy)]
enum Mark {
    Working,
    Asking,
    Ended,
}

fn home_row(look: Look, mark: Mark, name: &'static str, meta: &'static str, sub: &'static str, amber_sub: bool, sub2: Option<&'static str>, time: &'static str, action: Option<Div>, first: bool) -> Div {
    let lead = match mark {
        Mark::Working => Some(dot(GREEN_HEX).into_any_element()),
        Mark::Asking => Some(dot(0xff9500).into_any_element()),
        Mark::Ended => Some(div().text_size(px(10.)).font_weight(FontWeight::BOLD).text_color(c(0xe0342b)).child("✕").into_any_element()),
    };
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(11.))
        .when(!first, |r| r.border_t_1().border_color(hair(look)))
        .children(lead.map(|lead| div().w(px(8.)).flex().justify_center().flex_shrink_0().child(lead)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(8.))
                        .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child(name))
                        .child(div().text_size(px(12.5)).text_color(muted(look)).child(meta)),
                )
                .child(div().mt(px(2.)).text_size(px(12.5)).text_color(if amber_sub { amber(look) } else { muted(look) }).when(amber_sub, |d| d.font_weight(FontWeight::MEDIUM)).child(sub))
                .children(sub2.map(|text| div().mt(px(2.)).text_size(px(12.5)).text_color(muted(look)).child(text))),
        )
        .child(div().text_size(px(12.5)).text_color(muted(look)).child(time))
        .children(action)
}

fn hsec(look: Look, text: &'static str, first: bool) -> Div {
    div().mt(px(if first { 0. } else { 18. })).mb(px(8.)).text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(soft(look)).child(text)
}

fn hbox(look: Look) -> Div {
    div().flex().flex_col().rounded(px(10.)).border_1().border_color(tone(look, 0xdcdce2, 0x3a3a3d))
}

/// b5-1: Home across every channel.
pub fn home(look: Look) -> Div {
    let attention = hbox(look).child(home_row(
        look,
        Mark::Asking,
        "Refund retries",
        "# payments-rewrite · devbox-1 · copilot",
        "Asks: Which currency should a retried refund use?",
        true,
        Some("Also suggested a to-do, “Idempotency keys for refund retries”. It waits in To-do and isn’t holding the agent up."),
        "6m",
        Some(mbtn(look, "Answer", true)),
        true,
    ));
    let ended = hbox(look).child(home_row(look, Mark::Ended, "Upgrade Postgres 17", "# infra · devbox-1 · claude", "Exited with code 1 after the dump step", false, None, "40m", Some(mbtn(look, "Resume", false)), true));
    let working = hbox(look)
        .child(home_row(look, Mark::Working, "Ledger entries", "# payments-rewrite · devbox-1 · claude", "Writing the cut-over runbook", false, None, "now", None, true))
        .child(home_row(look, Mark::Working, "Benchmark BRIN", "# infra · devbox-1 · codex", "pgbench run 2 of 4", false, None, "1m", None, false))
        .child(home_row(look, Mark::Working, "Ledger totals", "# web · This Mac · claude", "Running the checkout tests", false, None, "now", None, false))
        .child(home_row(look, Mark::Working, "Totals, another way", "# web · This Mac · copilot", "Comparing two table layouts", false, None, "3m", None, false));
    let todo = hbox(look).child(
        div()
            .flex()
            .items_center()
            .gap(px(12.))
            .px(px(16.))
            .py(px(11.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child("Next: Rotate the staging database credentials"))
                            .child(div().text_size(px(12.5)).text_color(muted(look)).child("INF-6 · pinned · due today")),
                    )
                    .child(div().mt(px(2.)).text_size(px(12.5)).text_color(muted(look)).child("10 open · 3 need attention when you get to them · 1 overdue")),
            )
            .child(mbtn(look, "Open To-do", false))
            .child(mbtn(look, "Do next", false)),
    );
    let body = div().flex().justify_center().child(
        div()
            .w(px(900.))
            .pt(px(22.))
            .child(hsec(look, "Stopped until you answer · 1", true))
            .child(attention)
            .child(hsec(look, "Ended · 1", false))
            .child(ended)
            .child(hsec(look, "Working · 4", false))
            .child(working)
            .child(hsec(look, "To-do", false))
            .child(todo),
    );
    page(look, Side("Home", Some(1), false), page_top(look, "Home", "", Some(mbtn(look, "New session", true))), body)
}

// ---- b6-1

fn seg(look: Look, options: &[&'static str], on: &str) -> Div {
    div().flex().flex_shrink_0().p(px(2.)).rounded(px(7.)).bg(sunken(look)).children(options.iter().map(|option| {
        let lit = *option == on;
        div()
            .px(px(10.))
            .py(px(2.))
            .rounded(px(5.))
            .border_1()
            .border_color(rgba(0x00000000))
            .text_size(px(12.5))
            .text_color(if lit { c(look.text) } else { soft(look) })
            .when(lit, |s| s.bg(c(look.card)).font_weight(FontWeight::SEMIBOLD).border_color(tone(look, 0xd9d9de, 0x48484c)).shadow_sm())
            .child(*option)
    }))
}

fn setting(look: Look, name: &'static str, sub: &'static str, on: &'static str, wh: &'static str, options: &[&'static str]) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .py(px(7.))
        .border_t_1()
        .border_color(tone(look, 0xf0f0f3, 0x2a2a2d))
        .child(div().flex_1().min_w_0().child(div().child(name)).when(!sub.is_empty(), |n| n.child(div().mt(px(1.)).text_size(px(12.)).text_color(muted(look)).child(sub))))
        .child(seg(look, options, on))
        .child(div().w(px(400.)).flex_shrink_0().text_size(px(12.5)).text_color(muted(look)).child(wh))
}

fn group(look: Look, name: &'static str, note: &'static str) -> Div {
    div()
        .mt(px(16.))
        .mb(px(6.))
        .flex()
        .items_baseline()
        .gap(px(10.))
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(soft(look)).child(name))
        .when(!note.is_empty(), |g| g.child(div().text_size(px(10.5)).text_color(muted(look)).child(note)))
}

/// b6-1: Settings › Agent access.
pub fn agent_access(look: Look) -> Div {
    const ALL: [&str; 3] = ["Allow", "Ask", "Never"];
    const TODO: &str = "Asks on the item in To-do; the agent carries on.";
    const INBOX: &str = "Asks in the Agent inbox; the agent stops and waits.";
    const TOLD: &str = "The agent is told no, with this rule named.";
    let tab = |text: &'static str, on: bool| {
        div()
            .px(px(9.))
            .py(px(3.))
            .rounded(px(6.))
            .text_size(px(12.5))
            .when(!on, |t| t.bg(sunken(look)).text_color(soft(look)))
            .when(on, |t| t.bg(tone(look, 0x1c1c1e, 0xe9e9ec)).text_color(tone(look, 0xffffff, 0x1e1e20)).font_weight(FontWeight::MEDIUM))
            .child(text)
    };
    let tabs = div().flex().items_center().gap(px(6.)).px(px(28.)).pt(px(10.)).pb(px(4.)).children(
        ["General", "Agents", "Agent access", "Machines", "Notifications"].into_iter().map(|t| tab(t, t == "Agent access")),
    );
    let set = div()
        .w(px(1120.))
        .px(px(28.))
        .py(px(6.))
        .child(group(look, "TO-DO", "never stops an agent and never notifies you"))
        .child(setting(look, "Tick steps and edit items it made", "", "Allow", "", &ALL))
        .child(setting(look, "Add an item", "It appears under Needs attention, marked Suggested", "Ask", TODO, &ALL))
        .child(setting(look, "Close or edit an item it didn’t make", "State, due date, effort, impact, tags, what it waits on", "Ask", TODO, &ALL))
        .child(setting(look, "Delete an item", "", "Never", TOLD, &ALL))
        .child(group(look, "FILES AND SESSIONS", "risky: the agent stops until you answer, and you get a notification"))
        .child(setting(look, "Move or copy files inside its own session", "", "Allow", "", &ALL))
        .child(setting(look, "Copy files from another session", "", "Ask", INBOX, &ALL))
        .child(setting(look, "Move files or its session to another machine", "Running terminals move along and keep running", "Ask", INBOX, &ALL))
        .child(setting(look, "Start or fork a session", "", "Ask", INBOX, &ALL))
        .child(setting(look, "End a session or a terminal", "Only you end them", "Never", TOLD, &ALL))
        .child(group(look, "ACCESS AND QUESTIONS", ""))
        .child(setting(look, "Read other channels’ to-dos and files", "", "Allow", "", &ALL))
        .child(setting(look, "Ask for access it doesn’t have", "", "Ask", INBOX, &["Ask", "Never"]))
        .child(setting(look, "Ask you a question", "", "Ask", INBOX, &["Ask"]))
        .child(
            div()
                .mt(px(14.))
                .px(px(14.))
                .py(px(11.))
                .rounded(px(9.))
                .bg(tone(look, 0xf5f5f8, 0x2a2a2d))
                .text_color(soft(look))
                .line_height(relative(1.5))
                .child("Shell commands, edits and pushes are asked by each agent in its own terminal, as that agent is set up. Humata doesn’t repeat those asks here. Rules you made with “Always allow” are listed under their row. A channel can override these in its Agent access tab."),
        );
    page(
        look,
        Side("Settings", None, true),
        page_top(look, "Settings", "", None),
        div().child(tabs).child(set),
    )
}

// ---- b6-2

const ORANGE_DOT: u32 = 0xff9500;

fn f_chip(look: Look, text: &'static str, count: &'static str, on: bool) -> Div {
    let (bg, fg) = if on { (tone(look, 0x1c1c1e, 0xe9e9ec), tone(look, 0xffffff, 0x1e1e20)) } else { (sunken(look), soft(look)) };
    div()
        .flex()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(9.))
        .py(px(3.))
        .rounded(px(6.))
        .bg(bg)
        .text_color(fg)
        .text_size(px(12.5))
        .when(on, |d| d.font_weight(FontWeight::MEDIUM))
        .child(text)
        .child(div().ml(px(4.)).opacity(0.65).child(count))
}

fn list_sec(look: Look, text: &'static str, note: &'static str, columns: bool) -> Div {
    div()
        .flex()
        .items_baseline()
        .gap(px(8.))
        .px(px(8.))
        .pt(px(14.))
        .pb(px(4.))
        .text_size(px(11.5))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(muted(look))
        .child(text)
        .when(!note.is_empty(), |d| d.child(div().font_weight(FontWeight::NORMAL).child(note)))
        .when(columns, |d| {
            d.child(spacer()).child(
                div()
                    .flex()
                    .font_weight(FontWeight::MEDIUM)
                    .text_size(px(12.))
                    .text_color(tone(look, 0x55555b, 0xc3c3c9))
                    .child(div().w(px(110.)).flex_shrink_0().child("Effort · Impact"))
                    .child(
                        div()
                            .w(px(128.))
                            .flex_shrink_0()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(muted(look))
                            .child("Channel"),
                    )
                    .child(div().w(px(52.)).flex_shrink_0()),
            )
        })
}

fn bright_check(look: Look) -> Div {
    let ring = c(0xc9c9cf);
    div().size(px(22.)).flex_shrink_0().rounded_full().border_1().border_color(ring).flex().items_center().justify_center().child(Icon::new(IconName::Check).size(px(12.)).text_color(ring))
}

fn due(look: Look, text: &'static str, kind: u8) -> Div {
    match kind {
        2 => chip(text, tone(look, 0xfde4e4, 0x4b1f1f), red(look), FontWeight::SEMIBOLD),
        1 => chip(text, tone(look, 0xfff4e0, 0x3b2b10), tone(look, 0x8a4b00, 0xffcf85), FontWeight::MEDIUM),
        _ => chip(text, sunken(look), soft(look), FontWeight::NORMAL),
    }
}

fn na(look: Look, text: &'static str) -> Div {
    chip(text, tone(look, 0xffe9c2, 0x5b3f0c), tone(look, 0x7a4300, 0xffd995), FontWeight::SEMIBOLD)
}

fn st(look: Look, text: &'static str) -> Div {
    div().flex_shrink_0().text_size(px(12.)).text_color(muted(look)).child(text)
}

fn ses(look: Look, hex: u32, text: &'static str, dim: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(8.))
        .py(px(2.))
        .rounded(px(6.))
        .bg(sunken(look))
        .text_size(px(12.))
        .text_color(if dim { tone(look, 0x8a8a90, 0x85858c) } else { muted(look) })
        .child(dot(hex))
        .child(text)
}

struct Item {
    check: bool,
    rank: &'static str,
    line1: Vec<Div>,
    line2: Vec<Div>,
    size: Option<(&'static str, &'static str)>,
    channel: &'static str,
    key: &'static str,
}

fn list_row(look: Look, item: Item, first: bool) -> Div {
    let Item { check, rank, line1, line2, size, channel, key } = item;
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(8.))
        .py(px(7.))
        .min_h(px(50.))
        .relative()
        .when(!first, |r| r.child(div().absolute().top(px(-1.)).left(px(0.)).right(px(0.)).h(px(1.)).bg(tone(look, 0xf0f0f3, 0x2a2a2d))))
        .child(if check { bright_check(look) } else { div().size(px(22.)).flex_shrink_0() })
        .child(div().w(px(18.)).flex_shrink_0().flex().justify_end().text_size(px(12.)).text_color(tone(look, 0xa0a0a6, 0x77777e)).child(rank))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().flex().items_center().gap(px(7.)).whitespace_nowrap().overflow_hidden().children(line1))
                .child(div().mt(px(3.)).flex().items_center().gap(px(6.)).whitespace_nowrap().overflow_hidden().text_size(px(12.)).text_color(muted(look)).children(line2)),
        )
        .child(div().w(px(110.)).flex_shrink_0().flex().items_center().text_size(px(12.)).text_color(soft(look)).children(size.map(|(letter, level)| {
            div()
                .flex()
                .items_center()
                .child(div().size(px(18.)).mr(px(6.)).rounded(px(5.)).bg(tone(look, 0xf3f3f5, 0x38383b)).flex().items_center().justify_center().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).child(letter))
                .child(level)
        })))
        .child(div().w(px(128.)).flex_shrink_0().text_size(px(12.)).text_color(muted(look)).child(channel))
        .child(div().w(px(52.)).flex_shrink_0().flex().justify_end().text_size(px(12.)).text_color(tone(look, 0xa0a0a6, 0x77777e)).child(key))
}

fn ti(look: Look, text: &'static str) -> Div {
    div().flex_shrink_0().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(tone(look, 0x1c1c1e, 0xf2f2f4)).child(text)
}

fn list_fold(look: Look, bold: &'static str, rest: &'static str, first: bool) -> Div {
    div()
        .mx(px(8.))
        .when(first, |d| d.mt(px(4.)))
        .py(px(9.))
        .border_t_1()
        .border_color(hair(look))
        .flex()
        .items_center()
        .gap(px(8.))
        .text_color(muted(look))
        .child("›")
        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(soft(look)).child(bold))
        .child(rest)
}

/// The To-do list page of b6-2, drawn here rather than shared, with the
/// mockup's own sidebar.
fn todo_page(look: Look) -> Div {
    let resume = div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(9.))
        .py(px(2.))
        .rounded(px(6.))
        .border_1()
        .border_color(tone(look, 0xd4d4da, 0x48484c))
        .bg(tone(look, 0xffffff, 0x2d2d30))
        .text_color(tone(look, 0x1c1c1e, 0xe9e9ec))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .child("Resume");
    let top = page_top(
        look,
        "To-do",
        "10 open across 4 channels",
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(div().w(px(240.)).flex().justify_between().px(px(10.)).py(px(6.)).rounded(px(7.)).bg(tone(look, 0xffffff, 0x2d2d30)).border_1().border_color(tone(look, 0xd9d9de, 0x48484c)).text_color(muted(look)).child("Search").child("/"))
                .child(mbtn(look, "Sort: Ranked ⌄", false))
                .child(mbtn(look, "Filters", false))
                .child(mbtn(look, "Do next · INF-6", true)),
        ),
    );
    let bar = div()
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(28.))
        .pt(px(12.))
        .pb(px(4.))
        .child(f_chip(look, "All", "10", true))
        .child(f_chip(look, "payments-rewrite", "4", false))
        .child(f_chip(look, "infra", "2", false))
        .child(f_chip(look, "web", "2", false))
        .child(f_chip(look, "pr-reviews", "1", false))
        .child(f_chip(look, "No channel", "1", false));
    let add = div()
        .mx(px(8.))
        .mt(px(8.))
        .mb(px(4.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(9.))
        .border_1()
        .border_color(hair(look))
        .flex()
        .justify_between()
        .text_color(muted(look))
        .child("Add an item… try “fix login redirect fri #bug”")
        .child(div().px(px(5.)).rounded(px(4.)).border_1().border_color(tone(look, 0xd9d9de, 0x48484c)).bg(tone(look, 0xffffff, 0x2d2d30)).text_size(px(11.5)).child("N"));
    let pinned = div().flex_shrink_0().px(px(6.)).h(px(17.)).flex().items_center().rounded(px(5.)).text_size(px(11.5)).border_1().border_color(tone(look, 0xb9d0fb, 0x2f5ea8)).text_color(tone(look, BLUE, 0x7fb2ff)).child("Pinned");
    let pr = chip("Pull request review", tone(look, 0xece9fd, 0x322c5c), tone(look, 0x4a2fa8, 0xcdc4ff), FontWeight::MEDIUM);
    let item = |check, rank, line1, line2, size, channel, key| Item { check, rank, line1, line2, size, channel, key };
    let list = div()
        .px(px(20.))
        .pt(px(6.))
        .line_height(relative(1.35))
        .child(add)
        .child(list_sec(look, "NEEDS ATTENTION · 3", "an agent wants to change these; open one to say yes or no", true))
        .child(list_row(look, item(true, "", vec![ti(look, "Ledger schema v2"), na(look, "Needs attention · close?"), st(look, "4/5"), due(look, "Due tomorrow", 1), tag(look, "migration"), tag(look, "db")], vec![ses(look, ORANGE_DOT, "Ledger entries", false)], Some(("M", "high")), "payments-rewrite", "PAY-3"), true))
        .child(list_row(look, item(true, "", vec![ti(look, "Postgres 17 upgrade"), na(look, "Needs attention · new due date?"), st(look, "1/4"), due(look, "Overdue · 2 days", 2), tag(look, "db")], vec![ses(look, GREY_HEX, "Upgrade Postgres 17", true), div().child("session ended"), resume], Some(("L", "high")), "infra", "INF-2"), false))
        .child(list_row(look, item(false, "", vec![ti(look, "Idempotency keys for refund retries"), na(look, "Needs attention · add?"), tag(look, "refunds")], vec![why(look, "Suggested"), div().child("by copilot in Refund retries · not in your list until you say yes")], Some(("S", "high")), "payments-rewrite", ""), false))
        .child(list_sec(look, "IN PROGRESS · 1", "", false))
        .child(list_row(look, item(true, "", vec![ti(look, "Checkout page shows the new ledger totals"), st(look, "2/3"), due(look, "Fri 27 Nov", 0), tag(look, "checkout")], vec![ses(look, GREEN_HEX, "Ledger totals", false), ses(look, GREEN_HEX, "Totals, another way", false)], Some(("M", "medium")), "web", "WEB-8"), true))
        .child(list_sec(look, "NEXT · 5", "ranked for you; the order holds still while you look", false))
        .child(list_row(look, item(true, "1", vec![ti(look, "Rotate the staging database credentials"), due(look, "Due today", 1), tag(look, "security")], vec![pinned], Some(("S", "medium")), "infra", "INF-6"), true))
        .child(list_row(look, item(true, "2", vec![ti(look, "Refund retry with backoff"), due(look, "Overdue · 1 day", 2), tag(look, "refunds"), tag(look, "bug")], vec![why(look, "High impact for a small job"), div().child("PAY-9 waits on it")], Some(("S", "high")), "payments-rewrite", "PAY-7"), false))
        .child(list_row(look, item(false, "3", vec![ti(look, "Review: Ledger reads behind a flag"), pr, tag(look, "acme/api#483"), tag(look, "acme/web#210"), st(look, "by dana-k")], vec![why(look, "Waiting on you for 2 days"), div().child("dana-k asked for your review")], None, "pr-reviews", ""), false))
        .child(list_row(look, item(true, "4", vec![ti(look, "API rate limiter"), st(look, "0/3"), due(look, "Mon 23 Nov", 0), tag(look, "api")], vec![why(look, "High impact")], Some(("M", "high")), "payments-rewrite", "PAY-5"), false))
        .child(list_row(look, item(true, "5", vec![ti(look, "Renew the Apple developer certificate"), due(look, "Mon 14 Dec", 0), tag(look, "admin")], vec![why(look, "High impact for a small job")], Some(("S", "high")), "No channel", "TD-3"), false))
        .child(list_fold(look, "Waiting on another item · 2", "PAY-9 Backfill progress bar waits on PAY-7 · WEB-13 waits on PAY-3", true))
        .child(list_fold(look, "Hidden until a date · 3", "Next to come back: “Check the Stripe fee change” on Mon 23 Nov", false));
    page(look, Side("To-do", Some(1), true), top, div().child(bar).child(list))
}

/// b6-2: the notification over the dark To-do list.
pub fn notification(look: Look) -> Div {
    let banner = div()
        .absolute()
        .right(px(14.))
        .top(px(12.))
        .w(px(372.))
        .line_height(relative(1.35))
        .flex()
        .gap(px(11.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(16.))
        .bg(rgba(0x3a3a3ef5))
        .border_1()
        .border_color(rgba(0xffffff1f))
        .text_color(c(0xf2f2f4))
        .shadow_lg()
        .child(
            div()
                .size(px(36.))
                .flex_shrink_0()
                .rounded(px(9.))
                .bg(c(0x3a7ae8))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(17.))
                .font_weight(FontWeight::BOLD)
                .text_color(c(0xffffff))
                .child("H"),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(div().flex().justify_between().text_size(px(12.)).text_color(c(0xb4b4ba)).child("HUMATA").child("now"))
                .child(div().mt(px(1.)).mb(px(2.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).line_height(relative(1.35)).child("codex is stopped: move Benchmark BRIN to gpu-box?"))
                .child(div().text_size(px(12.5)).text_color(c(0xd4d4d9)).line_height(relative(1.4)).child("Its 2 terminals would move along and keep running. Open the Agent inbox to say yes or no.")),
        );
    div().relative().size_full().child(todo_page(look)).child(banner)
}

// ---- b9-2

/// b9-2: a session's inbox panel after a No.
pub fn after_no(look: Look) -> Div {
    let lines = vec![
        ln(look, &[(T::M, "~/src/api (ledger-schema) $"), (T::N, " claude")]),
        plain(look, "> Use the same backoff policy as Refund retries."),
        blank(look),
        plain(look, "● humata files copy"),
        plain(look, "    --from \"Refund retries\" api/src/retry/"),
        plain(look, "    --reason \"Refund retries already wrote the backoff"),
        plain(look, "              policy. The ledger client should use the"),
        plain(look, "              same one instead of a second copy.\""),
        ln(look, &[(T::A, "  ⎿ Asked the owner in the Agent inbox. Waiting.")]),
        blank(look),
        dim(look, "  The owner asked why. Answered."),
        blank(look),
        ln(look, &[(T::R, "✕ The owner said no"), (T::M, " · 2m after asking")]),
        ln(look, &[(T::M, "  Their note:"), (T::N, " “Keep my client.rs. Copy the two policy")]),
        plain(look, "  files only and add the imports by hand.”"),
        dim(look, "  Nothing was copied."),
        blank(look),
        plain(look, "● Understood. Asking again for backoff.rs and"),
        plain(look, "  policy.rs only."),
        plain(look, "● humata files copy --from \"Refund retries\""),
        plain(look, "    api/src/retry/backoff.rs api/src/retry/policy.rs"),
        ln(look, &[(T::G, "  ⎿ Allowed: you chose “Always allow between these")]),
        ln(look, &[(T::G, "    two sessions” for files that overwrite nothing.")]),
        plain(look, "● Edit(api/src/client.rs)"),
        dim(look, "  ⎿ Added 2 imports"),
    ];
    let ink = soft(look);
    let first = div()
        .px(px(14.))
        .py(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_color(tone(look, 0xdcdce2, 0x3a3a3d))
        .bg(c(look.card))
        .child(div().flex().items_center().gap(px(10.)).child(div().text_color(c(0xc0392b)).font_weight(FontWeight::BOLD).child("✕")).child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child("Copy 3 files from Refund retries into Ledger entries?")))
        .child(
            div()
                .mt(px(6.))
                .text_color(ink)
                .line_height(relative(1.5))
                .child(rich("You said no, 3m ago, with a note: “Keep my client.rs. Copy the two policy files only and add the imports by hand.”", &[("no", Some(FontWeight::BOLD), None)])),
        )
        .child(div().mt(px(6.)).text_size(px(12.5)).text_color(muted(look)).child("Nothing was copied. claude was told and carried on."));
    let second = div()
        .mt(px(10.))
        .px(px(14.))
        .py(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_color(tone(look, 0xdcdce2, 0x3a3a3d))
        .bg(c(look.card))
        .child(div().flex().items_center().gap(px(10.)).child(div().text_color(teal(look)).font_weight(FontWeight::BOLD).child("✓")).child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).child("Copy backoff.rs and policy.rs from Refund retries")))
        .child(div().mt(px(6.)).text_color(ink).line_height(relative(1.5)).child("Allowed without asking, 2m ago, by your rule “Always allow between these two sessions” (files that overwrite nothing)."))
        .child(div().mt(px(4.)).text_size(px(12.5)).flex().gap(px(4.)).text_color(blue(look)).child("Undo the copy").child(div().text_color(c(look.text)).child("·")).child("Change this rule"));
    let head = panel_head(look, bold_title("Agent inbox"), "this session · nothing waiting");
    let body = div()
        .p(px(14.))
        .child(div().mb(px(8.)).text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).text_color(muted(look)).child("ANSWERED TODAY"))
        .child(first)
        .child(second);
    session_screen(
        look,
        Side("Ledger entries", None, false),
        "# payments-rewrite ›",
        "Ledger entries",
        "Claude Code · devbox-1",
        GREEN_HEX,
        ("claude", "zsh"),
        lines,
        head,
        body,
        "Agent inbox",
        None,
    )
}
