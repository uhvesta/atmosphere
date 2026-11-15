//! The Desk's Clean up… dialog (frame 55; SPEC DK-11, MV-9, NV-27, LD-7):
//! closed sessions, least recently used first, with folder sizes.

use gpui_kit::{prelude::*, *};

use crate::desk;
use crate::kit::*;
use crate::overlays::scrim;

fn session(look: Look, on: bool, name: &'static str, ran: &'static str, closed: &'static str, size: &'static str, note: Option<(&'static str, u32)>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .py(px(8.))
        .border_b_1()
        .border_color(c(look.line))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(checkbox(look, on, name).flex_1().min_w_0().whitespace_nowrap())
                .child(secondary(look, ran).w(px(160.)).flex_shrink_0().whitespace_nowrap())
                .child(secondary(look, closed).w(px(96.)).flex_shrink_0().whitespace_nowrap())
                .child(div().w(px(150.)).flex_shrink_0().flex().justify_end().whitespace_nowrap().text_color(c(look.text)).child(size)),
        )
        .when_some(note, |row, (text, hex)| row.child(div().pl(px(21.)).text_size(px(11.5)).text_color(c(hex)).child(text)))
}

/// Frame 55: pick closed sessions to delete.
pub fn clean_up(look: Look) -> Div {
    desk::desk(look).relative().child(scrim(look)).child(
        div().absolute().top(px(70.)).left_0().w_full().flex().justify_center().child(
            div()
                .w(px(700.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .p(px(20.))
                .rounded(px(12.))
                .bg(c(look.card))
                .border_1()
                .border_color(c(look.line))
                .shadow_lg()
                .text_size(px(12.5))
                .child(div().flex().items_center().gap(px(8.)).child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Clean up closed sessions")).child(secondary(look, "Least recently used first")))
                .child(session(look, true, "Draft the onboarding doc", "Claude Code · This Mac", "closed Nov 3", "scratch folder 48 MB", None))
                .child(session(
                    look,
                    false,
                    "Port the cron jobs",
                    "Codex · devbox-1",
                    "closed Nov 6",
                    "2.3 GB",
                    Some(("acme/api: 2 commits on port-cron never pushed. Deleting deletes that work.", ORANGE)),
                ))
                .child(session(look, false, "Notes cleanup", "Terminal · This Mac", "closed Nov 9", "folder ~/notes", Some(("Your folder: left in place", look.secondary))))
                .child(session(look, true, "Sketch the release checklist", "Codex · This Mac", "closed Nov 14", "scratch folder 120 MB", None))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .pt(px(10.))
                        .child(div().text_size(px(11.)).text_color(c(look.tertiary)).child("Deleted sessions go to the trash for 30 days"))
                        .child(div().flex_1())
                        .child(button(look, Kind::Secondary, "Cancel"))
                        .child(button(look, Kind::Primary, "Delete 2 sessions · frees 168 MB")),
                ),
        ),
    )
}
