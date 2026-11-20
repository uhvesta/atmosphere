//! UXSIMPLE: the eleven signed-off frames of the simpler review loop (open a
//! terminal and start, ask for the stack, review and sign off layers, the
//! agent applies it). They copy `workflow/milestones/mockups/uxsimple/*.png`
//! at 1600 by 950: a stack of 3 layers (schema, dual write, reads) across 5
//! repos (api, infra, ledger, sdk, web).

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{prelude::*, *};

use crate::kit::*;

// The mockups' light-mode tones: dark text colours, bright status dots.
const GREEN: u32 = 0x28a745;
const ORANGE: u32 = 0xd98200;
const RED: u32 = 0xe0393e;
const GREY: u32 = 0xb0b0b6;
const DOT_GREEN: u32 = 0x28c840;
const DOT_ORANGE: u32 = 0xf5a623;

// ---------- small parts

fn dim(look: Look) -> u32 {
    look.term_dim
}

fn selected(look: Look) -> u32 {
    if look.dark { look.line } else { 0xe4e4ea }
}

fn t(hex: u32, text: &'static str) -> Div {
    div().flex_shrink_0().whitespace_nowrap().text_color(c(hex)).child(text)
}

fn bold(text: &'static str) -> Div {
    div().flex_shrink_0().whitespace_nowrap().font_weight(FontWeight::SEMIBOLD).child(text)
}

fn grow() -> Div {
    div().flex_1().min_w_0()
}

fn row() -> Div {
    div().flex().items_center()
}

fn link(look: Look, text: &'static str) -> Div {
    t(look.accent, text)
}

fn strong_link(look: Look, text: &'static str) -> Div {
    link(look, text).font_weight(FontWeight::SEMIBOLD)
}

fn frame(look: Look, inner: Div) -> Div {
    div().size_full().bg(c(look.content)).child(inner.size_full().text_size(px(14.)).line_height(relative(1.25)))
}

fn button14(look: Look, text: &'static str) -> Div {
    row()
        .flex_shrink_0()
        .whitespace_nowrap()
        .h(px(30.))
        .px(px(12.))
        .rounded(px(7.))
        .bg(c(look.fill))
        .border_1()
        .border_color(c(look.line))
        .child(text)
}

fn primary(look: Look, text: &'static str) -> Div {
    button14(look, text).bg(c(look.accent)).border_color(c(look.accent)).text_color(c(0xffffff)).font_weight(FontWeight::MEDIUM)
}

fn good_button(look: Look, text: &'static str) -> Div {
    let (bg, edge) = if look.dark { (0x17301f, 0x24542f) } else { (0xe9f7ed, 0xb7e0c2) };
    button14(look, text).bg(c(bg)).border_color(c(edge)).text_color(c(GREEN))
}

#[derive(Clone, Copy)]
enum Tone {
    Blue,
    Green,
    Orange,
}

fn tag(look: Look, tone: Tone, text: &'static str) -> Div {
    let (hex, bg, edge) = match (tone, look.dark) {
        (Tone::Blue, false) => (look.accent, 0xeef4ff, 0xc9d8f5),
        (Tone::Green, false) => (GREEN, 0xe9f7ed, 0xb7e0c2),
        (Tone::Orange, false) => (ORANGE, 0xfff6e3, 0xf0d49a),
        (Tone::Blue, true) => (look.accent, look.fill, look.line),
        (Tone::Green, true) => (GREEN, look.fill, look.line),
        (Tone::Orange, true) => (ORANGE, look.fill, look.line),
    };
    row()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(5.))
        .border_1()
        .border_color(c(edge))
        .bg(c(bg))
        .text_size(px(12.))
        .text_color(c(hex))
        .child(text)
}

fn dot(hex: u32) -> Div {
    div().size(px(9.)).rounded_full().flex_shrink_0().bg(c(hex))
}

fn lights() -> Div {
    traffic_lights().gap(px(8.))
}

fn sep(look: Look) -> Div {
    div().h(px(1.)).mx(px(4.)).my(px(8.)).flex_shrink_0().bg(c(look.line))
}

fn cap(look: Look, text: &'static str) -> Div {
    div().px(px(10.)).pt(px(6.)).pb(px(4.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(c(dim(look))).child(text)
}

// ---------- the sidebar, 280 px

fn nav(look: Look, text: &'static str, count: Option<&'static str>) -> Div {
    row().h(px(29.)).px(px(10.)).gap(px(10.)).rounded(px(7.)).whitespace_nowrap().child(text).when_some(count, |r, n| {
        r.child(grow()).child(t(dim(look), n))
    })
}

fn sec(look: Look, text: &'static str) -> Div {
    div().px(px(10.)).pt(px(16.)).pb(px(4.)).text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(c(dim(look))).child(text)
}

fn sidebar(look: Look, state: u32, badge: Option<&'static str>) -> Div {
    let selected_row = row()
        .h(px(29.))
        .px(px(10.))
        .ml(px(6.))
        .gap(px(10.))
        .rounded(px(7.))
        .bg(c(look.selection))
        .text_color(c(look.selection_text))
        .font_weight(FontWeight::SEMIBOLD)
        .child(dot(state))
        .child("Ledger entries")
        .when_some(badge, |r, b| r.child(grow()).child(div().text_size(px(12.)).font_weight(FontWeight::NORMAL).child(b)));
    let leaf = |hex: u32, text: &'static str| row().h(px(29.)).px(px(10.)).ml(px(6.)).gap(px(10.)).child(dot(hex)).child(text);
    let folded = |text: &'static str| row().h(px(29.)).px(px(10.)).gap(px(10.)).child(small_icon(IconName::ChevronRight, dim(look))).child(text);
    let host = |text: &'static str| row().h(px(29.)).px(px(10.)).gap(px(10.)).child(dot(DOT_GREEN)).child(text);
    div()
        .w(px(280.))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .px(px(12.))
        .py(px(22.))
        .bg(c(look.sidebar))
        .border_r_1()
        .border_color(c(look.line))
        .child(row().ml(px(8.)).mb(px(22.)).child(lights()))
        .child(
            row()
                .h(px(30.))
                .px(px(10.))
                .mb(px(12.))
                .rounded(px(7.))
                .bg(c(look.fill))
                .text_color(c(dim(look)))
                .child("Search")
                .child(grow())
                .child("⌘K"),
        )
        .child(nav(look, "Home", None))
        .child(nav(look, "Agent inbox", Some("2")))
        .child(nav(look, "Board", None))
        .child(nav(look, "Desk", None))
        .child(sec(look, "Channels"))
        .child(
            row()
                .h(px(29.))
                .px(px(10.))
                .gap(px(10.))
                .child(small_icon(IconName::ChevronDown, dim(look)))
                .child(bold("payments-rewrite")),
        )
        .child(selected_row)
        .child(leaf(DOT_GREEN, "Refund retries"))
        .child(leaf(GREY, "API rate limiter"))
        .child(folded("infra"))
        .child(folded("pr-reviews"))
        .child(sec(look, "Hosts"))
        .child(host("This Mac"))
        .child(host("devbox-1"))
        .child(grow())
        .child(nav(look, "Settings", None))
}

// ---------- the session in the middle

struct Tab {
    name: &'static str,
    dot: Option<u32>,
    on: bool,
}

fn session_tab(look: Look, tab: &Tab) -> Div {
    let lead = match tab.dot {
        Some(hex) => dot(hex),
        None => div().text_color(c(dim(look))).child(">_"),
    };
    row()
        .relative()
        .h_full()
        .px(px(14.))
        .gap(px(8.))
        .border_r_1()
        .border_color(c(look.line))
        .whitespace_nowrap()
        .when(tab.on, |r| {
            r.bg(c(look.content)).text_color(c(look.text)).child(
                div().absolute().left_0().right_0().bottom_0().h(px(2.)).bg(c(look.accent)),
            )
        })
        .when(!tab.on, |r| r.text_color(c(look.secondary)))
        .child(lead)
        .child(tab.name)
        .child(small_icon(IconName::X, dim(look)))
}

fn mono_line(runs: Vec<Div>) -> Div {
    row().h(px(21.)).flex_shrink_0().whitespace_nowrap().font_family(MONO).text_size(px(13.)).children(runs)
}

fn plain(text: &'static str) -> Div {
    div().flex_shrink_0().whitespace_nowrap().child(text)
}

fn user(text: &'static str) -> Div {
    bold(text)
}

fn dimmed(look: Look, text: &'static str) -> Div {
    t(dim(look), text)
}

fn session_main(look: Look, state: u32, tabs: &[Tab], tabs_right: Option<&'static str>, out: Vec<Div>, prompt: &'static str) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(
            row()
                .h(px(65.))
                .px(px(18.))
                .gap(px(12.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(c(look.line))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().text_size(px(13.)).text_color(c(look.accent)).child("# payments-rewrite ›"))
                        .child(row().gap(px(8.)).child(dot(state)).child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Ledger entries"))),
                )
                .child(grow())
                .child(t(dim(look), "Claude Code · devbox-1"))
                .child(small_icon(IconName::GitFork, dim(look)))
                .child(small_icon(IconName::Maximize2, dim(look)))
                .child(small_icon(IconName::Ellipsis, dim(look))),
        )
        .child(
            row()
                .h(px(40.))
                .flex_shrink_0()
                .items_stretch()
                .border_b_1()
                .border_color(c(look.line))
                .bg(c(look.panel))
                .children(tabs.iter().map(|tab| session_tab(look, tab)))
                .child(row().px(px(14.)).text_color(c(look.secondary)).child("+"))
                .child(grow())
                .when_some(tabs_right, |r, text| r.child(row().px(px(14.)).text_color(c(dim(look))).child(text))),
        )
        .child(div().flex_1().min_h_0().overflow_hidden().px(px(18.)).py(px(14.)).flex().flex_col().children(out))
        .child(
            row()
                .mx(px(18.))
                .mb(px(8.))
                .px(px(10.))
                .py(px(8.))
                .gap(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(c(look.tertiary))
                .font_family(MONO)
                .text_size(px(13.))
                .child(">")
                .child(prompt)
                .child(div().w(px(8.)).h(px(15.)).bg(c(look.text))),
        )
        .child(div().mx(px(18.)).mb(px(14.)).font_family(MONO).text_size(px(13.)).text_color(c(dim(look))).child("▸▸ accept edits on (shift+tab to cycle)"))
}

fn rail_item(look: Look, text: &'static str, on: bool) -> Div {
    let hex = if on { look.accent } else { look.secondary };
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(4.))
        .text_size(px(11.))
        .text_color(c(hex))
        .child(div().w(px(20.)).h(px(18.)).rounded(px(4.)).border_2().border_color(c(if on { look.accent } else { look.tertiary })))
        .child(text)
}

fn rail(look: Look, on: bool, diff: &'static str) -> Div {
    div()
        .w(px(76.))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(22.))
        .pt(px(70.))
        .border_l_1()
        .border_color(c(look.line))
        .bg(c(look.panel))
        .child(rail_item(look, "Agent inbox", false))
        .child(rail_item(look, diff, on))
        .child(rail_item(look, "Files", false))
        .child(rail_item(look, "Board", false))
        .child(rail_item(look, "Journal", false))
        .child(rail_item(look, "Info", false))
}

// ---------- the Diff panel's pieces

fn panel(look: Look, width: f32, head: &'static str, strip: Option<Div>, body: Div, foot: Div) -> Div {
    div()
        .w(px(width))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .border_l_1()
        .border_color(c(look.line))
        .bg(c(look.content))
        .child(
            row()
                .h(px(44.))
                .px(px(14.))
                .gap(px(10.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(c(look.line))
                .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Diff"))
                .child(t(dim(look), head))
                .child(grow())
                .child(small_icon(IconName::Maximize2, dim(look)))
                .child(small_icon(IconName::X, look.text)),
        )
        .children(strip)
        .child(body.flex_1().min_h_0().overflow_hidden().flex().flex_col())
        .child(footer(look, foot))
}

fn footer(look: Look, inner: Div) -> Div {
    inner.h(px(52.)).px(px(14.)).gap(px(16.)).flex_shrink_0().border_t_1().border_color(c(look.line)).bg(c(look.content)).whitespace_nowrap()
}

fn foot() -> Div {
    row()
}

fn strip(look: Look, text: &'static str) -> Div {
    let (bg, edge) = if look.dark { (look.fill, look.line) } else { (0xeef4ff, 0xd6e4fb) };
    row()
        .px(px(14.))
        .py(px(7.))
        .gap(px(10.))
        .flex_shrink_0()
        .bg(c(bg))
        .border_b_1()
        .border_color(c(edge))
        .text_size(px(13.))
        .child(t(look.accent, "●"))
        .child(text)
        .child(grow())
        .child(link(look, "Undo"))
}

fn list_box(look: Look) -> Div {
    div().flex().flex_col().px(px(8.)).py(px(10.)).bg(c(look.panel)).overflow_hidden()
}

fn looks_good(look: Look) -> Div {
    let _ = look;
    t(GREEN, "✓ Looks good").text_size(px(12.5))
}

fn layer(look: Look, n: Option<&'static str>, title: &'static str, prs: &'static str, pp: &'static str, sel: bool, status: Option<Div>) -> Div {
    let number = match n {
        Some(n) => div()
            .size(px(20.))
            .rounded_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .border_1()
            .border_color(c(look.tertiary))
            .bg(c(look.card))
            .text_color(c(look.text))
            .when(sel, |d| d.bg(c(look.accent)).border_color(c(look.accent)).text_color(c(0xffffff)))
            .child(n),
        None => div()
            .size(px(20.))
            .rounded_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .border_1()
            .when(!sel, |d| d.border_dashed().border_color(c(look.tertiary)).bg(c(look.card)))
            .when(sel, |d| d.border_color(c(look.accent)).bg(c(look.accent)).text_color(c(0xffffff)))
            .child("≡"),
    };
    row()
        .items_start()
        .px(px(10.))
        .py(px(6.))
        .gap(px(9.))
        .rounded(px(7.))
        .when(sel, |r| r.bg(c(selected(look))))
        .child(number)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    row()
                        .gap(px(5.))
                        .child(div().font_weight(if n.is_some() { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM }).child(title))
                        .child(t(dim(look), prs).text_size(px(12.5))),
                )
                .when(!pp.is_empty(), |d| d.child(div().mt(px(2.)).text_size(px(12.5)).text_color(c(look.secondary)).child(pp))),
        )
        .children(status.map(|s| s.text_size(px(12.5)).mt(px(1.))))
}

/// The stack list: Whole stack, then the three layers. `status` and `pp` are per layer.
fn stack(look: Look, sel: usize, whole: Option<Div>, prs: [&'static str; 3], status: [Option<Div>; 3], pp: [&'static str; 3]) -> Div {
    let [s1, s2, s3] = status;
    div()
        .flex()
        .flex_col()
        .child(layer(look, None, "Whole stack", "5 repos · 23 files", "", sel == 0, whole))
        .child(layer(look, Some("1"), "schema", prs[0], pp[0], sel == 1, s1))
        .child(layer(look, Some("2"), "dual write", prs[1], pp[1], sel == 2, s2))
        .child(layer(look, Some("3"), "reads", prs[2], pp[2], sel == 3, s3))
}

const REPOS: [&str; 3] = ["ledger · infra", "api · ledger · sdk", "api · web · sdk"];
const _: [&str; 3] = REPOS;

fn repo(look: Look, name: &'static str, base: &'static str, add: &'static str, del: &'static str) -> Div {
    row()
        .px(px(10.))
        .py(px(5.))
        .gap(px(7.))
        .child(small_icon(IconName::ChevronDown, dim(look)))
        .child(bold(name))
        .when(!base.is_empty(), |r| r.child(t(dim(look), base)))
        .child(grow())
        .child(pm(add, del))
}

fn pm(add: &'static str, del: &'static str) -> Div {
    row().gap(px(6.)).flex_shrink_0().child(t(GREEN, add)).child(t(RED, del))
}

#[derive(Clone, Copy, PartialEq)]
enum Ck {
    No,
    Done,
    Changed,
}

fn check(look: Look, ck: Ck) -> Option<Div> {
    let base = div().size(px(15.)).rounded_full().flex_shrink_0().flex().items_center().justify_center();
    match ck {
        Ck::No => Some(base.border_1().border_color(c(look.tertiary))),
        Ck::Done => Some(base.bg(c(GREEN)).child(Icon::new(IconName::Check).size(px(10.)).text_color(c(0xffffff)))),
        Ck::Changed => Some(base.border_1().border_color(c(ORANGE)).bg(c(look.attention_bg))),
    }
}

fn file(look: Look, name: &'static str, dir: &'static str, add: &'static str, del: &'static str, ck: Ck, sel: bool, extra: Vec<Div>) -> Div {
    row()
        .px(px(10.))
        .pl(px(30.))
        .py(px(5.))
        .gap(px(7.))
        .rounded(px(7.))
        .whitespace_nowrap()
        .when(sel, |r| r.bg(c(selected(look))))
        .children(check(look, ck))
        .child(name)
        .child(t(dim(look), dir))
        .child(grow())
        .children(extra.into_iter().map(|e| e.flex_shrink_0()))
        .child(pm(add, del))
}

fn more(look: Look, text: &'static str) -> Div {
    row().pl(px(40.)).py(px(5.)).text_color(c(dim(look))).child(text)
}

fn key(look: Look) -> Div {
    tag(look, Tone::Blue, "key")
}

fn count(hex: u32, n: &'static str) -> Div {
    row().gap(px(3.)).text_color(c(hex)).child(div().size(px(11.)).rounded(px(2.)).border_1().border_color(c(hex))).child(n)
}

fn done_count(n: &'static str) -> Div {
    t(GREEN, n)
}

fn file_bar(look: Look, path: &'static str, name: &'static str, key_tag: bool, right: Div) -> Div {
    file_bar_note(look, path, name, key_tag, None, right)
}

fn file_bar_note(look: Look, path: &'static str, name: &'static str, key_tag: bool, note: Option<&'static str>, right: Div) -> Div {
    row()
        .h(px(38.))
        .px(px(14.))
        .gap(px(8.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(c(look.line))
        .text_color(c(look.secondary))
        .whitespace_nowrap()
        .child(path)
        .child(div().text_color(c(look.text)).child(name))
        .when(key_tag, |r| r.child(key(look)))
        .when_some(note, |r, n| r.child(t(look.secondary, n)))
        .child(grow())
        .child(right)
}

fn open_in_editor(look: Look) -> Div {
    t(look.secondary, "Open in editor")
}

type Rows = Vec<(char, u32, &'static str)>;

fn lines(look: Look, hunk: &'static str, rows: Rows) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .font_family(MONO)
        .text_size(px(13.))
        .when(!hunk.is_empty(), |d| d.child(div().px(px(12.)).pt(px(8.)).pb(px(2.)).text_color(c(look.secondary)).child(hunk)))
        .children(rows.into_iter().map(|(kind, n, text)| {
            let (bg, sym) = match kind {
                '+' => (Some(look.add_bg), "+"),
                '-' => (Some(look.del_bg), "−"),
                _ => (Option::None, " "),
            };
            row()
                .px(px(12.))
                .py(px(1.))
                .h(px(17.))
                .whitespace_nowrap()
                .children(bg.map(|b| div().absolute().left_0().right_0().h(px(17.)).bg(c(b))))
                .relative()
                .child(div().w(px(52.)).flex_shrink_0().text_color(c(dim(look))).child(format!("{n:>3} {sym}")))
                .child(if text.is_empty() { " " } else { text })
        }))
}

#[derive(Clone, Copy, PartialEq)]
enum Note {
    Plain,
    Draft,
    Done,
}

fn note_card(look: Look, kind: Note, top: Vec<Div>, body: Div, reply: Option<(&'static str, &'static str)>, acts: Vec<Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .mx(px(12.))
        .my(px(10.))
        .px(px(12.))
        .py(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(c(look.line))
        .bg(c(look.card))
        .line_height(relative(1.4))
        .when(kind == Note::Draft, |d| d.border_2().border_color(c(look.accent)))
        .when(kind == Note::Done, |d| d.bg(c(look.panel)).text_color(c(look.secondary)))
        .child(row().gap(px(8.)).mb(px(5.)).whitespace_nowrap().children(top))
        .child(body)
        .when_some(reply, |d, (who, text)| {
            d.child(
                row()
                    .items_start()
                    .gap(px(8.))
                    .mt(px(8.))
                    .pt(px(8.))
                    .border_t_1()
                    .border_color(c(look.line))
                    .child(div().flex_shrink_0().text_color(c(look.text)).font_weight(FontWeight::SEMIBOLD).child(who))
                    .child(div().child(text)),
            )
        })
        .when(!acts.is_empty(), |d| d.child(row().gap(px(16.)).mt(px(8.)).children(acts)))
}

fn who(text: &'static str) -> Div {
    bold(text)
}

fn text_only(text: &'static str) -> Div {
    div().child(text)
}

fn menu(look: Look) -> Div {
    div()
        .absolute()
        .flex()
        .flex_col()
        .p(px(6.))
        .rounded(px(10.))
        .bg(c(look.card))
        .border_1()
        .border_color(c(look.line))
        .shadow_lg()
}

fn menu_caption(look: Look, text: &'static str) -> Div {
    div().px(px(10.)).pt(px(6.)).pb(px(8.)).text_size(px(12.5)).text_color(c(look.secondary)).line_height(relative(1.4)).child(text)
}

fn menu_item(look: Look, on: bool, lead: Div, text: &'static str, right: Option<Div>) -> Div {
    row()
        .px(px(10.))
        .py(px(6.))
        .gap(px(9.))
        .rounded(px(6.))
        .whitespace_nowrap()
        .when(on, |r| r.bg(c(look.fill)))
        .child(lead)
        .child(text)
        .children(right.map(|r| div().flex_1().flex().justify_end().child(r)))
}

// ---------- the full-screen Diff

fn full(look: Look, title_extra: Div, buttons: Vec<Div>, left: Div, code: Div, bottom: Div) -> Div {
    div()
        .size_full()
        .relative()
        .flex()
        .flex_col()
        .child(
            row()
                .h(px(65.))
                .px(px(18.))
                .gap(px(12.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(c(look.line))
                .child(div().ml(px(8.)).mr(px(10.)).child(lights()))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(div().text_size(px(13.)).text_color(c(look.secondary)).child("# payments-rewrite › Ledger entries"))
                        .child(row().gap(px(10.)).child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child("Diff")).child(title_extra)),
                )
                .child(grow())
                .child(row().gap(px(8.)).children(buttons)),
        )
        .child(
            row()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .child(left.w(px(420.)).h_full().flex_shrink_0().border_r_1().border_color(c(look.line)))
                .child(code.flex_1().min_w_0().h_full().flex().flex_col().overflow_hidden()),
        )
        .child(footer(look, bottom))
}

fn new_agent_tab(look: Look, pressed: bool) -> Div {
    button14(look, "New agent tab").gap(px(6.)).when(pressed, |b| b.bg(c(selected(look)))).child(small_icon(IconName::ChevronDown, look.text))
}

fn full_buttons(look: Look) -> Vec<Div> {
    vec![new_agent_tab(look, false), button14(look, "Exit full screen")]
}

fn full_title(look: Look, text: &'static str) -> Div {
    t(dim(look), text)
}

fn ok(look: Look) -> Option<Div> {
    Some(looks_good(look))
}

fn nav_links(look: Look) -> Vec<Div> {
    vec![link(look, "‹ Previous layer"), link(look, "Next layer ›")]
}

// ---------- shared content

const L1: &str = "Adds the accounts and entries tables";
const L2: &str = "Writes both tables in one transaction";
const L3: &str = "Moves reads to entries";

fn files1(look: Look, sel: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .child(repo(look, "ledger", "", "+79", "−0"))
        .child(file(look, "0042_accounts.sql", "migrations/", "+41", "−0", Ck::No, sel, vec![key(look)]))
        .child(file(look, "0043_entries.sql", "migrations/", "+38", "−0", Ck::No, false, vec![]))
        .child(repo(look, "infra", "", "+18", "−0"))
        .child(file(look, "ledger.tf", "db/", "+18", "−0", Ck::No, false, vec![]))
}

fn sql_rows() -> Rows {
    vec![
        ('+', 1, "CREATE TABLE accounts ("),
        ('+', 2, "    id          uuid PRIMARY KEY,"),
        ('+', 3, "    owner_id    uuid NOT NULL,"),
        ('+', 4, "    currency    char(3) NOT NULL,"),
        ('+', 5, "    created_at  timestamptz NOT NULL DEFAULT now()"),
        ('+', 6, ");"),
    ]
}

fn post_rows() -> Rows {
    vec![
        (' ', 40, "    pub async fn post(&self, entry: Entry) -> Result<()> {"),
        ('-', 41, "        self.db.insert_legacy(&entry).await?;"),
        ('+', 41, "        let tx = self.db.begin().await?;"),
        ('+', 42, "        tx.insert_entry(&entry).await?;"),
        ('+', 43, "        tx.insert_legacy(&entry).await?;"),
        ('+', 44, "        tx.commit().await?;"),
    ]
}

fn batch_rows() -> Rows {
    vec![
        (' ', 45, "        self.metrics.posted.inc();"),
        (' ', 46, "        Ok(())"),
        (' ', 47, "    }"),
        (' ', 48, ""),
        ('+', 49, "    pub async fn post_many(&self, entries: &[Entry]) -> Result<()> {"),
        ('+', 50, "        for e in entries {"),
        ('+', 51, "            self.post(e.clone()).await?;"),
        ('+', 52, "        }"),
    ]
}

fn files2(look: Look) -> Div {
    div()
        .flex()
        .flex_col()
        .child(repo(look, "api", "", "+143", "−24"))
        .child(file(look, "client.rs", "src/", "+62", "−21", Ck::No, true, vec![key(look), count(dim(look), "2")]))
        .child(file(look, "entry.rs", "src/", "+81", "−3", Ck::Done, false, vec![]))
        .child(repo(look, "ledger", "", "+63", "−18"))
        .child(file(look, "posting.rs", "src/", "+63", "−18", Ck::Done, false, vec![]))
        .child(repo(look, "sdk", "", "+44", "−8"))
        .child(file(look, "entry.ts", "src/", "+44", "−8", Ck::No, false, vec![]))
}

fn legacy_card(look: Look) -> Div {
    note_card(
        look,
        Note::Plain,
        vec![who("You"), tag(look, Tone::Blue, "Draft"), grow(), link(look, "Edit")],
        text_only("A failed commit is retried by the caller, and insert_legacy has no unique key. Make the legacy insert an upsert on entry.id."),
        None,
        vec![],
    )
}

fn all_files(look: Look, api_client: (Ck, Vec<Div>, &'static str, &'static str), posting: (Ck, Vec<Div>), base: bool, selected_file: Option<&'static str>) -> Div {
    let (client_ck, client_extra, client_add, client_del) = api_client;
    let (posting_ck, posting_extra) = posting;
    let against = |main: &'static str| if base { main } else { "" };
    div()
        .flex()
        .flex_col()
        .child(repo(look, "api", against("against main"), if base { "+216" } else { "+205" }, "−45"))
        .child(file(look, "client.rs", "src/", client_add, client_del, client_ck, selected_file == Some("client.rs"), client_extra))
        .child(file(look, "entry.rs", "src/", "+81", "−3", Ck::Done, false, vec![]))
        .child(file(look, "reads.rs", "src/ledger/", "+62", "−21", Ck::Done, false, vec![]))
        .child(repo(look, "infra", against("against main"), "+18", "−0"))
        .child(file(look, "ledger.tf", "db/", "+18", "−0", Ck::Done, false, vec![]))
        .child(repo(look, "ledger", against("against release/2.4"), "+142", "−18"))
        .child(file(look, "0042_accounts.sql", "migrations/", "+41", "−0", Ck::Done, false, vec![]))
        .child(file(look, "0043_entries.sql", "migrations/", "+38", "−0", Ck::Done, false, vec![]))
        .child(file(look, "posting.rs", "src/", "+63", "−18", posting_ck, false, posting_extra))
        .child(repo(look, "sdk", against("against main"), "+96", "−12"))
        .child(file(look, "entry.ts", "src/", "+44", "−8", Ck::Done, false, vec![]))
        .child(file(look, "client.ts", "src/", "+52", "−4", Ck::Done, false, vec![]))
        .child(repo(look, "web", against("against main"), "+71", "−30"))
        .child(file(look, "LedgerTable.tsx", "src/ledger/", "+71", "−30", Ck::Done, selected_file == Some("LedgerTable.tsx"), vec![]))
}

fn tabs2() -> Vec<Tab> {
    vec![Tab { name: "claude", dot: Some(DOT_GREEN), on: true }, Tab { name: "zsh", dot: None, on: false }]
}

// ---------- 1-1

pub fn start_working(look: Look) -> Div {
    let d = |s| dimmed(look, s);
    let out = vec![
        mono_line(vec![d("~/src (ledger-entries) $ claude")]),
        mono_line(vec![user("> Move ledger reads to the new entries table. It touches api, web, ledger,")]),
        mono_line(vec![user("  sdk and infra. Make it a stack of small PRs I can review in order.")]),
        mono_line(vec![]),
        mono_line(vec![plain("● Read(ledger/docs/model.md)")]),
        mono_line(vec![d("  ⎿ Read 212 lines")]),
        mono_line(vec![plain("● Updated plan")]),
        mono_line(vec![t(GREEN, "  ✓ Layer 1 · schema: accounts and entries tables (ledger, infra)")]),
        mono_line(vec![t(ORANGE, "  ◐ Layer 2 · dual write: write both tables (api, ledger, sdk)")]),
        mono_line(vec![d("    Layer 3 · reads: move reads to entries (api, web, sdk)")]),
        mono_line(vec![plain("● Write(ledger/migrations/0042_accounts.sql)")]),
        mono_line(vec![d("  ⎿ Wrote 41 lines")]),
        mono_line(vec![plain("● Write(infra/db/ledger.tf)")]),
        mono_line(vec![d("  ⎿ Wrote 18 lines")]),
        mono_line(vec![plain("● Update(api/src/client.rs)")]),
        mono_line(vec![d("  ⎿ Updated with 62 additions and 21 removals")]),
        mono_line(vec![plain("● Bash(bazel test //api/... //ledger/...)")]),
        mono_line(vec![t(GREEN, "  ⎿ 31 tests pass")]),
        mono_line(vec![]),
        mono_line(vec![t(ORANGE, "✱ Writing the sdk types… (3m 02s · esc to interrupt)")]),
    ];
    frame(
        look,
        row()
            .items_stretch()
            .child(sidebar(look, DOT_GREEN, None))
            .child(session_main(look, DOT_GREEN, &tabs2(), Some("5 repos · devbox-1"), out, ""))
            .child(rail(look, false, "Diff · 9")),
    )
}

// ---------- 1-2

pub fn stack_made(look: Look) -> Div {
    let d = |s| dimmed(look, s);
    let out = vec![
        mono_line(vec![plain("● Bash(stack submit --all)")]),
        mono_line(vec![d("  ⎿ 3 layers · 8 pull requests")]),
        mono_line(vec![d("    1 schema      acme/ledger#91  acme/infra#40")]),
        mono_line(vec![d("    2 dual write  acme/api#482  acme/ledger#92  acme/sdk#17")]),
        mono_line(vec![d("    3 reads       acme/api#483  acme/web#210  acme/sdk#18")]),
        mono_line(vec![]),
        mono_line(vec![plain("The stack is up: 3 layers across 5 repos, 23 files.")]),
        mono_line(vec![plain("Each layer builds and passes its tests on its own.")]),
        mono_line(vec![plain("Say \"show me the stack\" and I'll lay it out for")]),
        mono_line(vec![plain("review in the Diff panel.")]),
    ];
    let body = list_box(look)
        .flex_1()
        .child(cap(look, "STACK · 3 LAYERS"))
        .child(stack(look, 0, None, ["2 PRs", "3 PRs", "3 PRs"], [None, None, None], ["", "", ""]))
        .child(sep(look))
        .child(repo(look, "api", "against main", "+205", "−45"))
        .child(file(look, "client.rs", "src/", "+62", "−21", Ck::No, false, vec![]))
        .child(file(look, "entry.rs", "src/", "+81", "−3", Ck::No, false, vec![]))
        .child(file(look, "reads.rs", "src/ledger/", "+62", "−21", Ck::No, false, vec![]))
        .child(repo(look, "infra", "against main", "+18", "−0"))
        .child(file(look, "ledger.tf", "db/", "+18", "−0", Ck::No, false, vec![]))
        .child(repo(look, "ledger", "against release/2.4", "+142", "−18"))
        .child(file(look, "0042_accounts.sql", "migrations/", "+41", "−0", Ck::No, false, vec![]))
        .child(file(look, "0043_entries.sql", "migrations/", "+38", "−0", Ck::No, false, vec![]))
        .child(file(look, "posting.rs", "src/", "+63", "−18", Ck::No, false, vec![]))
        .child(repo(look, "sdk", "against main", "+96", "−12"))
        .child(file(look, "entry.ts", "src/", "+44", "−8", Ck::No, false, vec![]))
        .child(file(look, "client.ts", "src/", "+52", "−4", Ck::No, false, vec![]))
        .child(repo(look, "web", "against main", "+71", "−30"))
        .child(file(look, "LedgerTable.tsx", "src/ledger/", "+71", "−30", Ck::No, false, vec![]))
        .child(more(look, "11 more files…"));
    let bottom = foot().child(link(look, "8 pull requests open")).child(grow()).child(button14(look, "Send to claude").opacity(0.5));
    frame(
        look,
        row()
            .items_stretch()
            .child(sidebar(look, DOT_GREEN, None))
            .child(session_main(look, DOT_GREEN, &tabs2(), None, out, "show me the stack"))
            .child(panel(look, 500., "5 repos", None, body, bottom))
            .child(rail(look, true, "Diff · 23")),
    )
}

// ---------- 2-1

pub fn ask_agent_to_show(look: Look) -> Div {
    let d = |s| dimmed(look, s);
    let out = vec![
        mono_line(vec![user("> show me the stack")]),
        mono_line(vec![]),
        mono_line(vec![plain("● humata · review_prepare(stack)")]),
        mono_line(vec![d("  ⎿ 3 layers · 5 repos · 4 key files, 9 to skim")]),
        mono_line(vec![]),
        mono_line(vec![plain("The Diff panel now shows the stack, bottom layer")]),
        mono_line(vec![plain("first. Start at layer 1: two new tables and the")]),
        mono_line(vec![plain("Terraform that sizes them. In layer 2 the file to")]),
        mono_line(vec![plain("read closely is api/src/client.rs.")]),
        mono_line(vec![]),
        mono_line(vec![plain("I only arranged it. Ask me to review it if you")]),
        mono_line(vec![plain("want my findings on the lines.")]),
    ];
    let body = div()
        .flex()
        .flex_col()
        .child(
            list_box(look)
                .flex_shrink_0()
                .child(cap(look, "STACK · 3 LAYERS"))
                .child(stack(look, 1, None, REPOS, [None, None, None], [L1, "", ""]))
                .child(sep(look))
                .child(files1(look, true)),
        )
        .child(file_bar(look, "ledger · migrations/", "0042_accounts.sql", false, open_in_editor(look)))
        .child(lines(look, "", sql_rows()));
    let bottom = foot().child(link(look, "‹ Previous")).child(link(look, "Next layer ›")).child(grow()).child(button14(look, "Looks good"));
    frame(
        look,
        row()
            .items_stretch()
            .child(sidebar(look, DOT_GREEN, None))
            .child(session_main(look, DOT_GREEN, &tabs2(), None, out, ""))
            .child(panel(look, 500., "Layer 1 of 3", Some(strip(look, "claude arranged the stack and moved you to layer 1")), body, bottom))
            .child(rail(look, true, "Diff · 23")),
    )
}

// ---------- 2-2

pub fn stack_full_screen(look: Look) -> Div {
    let mut rows = sql_rows();
    rows.extend([
        (' ', 7, ""),
        ('+', 8, "CREATE TABLE entries ("),
        ('+', 9, "    id          bigserial PRIMARY KEY,"),
        ('+', 10, "    account_id  uuid NOT NULL REFERENCES accounts (id),"),
        ('+', 11, "    amount      numeric(20, 4) NOT NULL,"),
        ('+', 12, "    posted_at   timestamptz NOT NULL"),
        ('+', 13, ");"),
        (' ', 14, ""),
        ('+', 15, "CREATE INDEX entries_posted_brin ON entries"),
        ('+', 16, "    USING brin (posted_at);"),
    ]);
    let left = list_box(look)
        .child(cap(look, "STACK · 3 LAYERS"))
        .child(stack(look, 1, None, REPOS, [None, None, None], [L1, "", ""]))
        .child(sep(look))
        .child(files1(look, true))
        .child(more(look, "Repos this layer doesn't touch are hidden").pl(px(30.)));
    let code = div()
        .flex()
        .flex_col()
        .child(file_bar(look, "ledger · migrations/", "0042_accounts.sql", true, open_in_editor(look)))
        .child(lines(look, "", rows));
    let bottom = foot()
        .child(link(look, "‹ Previous layer"))
        .child(link(look, "Next layer ›"))
        .child(t(dim(look), "⌥⌘↑ ⌥⌘↓"))
        .child(grow())
        .child(t(dim(look), "Layer 1 · 0 of 3 files looked at"))
        .child(button14(look, "Looks good"));
    frame(look, full(look, full_title(look, "Layer 1 of 3 · schema"), full_buttons(look), left, code, bottom))
}

// ---------- 3-1

pub fn comment_on_a_line(look: Look) -> Div {
    let left = list_box(look)
        .child(cap(look, "STACK · 3 LAYERS"))
        .child(stack(look, 2, None, REPOS, [ok(look), Some(t(dim(look), "2 drafts")), None], ["", L2, ""]))
        .child(sep(look))
        .child(files2(look));
    let draft = note_card(
        look,
        Note::Draft,
        vec![who("You"), tag(look, Tone::Blue, "Draft"), t(dim(look), "line 51")],
        row().child("One transaction for the whole batch, not one per entry.").child(div().w(px(1.5)).h(px(16.)).bg(c(look.accent))),
        None,
        vec![],
    )
    .child(row().mt(px(8.)).gap(px(16.)).child(grow()).child(t(dim(look), "Esc to cancel")).child(strong_link(look, "Save draft ⌘↩")));
    let code = div()
        .flex()
        .flex_col()
        .child(file_bar(look, "api · src/", "client.rs", true, open_in_editor(look)))
        .child(lines(look, "@@ -40,7 +40,12 @@ impl Client {", post_rows()))
        .child(legacy_card(look))
        .child(lines(look, "", batch_rows()))
        .child(draft);
    let bottom = foot()
        .children(nav_links(look))
        .child(grow())
        .child(t(dim(look), "2 drafts · 1 layer looks good"))
        .child(primary(look, "Send to claude"));
    frame(look, full(look, full_title(look, "Layer 2 of 3 · dual write"), full_buttons(look), left, code, bottom))
}

// ---------- 3-2

pub fn looks_good_per_layer(look: Look) -> Div {
    let left = list_box(look)
        .child(cap(look, "STACK · 3 LAYERS"))
        .child(stack(look, 3, None, REPOS, [ok(look), Some(t(look.accent, "3 drafts")), ok(look)], ["", "", L3]))
        .child(sep(look))
        .child(
            div()
                .flex()
                .flex_col()
                .child(repo(look, "api", "", "+62", "−21"))
                .child(file(look, "reads.rs", "src/ledger/", "+62", "−21", Ck::Done, true, vec![key(look)]))
                .child(repo(look, "web", "", "+71", "−30"))
                .child(file(look, "LedgerTable.tsx", "src/ledger/", "+71", "−30", Ck::Done, false, vec![]))
                .child(file(look, "useEntries.ts", "src/ledger/", "+24", "−9", Ck::Done, false, vec![]))
                .child(repo(look, "sdk", "", "+52", "−4"))
                .child(file(look, "client.ts", "src/", "+52", "−4", Ck::Done, false, vec![]))
                .child(more(look, "▸ 4 generated files · skim").pl(px(30.))),
        );
    let rows: Rows = vec![
        (' ', 18, "    pub async fn balance(&self, account: AccountId) -> Result<Decimal> {"),
        ('-', 19, "        let rows = self.db.legacy_entries(account).await?;"),
        ('-', 20, "        Ok(rows.iter().map(|r| r.amount).sum())"),
        ('+', 19, "        let sum = self.db"),
        ('+', 20, "            .entries_sum(account)"),
        ('+', 21, "            .await?;"),
        ('+', 22, "        Ok(sum.unwrap_or(Decimal::ZERO))"),
        (' ', 23, "    }"),
    ];
    let code = div()
        .flex()
        .flex_col()
        .child(file_bar(look, "api · src/ledger/", "reads.rs", true, t(GREEN, "✓ You looked at this")))
        .child(lines(look, "@@ -18,6 +18,8 @@ impl Reads {", rows));
    let bottom = foot()
        .child(link(look, "‹ Previous layer"))
        .child(link(look, "Next layer ›"))
        .child(grow())
        .child(good_button(look, "✓ Looks good"))
        .child(primary(look, "Send to claude ▴"));
    let summary = menu(look)
        .right(px(14.))
        .bottom(px(60.))
        .w(px(330.))
        .child(menu_caption(look, "One message to claude:"))
        .child(menu_item(look, false, t(look.accent, "▢"), "3 comments on layer 2 · dual write", None))
        .child(menu_item(look, false, t(GREEN, "✓"), "Layer 1 · schema looks good", None))
        .child(menu_item(look, false, t(GREEN, "✓"), "Layer 3 · reads looks good", None))
        .child(menu_caption(look, "A layer that looks good is remembered. If its lines change later, the tick clears and you see only what changed.").pt(px(8.)));
    frame(look, full(look, full_title(look, "Layer 3 of 3 · reads"), full_buttons(look), left, code, bottom).child(summary))
}

// ---------- 3-3

pub fn new_agent_tab_menu(look: Look) -> Div {
    let left = list_box(look)
        .child(cap(look, "STACK · 3 LAYERS"))
        .child(stack(look, 2, None, REPOS, [ok(look), Some(t(look.accent, "3 drafts")), ok(look)], ["", L2, ""]))
        .child(sep(look))
        .child(files2(look));
    let code = div()
        .flex()
        .flex_col()
        .child(file_bar(look, "api · src/", "client.rs", true, open_in_editor(look)))
        .child(lines(look, "@@ -40,7 +40,12 @@ impl Client {", post_rows()))
        .child(legacy_card(look))
        .child(lines(look, "", batch_rows()));
    let bottom = foot()
        .children(nav_links(look))
        .child(grow())
        .child(button14(look, "Looks good"))
        .child(primary(look, "Send to claude"));
    let popup_menu = menu(look)
        .right(px(150.))
        .top(px(54.))
        .w(px(340.))
        .child(menu_caption(look, "Opens a new tab in this session, beside the diff. It starts with no history. Pick the model inside the agent."))
        .child(menu_item(look, false, t(dim(look), ">_"), "Terminal", Some(t(dim(look), "⌘T"))))
        .child(sep(look))
        .child(menu_item(look, false, dot(DOT_GREEN), "claude", None))
        .child(menu_item(look, true, dot(DOT_GREEN), "copilot", Some(t(dim(look), "last used"))))
        .child(menu_item(look, false, dot(GREY), "codex · not installed", None).text_color(c(dim(look))));
    let buttons = vec![new_agent_tab(look, true), button14(look, "Exit full screen")];
    frame(look, full(look, full_title(look, "Layer 2 of 3 · dual write"), buttons, left, code, bottom).child(popup_menu))
}

// ---------- 3-4

pub fn fresh_agent_beside_diff(look: Look) -> Div {
    let d = |s| dimmed(look, s);
    let out = vec![
        mono_line(vec![d("~/src (ledger-entries) $ copilot")]),
        mono_line(vec![d("New agent. No history from this session.")]),
        mono_line(vec![user("> Look at layer 2 of the stack in the Diff panel.")]),
        mono_line(vec![user("  Don't edit files. Leave what you find on the lines.")]),
        mono_line(vec![]),
        mono_line(vec![plain("● humata · review_context")]),
        mono_line(vec![d("  ⎿ Layer 2 · dual write · api, ledger, sdk")]),
        mono_line(vec![plain("● Read(api/src/client.rs)")]),
        mono_line(vec![plain("● Read(ledger/src/posting.rs)")]),
        mono_line(vec![plain("● Grep(\"insert_legacy\")")]),
        mono_line(vec![d("  ⎿ 3 matches in 2 files")]),
        mono_line(vec![]),
        mono_line(vec![plain("The transaction closes the crash gap. One thing")]),
        mono_line(vec![plain("is still open: posting.rs:88 reads the balance")]),
        mono_line(vec![plain("outside the transaction, so two posts can both")]),
        mono_line(vec![plain("pass the overdraft check.")]),
        mono_line(vec![]),
        mono_line(vec![plain("● humata · review_note(ledger/src/posting.rs:88)")]),
        mono_line(vec![]),
        mono_line(vec![t(ORANGE, "✱ Checking the sdk types… (38s · esc to interrupt)")]),
    ];
    let tabs = vec![
        Tab { name: "claude", dot: Some(DOT_ORANGE), on: false },
        Tab { name: "zsh", dot: None, on: false },
        Tab { name: "copilot", dot: Some(DOT_GREEN), on: true },
    ];
    let rows: Rows = vec![
        (' ', 86, "    let tx = db.begin().await?;"),
        ('+', 87, "    tx.insert_entry(&e).await?;"),
        (' ', 88, "    let bal = db.balance(e.account).await?;"),
        (' ', 89, "    ensure!(bal >= Decimal::ZERO, Overdrawn);"),
    ];
    let note = note_card(
        look,
        Note::Plain,
        vec![who("copilot"), tag(look, Tone::Orange, "Note"), t(dim(look), "line 88")],
        text_only("The balance is read on db, not tx, so two posts can both pass the overdraft check. Read it through tx."),
        None,
        vec![strong_link(look, "Send to claude"), link(look, "Edit"), t(dim(look), "Drop").text_color(c(look.accent)).opacity(0.7)],
    );
    let body = div()
        .flex()
        .flex_col()
        .child(
            list_box(look)
                .flex_shrink_0()
                .child(cap(look, "STACK · 3 LAYERS"))
                .child(stack(look, 2, None, REPOS, [ok(look), Some(t(look.accent, "3 drafts · 1 note")), ok(look)], ["", "", ""]))
                .child(sep(look))
                .child(repo(look, "api", "", "+143", "−24"))
                .child(file(look, "client.rs", "src/", "+62", "−21", Ck::No, false, vec![count(dim(look), "3")]))
                .child(repo(look, "ledger", "", "+63", "−18"))
                .child(file(look, "posting.rs", "src/", "+63", "−18", Ck::No, true, vec![count(ORANGE, "1")])),
        )
        .child(file_bar(look, "ledger · src/", "posting.rs", false, open_in_editor(look)))
        .child(lines(look, "@@ -84,6 +84,7 @@ pub async fn post(", rows))
        .child(note);
    let bottom = foot().child(t(dim(look), "3 drafts · 1 note · 2 layers look good")).child(grow()).child(primary(look, "Send to claude"));
    frame(
        look,
        row()
            .items_stretch()
            .child(sidebar(look, DOT_ORANGE, None))
            .child(session_main(look, DOT_GREEN, &tabs, Some("2 agents"), out, ""))
            .child(panel(look, 520., "Layer 2 of 3", None, body, bottom))
            .child(rail(look, true, "Diff · 23")),
    )
}

// ---------- 4-1

pub fn agent_applies_comments(look: Look) -> Div {
    let d = |s| dimmed(look, s);
    let out = vec![
        mono_line(vec![user("> From the Diff panel · 4 comments, 2 layers look good")]),
        mono_line(vec![user("  Layer 1 · schema: looks good.   Layer 3 · reads: looks good.")]),
        mono_line(vec![user("  Layer 2 · dual write:")]),
        mono_line(vec![user("   api/src/client.rs:43     Make the legacy insert an upsert on entry.id.")]),
        mono_line(vec![user("   api/src/client.rs:51     One transaction for the whole batch.")]),
        mono_line(vec![user("   api/src/client.rs:60     Name the timeout.")]),
        mono_line(vec![user("   ledger/src/posting.rs:88 Read the balance through tx.  (from copilot)")]),
        mono_line(vec![]),
        mono_line(vec![plain("● Update(api/src/client.rs)")]),
        mono_line(vec![d("  ⎿ Updated with 19 additions and 8 removals")]),
        mono_line(vec![plain("● Update(ledger/src/posting.rs)")]),
        mono_line(vec![d("  ⎿ Updated with 2 additions and 2 removals")]),
        mono_line(vec![plain("● Bash(bazel test //api/... //ledger/...)")]),
        mono_line(vec![t(GREEN, "  ⎿ 33 tests pass")]),
        mono_line(vec![plain("● Bash(stack submit --all)")]),
        mono_line(vec![d("  ⎿ updated acme/api#482, acme/ledger#92 · rebased layer 3")]),
        mono_line(vec![plain("● humata · comment_resolve × 4")]),
        mono_line(vec![]),
        mono_line(vec![plain("All four are done, each with a reply on its line.")]),
        mono_line(vec![plain("Layers 1 and 3 didn't change.")]),
    ];
    let tabs = vec![
        Tab { name: "claude", dot: Some(DOT_ORANGE), on: true },
        Tab { name: "zsh", dot: None, on: false },
        Tab { name: "copilot", dot: Some(GREY), on: false },
    ];
    let changed = Some(t(ORANGE, "● Changed since you looked"));
    let body = list_box(look)
        .flex_1()
        .child(cap(look, "STACK · 3 LAYERS"))
        .child(stack(look, 0, None, ["ledger · infra", "3 repos", "api · web · sdk"], [ok(look), changed, ok(look)], ["", "", ""]))
        .child(sep(look))
        .child(all_files(look, (Ck::Changed, vec![done_count("✓ 3 done")], "+73", "−21"), (Ck::Changed, vec![done_count("✓ 1 done")]), true, None));
    let bottom = foot().child(t(dim(look), "4 comments done · 2 files changed")).child(grow()).child(link(look, "Show what changed"));
    frame(
        look,
        row()
            .items_stretch()
            .child(sidebar(look, DOT_GREEN, Some("2 changed")))
            .child(session_main(look, DOT_GREEN, &tabs, None, out, ""))
            .child(panel(look, 500., "", None, body, bottom))
            .child(rail(look, true, "Diff · 2")),
    )
}

fn seg(look: Look, left: &'static str, right: &'static str, on_right: bool) -> Div {
    let item = |text: &'static str, on: bool| {
        row()
            .px(px(10.))
            .py(px(3.))
            .rounded(px(6.))
            .whitespace_nowrap()
            .when(on, |s| s.bg(c(look.card)).border_1().border_color(c(look.line)).font_weight(FontWeight::MEDIUM))
            .child(text)
    };
    row().p(px(1.)).rounded(px(7.)).bg(c(look.fill)).border_1().border_color(c(look.line)).child(item(left, !on_right)).child(item(right, on_right))
}

// ---------- 4-2

pub fn manual_diff_since_last_look(look: Look) -> Div {
    let changed = Some(t(ORANGE, "● Changed since you looked"));
    let left = list_box(look)
        .child(cap(look, "2 FILES CHANGED SINCE YOU LOOKED"))
        .child(repo(look, "api", "against main", "+11", "−3"))
        .child(file(look, "client.rs", "src/", "+11", "−3", Ck::Changed, true, vec![done_count("✓ 3 done")]))
        .child(repo(look, "ledger", "against release/2.4", "+2", "−2"))
        .child(file(look, "posting.rs", "src/", "+2", "−2", Ck::Changed, false, vec![done_count("✓ 1 done")]))
        .child(row().px(px(10.)).py(px(5.)).text_color(c(dim(look))).child("infra, sdk, web · nothing changed since you looked"))
        .child(sep(look))
        .child(cap(look, "STACK · 3 LAYERS"))
        .child(stack(look, 9, None, ["ledger · infra", "3 repos", "api · web · sdk"], [ok(look), changed, ok(look)], ["", "", ""]));
    let first: Rows = vec![
        (' ', 41, "        let tx = self.db.begin().await?;"),
        (' ', 42, "        tx.insert_entry(&entry).await?;"),
        ('-', 43, "        tx.insert_legacy(&entry).await?;"),
        ('+', 43, "        tx.upsert_legacy(&entry).await?;"),
        (' ', 44, "        tx.commit().await?;"),
    ];
    let second: Rows = vec![
        (' ', 49, "    pub async fn post_many(&self, entries: &[Entry]) -> Result<()> {"),
        ('-', 50, "        for e in entries {"),
        ('-', 51, "            self.post(e.clone()).await?;"),
        ('-', 52, "        }"),
        ('+', 50, "        let tx = self.db.begin().await?;"),
        ('+', 51, "        for e in entries {"),
        ('+', 52, "            tx.insert_entry(e).await?;"),
        ('+', 53, "            tx.upsert_legacy(e).await?;"),
        ('+', 54, "        }"),
        ('+', 55, "        tx.commit().await?;"),
    ];
    let done = |line: &'static str, text: &'static str, reply: &'static str| {
        note_card(
            look,
            Note::Done,
            vec![who("You"), tag(look, Tone::Green, "Done by claude"), t(dim(look), line), grow(), link(look, "Reopen")],
            text_only(text),
            Some(("claude", reply)),
            vec![],
        )
    };
    let code = div()
        .flex()
        .flex_col()
        .child(
            file_bar_note(look, "api · src/", "client.rs", false, Some("· changes since you looked, 12 min ago"), open_in_editor(look)),
        )
        .child(lines(look, "@@ -41,4 +41,4 @@ impl Client {", first))
        .child(done("line 43", "Make the legacy insert an upsert on entry.id.", "upsert_legacy keys on entry.id, so a retried commit writes the row once."))
        .child(lines(look, "@@ -49,4 +49,7 @@", second))
        .child(done("line 51", "One transaction for the whole batch, not one per entry.", "post_many opens one transaction and commits once."));
    let bottom = foot()
        .child(t(dim(look), "File 1 of 2"))
        .child(link(look, "Next file ›"))
        .child(grow())
        .child(t(dim(look), "4 comments done · 0 open"))
        .child(button14(look, "Looks good"));
    let title = seg(look, "All changes", "Since I last looked · 2 files", true);
    frame(look, full(look, title, full_buttons(look), left, code, bottom))
}

// ---------- 4-3

pub fn everything_looks_good(look: Look) -> Div {
    let left = list_box(look)
        .child(cap(look, "STACK · 3 LAYERS"))
        .child(
            div()
                .flex()
                .flex_col()
                .child(layer(look, None, "Whole stack", "5 repos · 23 files", "", true, ok(look)))
                .child(layer(look, Some("1"), "schema", "ledger#91 · infra#40", "", false, ok(look)))
                .child(layer(look, Some("2"), "dual write", "api#482 · ledger#92 · sdk#17", "", false, ok(look)))
                .child(layer(look, Some("3"), "reads", "api#483 · web#210 · sdk#18", "", false, ok(look))),
        )
        .child(sep(look))
        .child(all_files(look, (Ck::Done, vec![], "+73", "−21"), (Ck::Done, vec![]), true, Some("client.rs")));
    let rows: Rows = vec![
        (' ', 40, "    pub async fn post(&self, entry: Entry) -> Result<()> {"),
        ('-', 41, "        self.db.insert_legacy(&entry).await?;"),
        ('+', 41, "        let tx = self.db.begin().await?;"),
        ('+', 42, "        tx.insert_entry(&entry).await?;"),
        ('+', 43, "        tx.upsert_legacy(&entry).await?;"),
        ('+', 44, "        tx.commit().await?;"),
    ];
    let folded = row()
        .mx(px(12.))
        .my(px(10.))
        .px(px(12.))
        .py(px(7.))
        .gap(px(6.))
        .rounded(px(8.))
        .border_1()
        .border_color(c(look.line))
        .bg(c(look.panel))
        .text_color(c(look.secondary))
        .child(t(GREEN, "✓"))
        .child("3 comments done on this file ")
        .child(link(look, "Show"));
    let code = div()
        .flex()
        .flex_col()
        .child(file_bar(look, "api · src/", "client.rs", false, t(GREEN, "✓ You looked at this")))
        .child(lines(look, "@@ -40,7 +40,12 @@ impl Client {", rows))
        .child(folded);
    let bottom = foot()
        .child(t(GREEN, "✓ All 3 layers look good · 4 comments done"))
        .child(grow())
        .child(t(dim(look), "8 pull requests are up to date"))
        .child(primary(look, "Open the pull requests"));
    let title = seg(look, "All changes", "Since I last looked · 0 files", false);
    frame(look, full(look, title, full_buttons(look), left, code, bottom))
}
