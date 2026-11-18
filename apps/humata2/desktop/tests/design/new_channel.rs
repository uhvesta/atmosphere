//! The New channel dialog and its cloning progress (frames 44 and 45; SPEC
//! CH-1 to CH-5 and CH-10), drawn over the home list.

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::overlays::scrim;
use crate::pages;

/// The dialog's card, centred under the top of the window.
fn dialog(look: Look, under: Div, body: Div) -> Div {
    under.relative().child(scrim(look)).child(
        div().absolute().top(px(50.)).left_0().w_full().flex().justify_center().child(
            body.w(px(560.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(20.))
                .rounded(px(12.))
                .bg(c(look.card))
                .border_1()
                .border_color(c(look.line))
                .shadow_lg()
                .text_size(px(12.5)),
        ),
    )
}

fn title(text: &'static str) -> Div {
    div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(text)
}

fn row(look: Look, key: &'static str, value: Div) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(div().w(px(110.)).pt(px(4.)).flex().justify_end().flex_shrink_0().text_color(c(look.secondary)).child(key))
        .child(value.flex_1())
}

/// A ticked or unticked repo, with its icon.
fn repo(look: Look, on: bool, name: &'static str) -> Div {
    div().flex().items_center().gap(px(8.)).child(checkbox(look, on, "")).child(small_icon(IconName::FolderGit2, look.secondary)).child(name)
}

/// A repo source chip (REPOS): the picked one is filled and can be removed.
fn source(look: Look, text: &'static str, on: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .h(px(22.))
        .px(px(9.))
        .rounded_full()
        .border_1()
        .border_color(c(look.line))
        .text_size(px(12.))
        .when(on, |chip| chip.bg(c(look.fill)))
        .child(text)
        .when(on, |chip| chip.child(small_icon(IconName::X, look.secondary)))
}

fn machine(look: Look, on: bool, name: &'static str, note: &'static str, choosable: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .when(!choosable, |row| row.opacity(0.5))
        .child(checkbox(look, on, ""))
        .child(small_icon(IconName::Server, look.secondary))
        .child(name)
        .child(secondary(look, note))
}

/// Frame 44: the dialog before Create (CH-1 to CH-4, CH-10).
pub fn create(look: Look) -> Div {
    let body = div()
        .child(title("New channel"))
        .child(row(
            look,
            "Name",
            div().flex().flex_col().gap(px(5.)).child(field(look, "Billing exports", false)).child(secondary(look, "On: This Mac, devbox-1")),
        ))
        .child(row(
            look,
            "Repos",
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(source(look, "Your repos", true))
                        .child(source(look, "+ Org", false))
                        .child(source(look, "All of GitHub", false)),
                )
                .child(div().flex().items_center().gap(px(8.)).child(small_icon(IconName::Search, look.secondary)).child(field(look, "acme", false).flex_1()))
                .child(div().text_size(px(11.)).text_color(c(look.tertiary)).child("Your library first, then GitHub. Zero or more."))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(7.))
                        .child(repo(look, true, "acme/ledger"))
                        .child(repo(look, true, "acme/api"))
                        .child(div().pt(px(4.)).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(c(look.tertiary)).child("GitHub · your repos"))
                        .child(repo(look, false, "acme/web"))
                        .child(repo(look, false, "acme/infra"))
                        .child(repo(look, false, "acme/docs")),
                ),
        ))
        .child(row(
            look,
            "Machines",
            div()
                .flex()
                .flex_col()
                .gap(px(7.))
                .child(machine(look, true, "This Mac", "", true))
                .child(machine(look, true, "devbox-1", "online", true))
                .child(machine(look, false, "gpu-box", "offline", false)),
        ))
        .child(
            div()
                .flex()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Cancel"))
                .child(button(look, Kind::Primary, "Create channel and clone 2 repos")),
        );
    dialog(look, pages::home_list(look), body)
}

/// A thin progress bar filled to `percent`.
fn bar(look: Look, percent: f32, hex: u32) -> Div {
    div().w(px(160.)).h(px(4.)).rounded_full().bg(c(look.fill)).flex_shrink_0().child(div().h_full().w(px(160. * percent / 100.)).rounded_full().bg(c(hex)))
}

fn clone_row(look: Look, name: &'static str, percent: f32, state: &'static str, done: bool) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(8.))
        .child(small_icon(IconName::FolderGit2, look.secondary))
        .child(div().w(px(110.)).child(name))
        .child(bar(look, percent, if done { GREEN } else { look.accent }))
        .child(if done { small_icon(IconName::Check, GREEN) } else { small_icon(IconName::ArrowRight, look.tertiary) })
        .child(secondary(look, state))
}

/// A clone that failed: why, what to do, and **Retry** (owner pick
/// 2026-11-15 for frame 45).
fn failed_row(look: Look, name: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pl(px(8.))
                .child(small_icon(IconName::FolderGit2, look.secondary))
                .child(div().w(px(110.)).child(name))
                .child(bar(look, 12., RED))
                .child(small_icon(IconName::CircleX, RED))
                .child(div().text_size(px(11.5)).text_color(c(RED)).child("Clone failed"))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Retry")),
        )
        .child(
            div()
                .pl(px(30.))
                .text_size(px(11.5))
                .text_color(c(look.secondary))
                .child("devbox-1 lost its connection to github.com after 12%. Check its network, then Retry."),
        )
}

fn group(look: Look, name: &'static str, rows: Vec<Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(7.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(c(look.fill))
        .child(div().flex().items_center().gap(px(7.)).child(small_icon(IconName::Server, look.secondary)).child(div().font_weight(FontWeight::SEMIBOLD).child(name)))
        .child(div().flex().items_center().gap(px(8.)).pl(px(8.)).child(small_icon(IconName::Check, GREEN)).child(secondary(look, "Folders made")))
        .children(rows)
}

/// Frame 45: the same dialog while the clones run (CH-5), with Cancel and
/// one failed clone with Retry (owner pick 2026-11-15).
pub fn cloning(look: Look) -> Div {
    let body = div()
        .child(title("Creating # billing-exports"))
        .child(group(
            look,
            "This Mac",
            vec![clone_row(look, "acme/ledger", 100., "Done", true), clone_row(look, "acme/api", 64., "Receiving objects 64%", false)],
        ))
        .child(group(
            look,
            "devbox-1",
            vec![clone_row(look, "acme/ledger", 31., "Receiving objects 31%", false), failed_row(look, "acme/api")],
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .pt(px(4.))
                .child(div().text_size(px(11.)).text_color(c(look.tertiary)).child("The channel opens when every clone is done."))
                .child(div().flex_1())
                .child(button(look, Kind::Secondary, "Cancel")),
        );
    dialog(look, pages::home_list(look), body)
}
