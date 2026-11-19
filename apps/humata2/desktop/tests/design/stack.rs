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
/// in tertiary; the picked row has a fill and its purpose under it.
fn layer_row(look: Look, number: &'static str, title: &'static str, note: &'static str, purpose: Option<&'static str>) -> Div {
    let on = purpose.is_some();
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
                .child(
                    div()
                        .w(px(16.))
                        .h(px(16.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .border_1()
                        .border_color(c(look.line))
                        .text_size(px(10.))
                        .text_color(c(look.tertiary))
                        .child(number)
                        .when(number.is_empty(), |mark| mark.child(small_icon(IconName::FolderTree, look.tertiary))),
                )
                .child(div().flex_shrink_0().font_weight(if on || number.is_empty() { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).child(title))
                .child(div().min_w_0().overflow_hidden().text_ellipsis().text_size(px(11.5)).text_color(c(look.tertiary)).child(note)),
        )
        .children(purpose.map(|text| div().pl(px(22.)).pb(px(4.)).text_size(px(11.5)).text_color(c(look.secondary)).child(text)))
}

fn whole_row(look: Look, on: bool) -> Div {
    layer_row(look, "", "Whole stack", "2 repos \u{b7} 14 files", on.then_some("Journal compression, in 3 layers over api and web"))
}

fn repo_head(look: Look, name: &'static str, add: &'static str, del: &'static str) -> Div {
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
        .child(div().flex_1())
        .child(div().text_size(px(11.)).text_color(c(GREEN)).child(add))
        .child(div().text_size(px(11.)).text_color(c(RED)).child(del))
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
        .child(div().w(px(8.)).flex_shrink_0().flex().justify_center().when(key, |dot| dot.child(div().size(px(6.)).rounded_full().bg(c(look.accent)))))
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

fn quiet_line(look: Look, text: &'static str) -> Div {
    secondary(look, text).px(px(8.)).py(px(2.))
}

fn drift_line(look: Look) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(28.))
        .pr(px(8.))
        .py(px(2.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(secondary(look, "web moved since it was sent").whitespace_nowrap())
                .child(div().text_size(px(11.5)).whitespace_nowrap().text_color(c(look.accent)).child("Ask the agent to send it again")),
        )
}

fn diff_view(look: Look, file: &'static str) -> Div {
    let line = |n: &'static str, bg: Option<u32>, text: &'static str| {
        div()
            .flex()
            .when_some(bg, |row, bg| row.bg(c(bg)))
            .child(div().w(px(34.)).pr(px(8.)).text_color(c(look.term_dim)).flex().justify_end().child(n))
            .child(div().whitespace_nowrap().text_color(c(look.term_text)).child(text))
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
                .child(small_icon(IconName::FileCode, look.secondary))
                .child(secondary(look, "api \u{b7} src/codec/"))
                .child(div().font_weight(FontWeight::MEDIUM).child(file)),
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
                .child(line("1", Some(look.add_bg), "+use zstd::stream::{encode_all, decode_all};"))
                .child(line("2", Some(look.add_bg), "+"))
                .child(line("3", Some(look.add_bg), "+pub struct Zstd { level: i32 }"))
                .child(line("4", Some(look.add_bg), "+"))
                .child(line("5", Some(look.add_bg), "+impl Codec for Zstd {")),
        )
}

fn footer(look: Look) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .p(px(10.))
        .border_t_1()
        .border_color(c(look.line))
        .child(session::quiet(look, "Push & PR\u{2026}"))
        .child(session::quiet(look, "Add repo\u{2026}"))
}

fn body(look: Look, header: &'static str, top: Div, files: Div, file: &'static str) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(session::panel_header(look, "Diff", false, Some(secondary(look, header))))
        .child(div().flex().flex_col().px(px(8.)).pt(px(8.)).gap(px(2.)).child(top))
        .child(div().flex().flex_col().p(px(8.)).gap(px(1.)).text_size(px(12.5)).child(files))
        .child(diff_view(look, file))
        .child(footer(look))
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

/// Frame 119 (light): Whole stack picked.
pub fn whole(look: Look) -> Div {
    let files = div()
        .flex()
        .flex_col()
        .child(repo_head(look, "api", "+486", "\u{2212}61"))
        .child(file_row(look, "src/codec/", "zstd.rs", true, "+212", "\u{2212}0", true))
        .child(file_row(look, "src/journal/", "pages.rs", true, "+148", "\u{2212}52", false))
        .child(file_row(look, "src/journal/", "writer.rs", false, "+61", "\u{2212}9", false))
        .child(skim_row(look, false, "3 files to skim \u{b7} Cargo.lock, mod.rs, README.md"))
        .child(repo_head(look, "web", "+39", "\u{2212}6"))
        .child(file_row(look, "src/lib/", "codec.ts", true, "+31", "\u{2212}6", false))
        .child(skim_row(look, false, "1 file to skim \u{b7} package-lock.json"));
    frame(look, body(look, "Whole stack \u{b7} 3 layers", rows(look, 0, false), files, "zstd.rs"))
}

/// Frame 120 (light): layer 2 "journal" picked; web isn't in it.
pub fn layer(look: Look) -> Div {
    let files = div()
        .flex()
        .flex_col()
        .child(repo_head(look, "api", "+209", "\u{2212}61"))
        .child(file_row(look, "src/journal/", "pages.rs", true, "+148", "\u{2212}52", true))
        .child(file_row(look, "src/journal/", "writer.rs", false, "+61", "\u{2212}9", false))
        .child(skim_row(look, false, "3 files to skim \u{b7} Cargo.lock, mod.rs, README.md"))
        .child(quiet_line(look, "web isn't in this layer"));
    frame(look, body(look, "Layer 2 of 3 \u{b7} journal", rows(look, 2, false), files, "pages.rs"))
}

/// Frame 121 (dark): layer 1 picked, web moved since it was sent, api's
/// skim files opened in place.
pub fn skim_drift(look: Look) -> Div {
    let files = div()
        .flex()
        .flex_col()
        .child(repo_head(look, "api", "+277", "\u{2212}0"))
        .child(file_row(look, "src/codec/", "zstd.rs", true, "+212", "\u{2212}0", true))
        .child(file_row(look, "src/journal/", "writer.rs", false, "+61", "\u{2212}9", false))
        .child(skim_row(look, true, "3 files to skim \u{b7} Cargo.lock, codec.rs, README.md"))
        .child(file_row(look, "", "Cargo.lock", false, "+31", "\u{2212}0", false))
        .child(file_row(look, "", "codec.rs", false, "+14", "\u{2212}0", false))
        .child(file_row(look, "", "README.md", false, "+3", "\u{2212}0", false))
        .child(repo_head(look, "web", "+39", "\u{2212}6"))
        .child(file_row(look, "src/lib/", "codec.ts", true, "+31", "\u{2212}6", false));
    frame(look, body(look, "Layer 1 of 3 \u{b7} codec", rows(look, 1, true), files, "zstd.rs"))
}
