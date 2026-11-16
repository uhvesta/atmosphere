//! The file manager (frame 61; SPEC FM-1–FM-3, FM-5–FM-8, FM-12–FM-14): two
//! panes with a machine picker each, the actions between them, and the
//! transfers tray.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::{page, toolbar};
use crate::sidebar::Side;

const SIZE_W: f32 = 80.;
const WHEN_W: f32 = 112.;

/// One listing row: icon, name, size and modified time.
fn entry(look: Look, indent: f32, glyph: IconName, name: &'static str, size: &'static str, when: &'static str, on: bool) -> Div {
    let (text, dim) = if on { (look.selection_text, look.selection_text) } else { (look.text, look.secondary) };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(26.))
        .pl(px(12. + indent))
        .pr(px(12.))
        .text_size(px(12.5))
        .when(on, |row| row.bg(c(look.selection)))
        .child(small_icon(glyph, dim))
        .child(div().flex_1().min_w_0().whitespace_nowrap().text_color(c(text)).child(name))
        .child(div().w(px(SIZE_W)).flex_shrink_0().flex().justify_end().whitespace_nowrap().text_color(c(dim)).child(size))
        .child(div().w(px(WHEN_W)).flex_shrink_0().flex().justify_end().whitespace_nowrap().text_color(c(dim)).child(when))
}

/// The column titles above a listing.
fn columns(look: Look) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .px(px(12.))
        .bg(c(look.panel))
        .border_b_1()
        .border_color(c(look.line))
        .child(div().w(px(15.)).flex_shrink_0())
        .child(div().flex_1().child(label(look, "Name")))
        .child(div().w(px(SIZE_W)).flex_shrink_0().flex().justify_end().child(label(look, "Size")))
        .child(div().w(px(WHEN_W)).flex_shrink_0().flex().justify_end().child(label(look, "Modified")))
}

/// A pane's header: its machine picker and the folder it shows.
fn pane_head(look: Look, machine: &'static str, path: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(40.))
        .px(px(12.))
        .border_b_1()
        .border_color(c(look.line))
        .child(popup(look, machine))
        .child(div().flex_1().min_w_0().whitespace_nowrap().text_size(px(12.5)).text_color(c(look.secondary)).child(path))
}

fn pane(look: Look, head: Div, rows: Vec<Div>) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h_full()
        .flex()
        .flex_col()
        .child(head)
        .child(columns(look))
        .child(div().flex().flex_col().children(rows))
}

fn bar(look: Look, done: f32, hex: u32) -> Div {
    div().w(px(140.)).h(px(5.)).flex_shrink_0().rounded_full().bg(c(look.fill)).child(div().w(px(140. * done)).h_full().rounded_full().bg(c(hex)))
}

/// One transfer: source → destination, progress, state, and what it needs.
fn transfer(look: Look, route: &'static str, amount: &'static str, done: f32, hex: u32, state: &'static str, note: &'static str, action: Option<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .px(px(16.))
        .py(px(8.))
        .border_b_1()
        .border_color(c(look.line))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .text_size(px(12.5))
                        .child(div().w(px(220.)).flex_shrink_0().whitespace_nowrap().font_weight(FontWeight::MEDIUM).child(route))
                        .child(div().w(px(100.)).flex_shrink_0().whitespace_nowrap().text_color(c(look.secondary)).child(amount))
                        .child(bar(look, done, hex))
                        .child(div().flex_1().min_w_0().whitespace_nowrap().text_color(c(hex)).child(state)),
                )
                .child(div().whitespace_nowrap().text_size(px(11.5)).text_color(c(look.secondary)).child(note)),
        )
        .child(div().w(px(70.)).flex_shrink_0().flex().justify_end().children(action))
}

/// The file manager with This Mac on the left and devbox-1 on the right; a
/// paused transfer waits for gpu-box, which is offline.
pub fn file_manager(look: Look) -> Div {
    let left = pane(
        look,
        pane_head(look, "This Mac", "~/Downloads/fixtures"),
        vec![
            entry(look, 0., IconName::ChevronRight, "..", "", "", false),
            entry(look, 0., IconName::FolderOpen, "datasets/", "2.1 GB", "Today 09:12", true),
            entry(look, 18., IconName::FileText, "train.parquet", "1.8 GB", "Today 09:12", false),
            entry(look, 18., IconName::FileText, "eval.parquet", "310 MB", "Today 09:12", false),
            entry(look, 0., IconName::FileText, "README.md", "2 KB", "Mon 16:40", false),
        ],
    );
    let drop_hint = div()
        .mx(px(12.))
        .mt(px(10.))
        .h(px(40.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .bg(c(look.panel))
        .border_1()
        .border_color(c(look.line))
        .text_size(px(12.))
        .text_color(c(look.secondary))
        .child("Drop here, or drag from Finder");
    let right = pane(
        look,
        pane_head(look, "devbox-1", "~/Humata/payments-rewrite/shared"),
        vec![
            entry(look, 0., IconName::ChevronRight, "..", "", "", false),
            entry(look, 0., IconName::Folder, "weights/", "—", "Sun 21:03", false),
            entry(look, 0., IconName::FileText, "notes.md", "4 KB", "Sun 21:05", false),
        ],
    )
    .child(drop_hint);

    let actions = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(42.))
        .px(px(16.))
        .flex_shrink_0()
        .border_t_1()
        .border_b_1()
        .border_color(c(look.line))
        .child(button(look, Kind::Secondary, "New folder"))
        .child(button(look, Kind::Secondary, "Rename"))
        .child(button(look, Kind::Primary, "Copy →"))
        .child(button(look, Kind::Secondary, "Move →"))
        .child(button(look, Kind::Secondary, "Delete"))
        .child(div().flex_1())
        .child(secondary(look, "Trash on This Mac").whitespace_nowrap())
        .child(badge(look, "3", false));

    let tray = div()
        .flex()
        .flex_col()
        .flex_shrink_0()
        .bg(c(look.panel))
        .child(
            div()
                .flex()
                .items_center()
                .h(px(30.))
                .px(px(16.))
                .border_b_1()
                .border_color(c(look.line))
                .child(label(look, "Transfers"))
                .child(div().flex_1())
                .child(secondary(look, "over each machine's SSH link").whitespace_nowrap()),
        )
        .child(transfer(
            look,
            "datasets/ → devbox-1",
            "1.4 / 2.1 GB",
            0.67,
            look.accent,
            "Copying",
            "Resumable · checksummed per chunk",
            Some(button(look, Kind::Secondary, "Cancel")),
        ))
        .child(transfer(
            look,
            "datasets/ → gpu-box",
            "0.6 / 2.1 GB",
            0.29,
            ORANGE,
            "Paused: gpu-box is offline",
            "Resumes on reconnect, from the last confirmed offset · checksummed per chunk",
            Some(button(look, Kind::Secondary, "Cancel")),
        ))
        .child(transfer(
            look,
            "notes.md → This Mac",
            "4 KB",
            1.,
            GREEN,
            "✓ Done",
            "Checksum ok",
            None,
        ));

    page(
        look,
        Side::at("This Mac"),
        toolbar(look, "Files", None, vec![]),
        div()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().flex().child(left).child(handle(look, false)).child(right))
            .child(actions)
            .child(tray),
    )
}
