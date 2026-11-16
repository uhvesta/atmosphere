//! Home after a restart: the sessions that could not be resumed link to the
//! restart report, mid-turn agents come back idle, and a toast says what
//! happened (PER-15, PER-17, PER-29, flow 26.8 step 5).

use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};

use crate::kit::*;
use crate::pages::{group, page, session_row, toolbar};
use crate::sidebar::Side;

/// A toast with a check mark and no action.
fn done_toast(look: Look, under: Div, text: &'static str) -> Div {
    under.relative().child(
        div().absolute().bottom(px(24.)).left_0().w_full().flex().justify_center().child(
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .px(px(14.))
                .py(px(9.))
                .rounded(px(10.))
                .bg(c(if look.dark { 0x3a3a3d } else { 0x2c2c2e }))
                .text_color(c(0xffffff))
                .text_size(px(12.5))
                .shadow_lg()
                .child(small_icon(IconName::Check, 0xffffff))
                .child(text),
        ),
    )
}

/// Frame 47: Home right after devbox-1 restarted.
pub fn home_after_restart(look: Look) -> Div {
    let report = div()
        .flex()
        .items_center()
        .gap(px(4.))
        .text_size(px(12.))
        .text_color(c(look.accent))
        .child(small_icon(IconName::FileText, look.accent))
        .child("Restart report");
    let home = page(
        look,
        Side { pinned: true, ..Side::at("Home") },
        toolbar(look, "Home", None, vec![button(look, Kind::Primary, "New session")]),
        div().flex().justify_center().p(px(24.)).text_size(px(13.)).child(
            div()
                .w(px(720.))
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(group(
                    look,
                    "Couldn't resume after the restart · 1",
                    vec![
                        session_row(
                            look,
                            St::Exited,
                            "Ledger schema migration",
                            "# payments-rewrite · devbox-1",
                            "Couldn't resume: its conversation file is missing",
                            "09:12",
                            Some(div().flex().items_center().gap(px(12.)).child(report).child(button(look, Kind::Secondary, "Open"))),
                        ),
                    ],
                ))
                .child(group(
                    look,
                    "Needs you · 2",
                    vec![
                        session_row(look, St::NeedsYou, "Try BRIN index", "fork of Ledger schema", "Asks to push try-brin to origin", "9m", Some(button(look, Kind::Primary, "Review"))),
                        session_row(look, St::Exited, "Upgrade Postgres 17", "# infra", "Exited with code 1 after the dump step", "40m", Some(button(look, Kind::Secondary, "Resume"))),
                    ],
                ))
                .child(group(
                    look,
                    "Working · 1",
                    vec![session_row(look, St::Working, "Benchmark BRIN", "# infra · gpu-box", "pgbench run 2 of 4", "1m", None)],
                ))
                .child(group(
                    look,
                    "Idle after the restart · 1",
                    vec![session_row(
                        look,
                        St::Idle,
                        "Refund retries",
                        "# payments-rewrite",
                        "Was mid-turn when devbox-1 restarted. Tell it to continue.",
                        "09:12",
                        Some(button(look, Kind::Secondary, "Open")),
                    )],
                )),
        ),
    );
    done_toast(look, home, "devbox-1 restarted: 3 sessions started again, 1 couldn't. Mid-turn agents are idle.")
}
