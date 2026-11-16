//! A machine's Storage tab (frame 60; LD-13 to LD-15, LD-17): what Humata
//! keeps on the machine, with the one action that frees each row.

use std::rc::Rc;

use crate::bridge::Ui;
use crate::file_manager::{action_label, kind_label};
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
};
use gpui_kit::base::ObservedElement;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::StorageRow;
use humata2_clients::model::MachineId;
use humata2_files::{FilesAction as A, FilesState, human_bytes};


const ORANGE: u32 = 0xff9f0a;

fn small_button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: &'static str,
    action: AppAction,
    look: Look,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .small()
        .h(px(24.))
        .px(px(11.))
        .label(label)
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

fn head(look: Look, text: &'static str) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.secondary))
        .child(text)
}

fn table_row(
    ui: &Rc<Ui>,
    state: &FilesState,
    index: usize,
    item: &StorageRow,
    show_kind: bool,
    look: Look,
) -> ObservedElement<Stateful<Div>> {
    let action: AnyElement = match action_label(&item.action) {
        Some(label) => Button::new(("storage-act", index))
            .small()
            .ghost()
            .text_color(rgb(look.accent))
            .label(label)
            .on_click({
                let ui = ui.clone();
                move |_, _, cx| ui.send(AppAction::Files(A::StorageAct(index)), cx)
            })
            .into_any_element(),
        None => div()
            .text_color(rgb(look.tertiary))
            .child("—")
            .into_any_element(),
    };
    row()
        .id(SharedString::from(format!("storage-row {index}")))
        .role(Role::ListItem)
        .aria_label(item.what.clone())
        .test_support()
        .gap(px(8.))
        .px(px(12.))
        .min_h(px(38.))
        .py(px(4.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .w(px(120.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_color(rgb(look.secondary))
                .child(if show_kind { kind_label(item.kind) } else { "" }),
        )
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .child(item.what.clone()),
                )
                .children(item.note.clone().map(|note| {
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(ORANGE))
                        .child(note)
                })),
        )
        .child(
            div()
                .w(px(70.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .child(human_bytes(item.bytes)),
        )
        .child(
            div()
                .w(px(60.))
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_color(rgb(look.secondary))
                .child(state.used(item)),
        )
        .child(div().w(px(80.)).flex_shrink_0().child(action))
}

/// The Storage tab's body for `machine`. `look` comes from the caller
/// because only it has the app context.
pub(crate) fn storage(
    ui: &Rc<Ui>,
    state: &FilesState,
    machine: &MachineId,
    look: Look,
) -> impl IntoElement {
    let mut body = col()
        .id("storage-tab")
        .p(px(20.))
        .gap(px(12.))
        .text_size(px(12.5))
        .text_color(rgb(look.text));
    let Some(storage) = state.storage.as_ref().filter(|s| &s.machine == machine) else {
        return body.child(div().text_color(rgb(look.secondary)).child("Measuring…"));
    };
    let Some(report) = storage.report.as_ref() else {
        return body.child(
            div()
                .id("storage-measuring")
                .text_color(rgb(look.secondary))
                .child(if storage.measuring {
                    "Measuring…"
                } else {
                    "Nothing measured yet."
                }),
        );
    };
    body = body.child(
        row()
            .gap(px(10.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(format!(
                        "{} free of {}",
                        human_bytes(report.free_bytes),
                        human_bytes(report.total_bytes)
                    )),
            )
            .child(div().text_size(px(12.)).text_color(rgb(look.secondary)).child(
                if storage.measuring {
                    "measuring…".to_owned()
                } else {
                    match state.measured_ago().as_deref() {
                        None | Some("now") => "measured just now".to_owned(),
                        Some(ago) => format!("measured {ago} ago"),
                    }
                },
            ))
            .child(div().flex_1())
            .child(small_button(
                ui,
                "storage-measure",
                "Measure again",
                AppAction::Files(A::MeasureAgain),
                look,
            )),
    );
    let mut table = col()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
        .overflow_hidden()
        .child(
            row()
                .gap(px(8.))
                .px(px(12.))
                .h(px(30.))
                .bg(rgb(look.panel))
                .border_b_1()
                .border_color(rgb(look.line))
                .child(head(look, "Kind").w(px(120.)).flex_shrink_0())
                .child(head(look, "What").flex_1())
                .child(head(look, "Size").w(px(70.)).flex_shrink_0())
                .child(head(look, "Used").w(px(60.)).flex_shrink_0())
                .child(head(look, "Action").w(px(80.)).flex_shrink_0()),
        );
    let mut previous = None;
    for (index, item) in report.rows.iter().enumerate() {
        table = table.child(table_row(ui, state, index, item, previous != Some(item.kind), look));
        previous = Some(item.kind);
    }
    body = body.child(table);

    if let Some(pending) = state
        .pending_clear
        .as_ref()
        .filter(|pending| &pending.machine == machine)
    {
        body = body.child(
            row()
                .id("storage-confirm-clear")
                .role(Role::Alert)
                .aria_label("Confirm clearing a build cache")
                .test_support()
                .gap(px(10.))
                .px(px(12.))
                .py(px(8.))
                .rounded(px(8.))
                .bg(rgb(look.attention_bg))
                .child(div().flex_1().min_w_0().child(format!(
                    "{} is running. Clearing its build cache makes its next build slower. Clear it anyway?",
                    pending.what
                )))
                .child(small_button(
                    ui,
                    "storage-clear-confirm",
                    "Clear",
                    AppAction::Files(A::ConfirmClear),
                    look,
                ))
                .child(small_button(
                    ui,
                    "storage-clear-cancel",
                    "Cancel",
                    AppAction::Files(A::CancelClear),
                    look,
                )),
        );
    }

    if !report.retention_skips.is_empty() {
        let mut skips = col()
            .id("storage-skips")
            .gap(px(4.))
            .child(head(look, "Left alone by the retention rules"));
        for skip in &report.retention_skips {
            skips = skips.child(
                row()
                    .gap(px(8.))
                    .child(div().font_weight(FontWeight::MEDIUM).child(skip.title.clone()))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(rgb(look.secondary))
                            .child(skip.reason.clone()),
                    ),
            );
        }
        body = body.child(skips);
    }
    body.child(
        div()
            .text_size(px(12.))
            .text_color(rgb(look.secondary))
            .child("Sizes are measured when this page opens. Archive… runs the same checks as archiving from a session."),
    )
}
