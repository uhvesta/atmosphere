//! A Desk session's Agent access block in the Info panel (DK-4, frame 40):
//! the two switches the New desk session dialog set, changeable here.
use crate::bridge::Ui;
use crate::chrome;
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::desk::Access;
use humata2_clients::model::Session;
use humata2_desk::DeskAction;
use std::rc::Rc;

/// One switch: a check box with its words. Clicking sends `action` unless
/// `enabled` is false.
fn switch(
    ui: &Rc<Ui>,
    look: Look,
    id: &'static str,
    words: &'static str,
    on: bool,
    enabled: bool,
    action: AppAction,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(words)
        .aria_selected(on)
        .test_support()
        .gap(px(7.))
        .opacity(if enabled { 1. } else { 0.55 })
        .child(
            row()
                .size(px(14.))
                .flex_shrink_0()
                .rounded(px(4.))
                .justify_center()
                .when(on, |x| {
                    x.bg(rgb(look.accent))
                        .child(chrome::icon(IconName::Check, 0xffffff, 10.))
                })
                .when(!on, |x| {
                    x.border_1()
                        .border_color(rgb(look.tertiary))
                        .bg(rgb(look.card))
                }),
        )
        .child(div().min_w_0().child(words))
        .when(enabled, |x| {
            x.cursor_pointer()
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
        })
}

/// The Agent access block for a Desk session, or `None` when `session` has
/// no Desk record. The switches are off the table while a change to the
/// session is in flight.
pub(crate) fn access(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Option<AnyElement> {
    let record = state.desk.record(&session.id)?;
    let current = record.access;
    let enabled = !state.desk.pending.contains(&session.id);
    let set = |access: Access| AppAction::Desk(DeskAction::SetAccess(session.id.clone(), access));
    Some(
        col()
            .rounded(px(8.))
            .bg(rgb(look.card))
            .border_1()
            .border_color(rgb(look.line))
            .p(px(10.))
            .gap(px(6.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Agent access"),
            )
            .child(switch(
                ui,
                look,
                "desk-info-access-boards",
                "Read and change every channel's board",
                current.boards,
                enabled,
                set(Access {
                    boards: !current.boards,
                    ..current
                }),
            ))
            .child(switch(
                ui,
                look,
                "desk-info-access-channels",
                "Make channels and add machines",
                current.channels_and_machines,
                enabled,
                set(Access {
                    channels_and_machines: !current.channels_and_machines,
                    ..current
                }),
            ))
            .into_any_element(),
    )
}
