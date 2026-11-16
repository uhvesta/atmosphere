//! The transfers tray under the Files panes (frame 61; FM-7, FM-8, FM-13).

use std::rc::Rc;

use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::component::{Sizable as _, button::Button};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::TransferPhase;
use humata2_files::{FilesAction as A, FilesState, human_bytes};

const ORANGE: u32 = 0xff9f0a;
const GREEN: u32 = 0x30d158;
const RED: u32 = 0xff453a;
const BAR_W: f32 = 140.;

fn phase_color(phase: &TransferPhase, look: Look) -> u32 {
    match phase {
        TransferPhase::Copying | TransferPhase::Verifying => look.accent,
        TransferPhase::Paused { .. } => ORANGE,
        TransferPhase::Done => GREEN,
        TransferPhase::Failed { .. } => RED,
        TransferPhase::Cancelled => look.secondary,
    }
}

/// The tray: a header and one row per transfer. `look` comes from the
/// caller because only it has the app context.
pub(crate) fn tray(ui: &Rc<Ui>, state: &FilesState, look: Look) -> impl IntoElement {
    let mut tray = col()
        .id("files-transfers")
        .flex_shrink_0()
        .max_h(px(260.))
        .bg(rgb(look.panel))
        .child(
            row()
                .h(px(30.))
                .px(px(16.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(rgb(look.line))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(look.secondary))
                        .child("Transfers"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .whitespace_nowrap()
                        .text_size(px(12.))
                        .text_color(rgb(look.secondary))
                        .child("over each machine's SSH link"),
                ),
        );
    if state.transfers.is_empty() {
        tray = tray.child(
            div()
                .px(px(16.))
                .py(px(8.))
                .text_size(px(12.))
                .text_color(rgb(look.secondary))
                .child("No transfers yet. Copy or move something between the panes."),
        );
    }
    let mut list = col();
    for (index, transfer) in state.transfers.iter().enumerate() {
        let color = phase_color(&transfer.phase, look);
        let route = format!(
            "{} \u{2192} {}",
            transfer.what,
            state.machine_name(&transfer.to)
        );
        let amount = format!(
            "{} / {}",
            human_bytes(transfer.done_bytes),
            human_bytes(transfer.total_bytes)
        );
        let action = if transfer.phase.is_over() {
            None
        } else {
            let ui = ui.clone();
            let id = transfer.id.clone();
            Some(
                Button::new(("files-transfer-cancel", index))
                    .small()
                    .h(px(24.))
                    .px(px(11.))
                    .label("Cancel")
                    .bg(rgb(look.fill))
                    .text_color(rgb(look.text))
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::Files(A::CancelTransfer(id.clone())), cx);
                    }),
            )
        };
        list = list.child(
            row()
                .id(SharedString::from(format!("files-transfer {}", transfer.id)))
                .role(Role::ListItem)
                .aria_label(route.clone())
                .test_support()
                .gap(px(12.))
                .px(px(16.))
                .py(px(8.))
                .flex_shrink_0()
                .border_b_1()
                .border_color(rgb(look.line))
                .child(
                    col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(2.))
                        .child(
                            row()
                                .gap(px(12.))
                                .text_size(px(12.5))
                                .child(
                                    div()
                                        .w(px(220.))
                                        .flex_shrink_0()
                                        .whitespace_nowrap()
                                        .overflow_hidden()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(route),
                                )
                                .child(
                                    div()
                                        .w(px(100.))
                                        .flex_shrink_0()
                                        .whitespace_nowrap()
                                        .text_color(rgb(look.secondary))
                                        .child(amount),
                                )
                                .child(
                                    div()
                                        .w(px(BAR_W))
                                        .h(px(5.))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(rgb(look.fill))
                                        .child(
                                            div()
                                                .w(px(BAR_W * transfer.fraction() as f32))
                                                .h_full()
                                                .rounded_full()
                                                .bg(rgb(color)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .whitespace_nowrap()
                                        .text_color(rgb(color))
                                        .child(state.status_line(transfer)),
                                ),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child(state.detail_line(transfer)),
                        ),
                )
                .child(
                    div()
                        .w(px(70.))
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .children(action),
                ),
        );
    }
    tray.child(
        div()
            .id("files-transfers-list")
            .min_h_0()
            .overflow_y_scroll()
            .child(list),
    )
}
