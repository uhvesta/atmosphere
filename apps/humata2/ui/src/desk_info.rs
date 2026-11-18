//! A Desk session's Agent access block in the Info panel (DK-4, frame 40):
//! the two switches the New desk session dialog set, changeable here.
use crate::bridge::Ui;
use crate::chrome;
use crate::theme::{GREEN, Look, ORANGE};
use crate::widgets::{col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::desk::{Access, Switch};
use humata2_clients::model::{ChannelId, Session};
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

/// One management switch as frames 92 and 93 draw it (CD-3): its words and
/// a pill switch. Clicking sends `action` unless `enabled` is false. A
/// switch its channel rule blocks draws its pill dimmed, on or off; the
/// orange line under it says why.
#[allow(clippy::too_many_arguments)]
pub(crate) fn switch_row(
    ui: &Rc<Ui>,
    look: Look,
    id: SharedString,
    words: &'static str,
    on: bool,
    enabled: bool,
    blocked: bool,
    action: AppAction,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(words)
        .aria_selected(on)
        .test_support()
        .gap(px(10.))
        .opacity(if enabled { 1. } else { 0.55 })
        .child(div().flex_1().min_w_0().child(words))
        .child(
            div()
                .w(px(28.))
                .h(px(16.))
                .rounded_full()
                .p(px(2.))
                .flex()
                .flex_shrink_0()
                .when(blocked, |x| x.opacity(0.5))
                .when(on, |x| x.bg(rgb(GREEN)).justify_end())
                .when(!on, |x| {
                    x.bg(rgb(look.fill)).border_1().border_color(rgb(look.line))
                })
                .child(
                    div()
                        .size(px(12.))
                        .rounded_full()
                        .bg(rgb(0xffffff))
                        .shadow_sm(),
                ),
        )
        .when(enabled, |x| {
            x.cursor_pointer()
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
        })
}

/// "Blocked by the channel rule: Never", with the link to that rule (CD-3):
/// the channel's Agent access tab. It closes a dialog first, so the New
/// desk session dialog doesn't stay over the page the link opens. The New
/// desk session dialog and the Info panel both use it, so both go to the
/// same place.
pub(crate) fn blocked_line(
    ui: &Rc<Ui>,
    look: Look,
    id: SharedString,
    channel: &ChannelId,
) -> Div {
    let ui = ui.clone();
    let channel = channel.clone();
    row()
        .gap(px(8.))
        .text_size(px(11.5))
        .child(
            div()
                .text_color(rgb(ORANGE))
                .child("Blocked by the channel rule: Never"),
        )
        .child(
            div()
                .id(id)
                .role(Role::Link)
                .aria_label("Agent access")
                .text_color(rgb(look.accent))
                .cursor_pointer()
                .child("Agent access")
                .on_click(move |_, _, cx| {
                    ui.send(AppAction::Desk(DeskAction::CancelDialog), cx);
                    ui.send(AppAction::OpenChannelAccess(channel.clone()), cx);
                })
                .test_support(),
        )
}

/// The element id of a channel Desk session's switch in the Info panel.
const fn switch_id(switch: Switch) -> &'static str {
    match switch {
        Switch::ReadFiles => "channel-desk-info-read-files",
        Switch::SendNotes => "channel-desk-info-send-notes",
        Switch::TypeIntoTerminal => "channel-desk-info-type-into-terminal",
        Switch::StartAndFork => "channel-desk-info-start-and-fork",
        Switch::StopAndArchive => "channel-desk-info-stop-and-archive",
        Switch::ChangeBoardItems => "channel-desk-info-change-board-items",
    }
}

/// A channel Desk session's six switches (CD-3, frame 93), or `None` when
/// `session` isn't one. A switch whose channel rule is Never says so, on or
/// off; only the user changes them (IN-23).
pub(crate) fn channel_switches(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Option<AnyElement> {
    let record = state.desk.channel_record(&session.id)?;
    let enabled = !state.desk.pending.contains(&session.id);
    let mut block = col()
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .p(px(10.))
        .gap(px(9.))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("What this session may do"),
        );
    for switch_kind in Switch::ALL {
        let on = record.switches.get(switch_kind);
        let id = switch_id(switch_kind);
        let blocked = record.blocked.contains(&switch_kind);
        let mut item = col().gap(px(2.)).child(switch_row(
            ui,
            look,
            id.into(),
            switch_kind.label(),
            on,
            enabled,
            blocked,
            AppAction::Desk(DeskAction::SetSwitch(
                session.id.clone(),
                switch_kind,
                !on,
            )),
        ));
        if blocked {
            item = item.child(blocked_line(
                ui,
                look,
                format!("{id}-rule").into(),
                &record.channel,
            ));
        }
        block = block.child(item);
    }
    Some(
        block
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child("Only you change these. Everything else follows the channel's rules."),
            )
            .into_any_element(),
    )
}

/// The channel Desk sessions looking after `session` (CD-2), as a card with a
/// row each; a click opens that session. `None` when nothing looks after it.
pub(crate) fn targeting(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Option<AnyElement> {
    let rows = state.desk.targeting_rows(&session.id);
    if rows.is_empty() {
        return None;
    }
    let mut block = col()
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .p(px(10.))
        .gap(px(6.))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Desk sessions looking after this"),
        );
    for item in rows {
        let ui = ui.clone();
        let open = AppAction::Desk(DeskAction::Open(item.session.id.clone()));
        block = block.child(
            row()
                .id(SharedString::from(format!(
                    "desk-info-targeting-{}",
                    item.session.id.as_str()
                )))
                .role(Role::Button)
                .aria_label(item.session.title.clone())
                .gap(px(7.))
                .cursor_pointer()
                .child(chrome::status(item.session.status))
                .child(
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .child(item.session.title.clone()),
                )
                .on_click(move |_, _, cx| ui.send(open.clone(), cx))
                .test_support(),
        );
    }
    Some(block.into_any_element())
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
