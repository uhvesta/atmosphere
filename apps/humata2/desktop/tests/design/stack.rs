//! Frames 119 to 121 (STACKVIEW): the Diff panel reviewing a stack of three
//! layers over two repos, beside the terminal. 119 has Whole stack picked,
//! 120 layer 2 "journal" picked (only api shown, web gets its quiet line),
//! 121 the skim fold open in dark with a repo that moved since it was sent.
//! Rows mirror `ui/changes/src/panel_stack.rs`.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::session::{self, Panel, Screen, Term};

/// One layer row: number (or the whole-stack mark), title, what it touches
/// in tertiary; the picked row has a fill, a filled badge and its purpose.
fn layer_row(look: Look, number: &'static str, title: &'static str, note: &'static str, purpose: Option<&'static str>) -> Div {
    let on = purpose.is_some();
    let badge = div()
        .w(px(16.))
        .h(px(16.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .text_size(px(10.));
    let badge = if on {
        badge.bg(c(look.accent)).text_color(c(0xffffff)).font_weight(FontWeight::SEMIBOLD).child(if number.is_empty() { "\u{2261}" } else { number })
    } else {
        badge
            .border_1()
            .border_color(c(look.line))
            .text_color(c(look.tertiary))
            .child(if number.is_empty() { "\u{2261}" } else { number })
    };
    div()
        .flex()
        .flex_col()
        .px(px(8.))
        .rounded(px(5.))
        .when(on, |row| row.bg(c(look.fill)))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .h(px(25.))
                .whitespace_nowrap()
                .child(badge)
                .child(div().flex_shrink_0().font_weight(if on || number.is_empty() { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).child(title))
                .child(div().min_w_0().overflow_hidden().text_ellipsis().text_size(px(11.5)).text_color(c(look.tertiary)).child(note)),
        )
        .children(purpose.filter(|text| !text.is_empty()).map(|text| div().pl(px(22.)).pb(px(4.)).text_size(px(11.5)).text_color(c(look.secondary)).child(text)))
}

fn whole_row(look: Look, on: bool) -> Div {
    layer_row(look, "", "Whole stack", "2 repos \u{b7} 8 files", on.then_some(""))
}

fn repo_head(look: Look, name: &'static str, against: bool, add: &'static str, del: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .px(px(4.))
        .pr(px(8.))
        .whitespace_nowrap()
        .child(div().w(px(20.)).h(px(20.)).flex_shrink_0().flex().items_center().justify_center().child(small_icon(IconName::ChevronDown, look.tertiary)))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
        .when(against, |row| row.child(secondary(look, "against main").text_size(px(11.5))))
        .child(div().flex_1())
        .child(div().text_size(px(11.)).text_color(c(GREEN)).child(add))
        .child(div().text_size(px(11.)).text_color(c(RED)).child(del))
}

fn key_chip(look: Look) -> Div {
    let mut fill = c(look.accent);
    fill.a = 0.14;
    div().flex_shrink_0().px(px(5.)).rounded(px(4.)).bg(fill).text_size(px(10.5)).text_color(c(look.accent)).child("key")
}

fn file_row(look: Look, dir: &'static str, name: &'static str, key: bool, add: &'static str, del: &'static str, on: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(25.))
        .pl(px(22.))
        .pr(px(8.))
        .rounded(px(5.))
        .when(on, |row| row.bg(c(look.fill)))
        .child(div().size(px(14.)).flex_shrink_0().rounded_full().border_1().border_color(c(look.line)))
        .child(
            div()
                .flex()
                .items_center()
                .flex_1()
                .min_w_0()
                .gap(px(6.))
                .child(div().flex_shrink_0().child(name))
                .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_color(c(look.tertiary)).child(dir)),
        )
        .when(key, |row| row.child(key_chip(look)))
        .child(div().flex_shrink_0().text_size(px(11.)).text_color(c(GREEN)).child(add))
        .child(div().flex_shrink_0().text_size(px(11.)).text_color(c(RED)).child(del))
}

fn skim_row(look: Look, open: bool, text: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(25.))
        .pl(px(22.))
        .pr(px(8.))
        .child(small_icon(if open { IconName::ChevronDown } else { IconName::ChevronRight }, look.secondary))
        .child(secondary(look, text).min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis())
}

/// Aligned with the file names (22 pt indent, 14 pt tick, 6 pt gap).
fn quiet_line(look: Look, text: &'static str) -> Div {
    div().pl(px(42.)).pr(px(8.)).py(px(2.)).text_size(px(12.)).text_color(c(look.tertiary)).child(text)
}

fn drift_line(look: Look) -> Div {
    div().px(px(0.)).py(px(2.)).child(
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_between()
            .gap(px(6.))
            .px(px(8.))
            .py(px(5.))
            .rounded(px(6.))
            .bg(c(look.card))
            .child(secondary(look, "web moved since it was sent").whitespace_nowrap())
            .child(button(look, Kind::Secondary, "Ask the agent to send it again")),
    )
}

/// One diff row: old and new numbers (empty when absent), sign, and the code
/// as (text, highlighted) pieces.
struct Ln {
    old: &'static str,
    new: &'static str,
    sign: &'static str,
    parts: Vec<(&'static str, bool)>,
}

fn ln(old: &'static str, new: &'static str, sign: &'static str, parts: Vec<(&'static str, bool)>) -> Ln {
    Ln { old, new, sign, parts }
}

fn diff_view(look: Look, dir: &'static str, file: &'static str, hunk: Option<&'static str>, lines: Vec<Ln>) -> Div {
    let two = lines.iter().any(|l| !l.old.is_empty());
    let line = move |l: Ln| {
        let bg = match l.sign {
            "+" => Some(look.add_bg),
            "\u{2212}" => Some(look.del_bg),
            _ => None,
        };
        let mut hl = c(if l.sign == "+" { GREEN } else { RED });
        hl.a = 0.3;
        let sign_color = match l.sign {
            "+" => GREEN,
            "\u{2212}" => RED,
            _ => look.term_dim,
        };
        let num = |n: &'static str| div().w(px(30.)).flex_shrink_0().pr(px(6.)).text_color(c(look.term_dim)).flex().justify_end().child(n);
        div()
            .flex()
            .overflow_hidden()
            .when_some(bg, |row, bg| row.bg(c(bg)))
            .when(two, |row| row.child(num(l.old)))
            .child(num(l.new))
            .child(div().w(px(14.)).flex_shrink_0().text_color(c(sign_color)).child(l.sign))
            .child(
                div().flex().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().mr(px(14.)).text_color(c(look.term_text)).children(
                    l.parts.into_iter().map(move |(text, on)| div().flex_shrink_0().when(on, |p| p.bg(hl)).child(text)),
                ),
            )
    };
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .h(px(30.))
                .border_y_1()
                .border_color(c(look.line))
                .text_size(px(12.))
                .child(secondary(look, dir))
                .child(div().font_weight(FontWeight::MEDIUM).child(file))
                .child(div().flex_1())
                .child(secondary(look, "Open in editor")),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .py(px(6.))
                .font_family(MONO)
                .text_size(px(11.5))
                .line_height(px(17.))
                .bg(c(look.term))
                .when_some(hunk, |col, label| col.child(div().px(px(10.)).mb(px(2.)).bg(c(look.fill)).text_color(c(look.term_dim)).child(label)))
                .children(lines.into_iter().map(line)),
        )
}

fn link(look: Look, text: &'static str) -> Div {
    div().flex_shrink_0().whitespace_nowrap().text_size(px(12.)).text_color(c(look.accent)).child(text)
}

fn footer(look: Look, whole: bool) -> Div {
    let bar = div().flex().items_center().gap(px(12.)).p(px(10.)).border_t_1().border_color(c(look.line));
    if whole {
        bar.child(link(look, "3 pull requests open"))
            .child(div().flex_1())
            .child(button(look, Kind::Secondary, "Send to claude").text_color(c(look.tertiary)))
    } else {
        bar.child(link(look, "\u{2039} Previous"))
            .child(link(look, "Next layer \u{203a}"))
            .child(div().flex_1())
            .child(button(look, Kind::Secondary, "Looks good"))
    }
}

fn body(look: Look, header: &'static str, top: Div, files: Div, diff: Div, whole: bool) -> Div {
    let caption = div().px(px(8.)).pb(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(look.secondary)).child("STACK \u{b7} 3 LAYERS");
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(session::panel_header(look, "Diff", false, Some(secondary(look, header))))
        .child(div().flex().flex_col().px(px(8.)).pt(px(8.)).gap(px(2.)).child(caption).child(top))
        .child(div().flex().flex_col().p(px(8.)).gap(px(1.)).text_size(px(12.5)).child(files))
        .child(diff)
        .child(footer(look, whole))
}

fn frame(look: Look, body: Div) -> Div {
    session::session_with(look, Screen::new(Some(Panel::Diff), Term::Working), Panel::Diff, "Diff", body)
}

fn rows(look: Look, picked: usize, drift: bool) -> Div {
    let p = |at: usize, text: &'static str| (picked == at).then_some(text);
    div()
        .flex()
        .flex_col()
        .child(whole_row(look, picked == 0))
        .child(layer_row(look, "3", "Not committed yet", "api", p(3, "Work still in the api worktree")))
        .child(layer_row(look, "2", "journal", "api", p(2, "Writes journal pages through the codec")))
        .child(layer_row(look, "1", "codec", "api \u{b7} web", p(1, "Adds the zstd codec to api and web")))
        .when(drift, |list| list.child(drift_line(look)))
}

fn zstd_lines() -> Vec<Ln> {
    vec![
        ln("", "1", "+", vec![("use zstd::stream::{encode_all, decode_all};", false)]),
        ln("", "2", "+", vec![("", false)]),
        ln("", "3", "+", vec![("pub struct Zstd { level: i32 }", false)]),
        ln("", "4", "+", vec![("", false)]),
        ln("", "5", "+", vec![("impl Codec for Zstd {", false)]),
    ]
}

/// Frame 119 (light): Whole stack picked.
pub fn whole(look: Look) -> Div {
    let files = div()
        .flex()
        .flex_col()
        .child(repo_head(look, "api", true, "+486", "\u{2212}61"))
        .child(file_row(look, "src/codec/", "zstd.rs", true, "+212", "\u{2212}0", true))
        .child(file_row(look, "src/journal/", "pages.rs", true, "+148", "\u{2212}52", false))
        .child(file_row(look, "src/journal/", "writer.rs", false, "+61", "\u{2212}9", false))
        .child(skim_row(look, false, "3 files to skim \u{b7} Cargo.lock, mod.rs, README.md"))
        .child(repo_head(look, "web", true, "+39", "\u{2212}6"))
        .child(file_row(look, "src/lib/", "codec.ts", true, "+31", "\u{2212}6", false))
        .child(skim_row(look, false, "1 file to skim \u{b7} package-lock.json"));
    let diff = diff_view(look, "api \u{b7} src/codec/", "zstd.rs", None, zstd_lines());
    frame(look, body(look, "Whole stack \u{b7} 3 layers", rows(look, 0, false), files, diff, true))
}

/// Frame 120 (light): layer 2 "journal" picked; web isn't in it.
pub fn layer(look: Look) -> Div {
    let files = div()
        .flex()
        .flex_col()
        .child(repo_head(look, "api", false, "+209", "\u{2212}61"))
        .child(file_row(look, "src/journal/", "pages.rs", true, "+148", "\u{2212}52", true))
        .child(file_row(look, "src/journal/", "writer.rs", false, "+61", "\u{2212}9", false))
        .child(skim_row(look, false, "3 files to skim \u{b7} Cargo.lock, mod.rs, README.md"))
        .child(quiet_line(look, "web isn't in this layer"));
    let m = "\u{2212}";
    let lines = vec![
        ln("40", "40", " ", vec![("    pub fn write(&mut self, entry: &Entry) -> Result<()> {", false)]),
        ln("41", "", m, vec![("        self.buf.extend_from_slice(", false), ("&entry.bytes()", true), (");", false)]),
        ln("", "41", "+", vec![("        let packed = self.", false), ("codec.encode(&entry.bytes())?", true), (";", false)]),
        ln("", "42", "+", vec![("        self.buf.extend_from_slice(&packed);", false)]),
        ln("42", "43", " ", vec![("        self.len += 1;", false)]),
        ln("43", "44", " ", vec![("        Ok(())", false)]),
        ln("44", "45", " ", vec![("    }", false)]),
    ];
    let diff = diff_view(look, "api \u{b7} src/journal/", "pages.rs", Some("impl Page"), lines);
    frame(look, body(look, "Layer 2 of 3 \u{b7} journal", rows(look, 2, false), files, diff, false))
}

/// Frame 121 (dark): layer 1 picked, web moved since it was sent, api's
/// skim files opened in place.
pub fn skim_drift(look: Look) -> Div {
    let files = div()
        .flex()
        .flex_col()
        .child(repo_head(look, "api", false, "+260", "\u{2212}0"))
        .child(file_row(look, "src/codec/", "zstd.rs", true, "+212", "\u{2212}0", true))
        .child(skim_row(look, true, "3 files to skim \u{b7} Cargo.lock, codec.rs, README.md"))
        .child(file_row(look, "", "Cargo.lock", false, "+31", "\u{2212}0", false))
        .child(file_row(look, "", "codec.rs", false, "+14", "\u{2212}0", false))
        .child(file_row(look, "", "README.md", false, "+3", "\u{2212}0", false))
        .child(repo_head(look, "web", false, "+39", "\u{2212}6"))
        .child(file_row(look, "src/lib/", "codec.ts", true, "+31", "\u{2212}6", false))
        .child(skim_row(look, false, "1 file to skim \u{b7} package-lock.json"));
    let diff = diff_view(look, "api \u{b7} src/codec/", "zstd.rs", None, zstd_lines());
    frame(look, body(look, "Layer 1 of 3 \u{b7} codec", rows(look, 1, true), files, diff, false))
}
