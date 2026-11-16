//! Copy to a channel… (DK-13): pick the channel, and its machine when it has
//! several, to copy a Desk document into the channel's shared folder.

use super::{Overlays, button, form_row, header, primary, secondary};
use crate::bridge::Ui;
use crate::chrome;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::Workspace;
use humata2_clients::model::MachineId;
use humata2_desk::{CopyDialog, DeskAction};
use std::rc::Rc;

/// One pick in a list: its words, marked when it is the one picked.
fn pick(
    ui: &Rc<Ui>,
    look: Look,
    id: SharedString,
    text: String,
    on: bool,
    enabled: bool,
    action: AppAction,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(text.clone())
        .aria_selected(on)
        .test_support()
        .gap(px(8.))
        .h(px(26.))
        .px(px(8.))
        .rounded(px(6.))
        .when(on, |x| x.bg(rgb(look.fill)))
        .child(div().size(px(14.)).flex_shrink_0().when(on, |x| {
            x.child(chrome::icon(IconName::Check, look.accent, 13.))
        }))
        .child(div().min_w_0().child(text))
        .when(enabled, |x| {
            x.cursor_pointer()
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
        })
}

impl Overlays {
    /// The Copy to a channel… dialog; `machines` are the picked channel's.
    pub(super) fn desk_copy(
        &self,
        dialog: &CopyDialog,
        workspace: &Workspace,
        machines: &[(MachineId, String)],
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let busy = dialog.copying;
        let file = dialog
            .document
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&dialog.document.path)
            .to_owned();
        let channels = col().gap(px(2.)).children(workspace.channels.iter().map(|c| {
            pick(
                ui,
                look,
                SharedString::from(format!("desk-copy-channel-{}", c.id.as_str())),
                format!("# {}", c.name),
                dialog.channel.as_ref() == Some(&c.id),
                !busy,
                AppAction::Desk(DeskAction::CopyChannel(c.id.clone())),
            )
        }));
        let mut content = super::card("desk-copy-dialog", "Copy to a channel", 560., look)
            .track_focus(&self.sheet)
            .gap(px(12.))
            .child(header(look, "Copy to a channel", file))
            .child(form_row(look, "Channel", channels));
        if machines.len() > 1 {
            content = content.child(form_row(
                look,
                "Machine",
                col().gap(px(2.)).children(machines.iter().map(|(id, name)| {
                    pick(
                        ui,
                        look,
                        SharedString::from(format!("desk-copy-machine-{}", id.as_str())),
                        name.clone(),
                        dialog.machine.as_ref() == Some(id),
                        !busy,
                        AppAction::Desk(DeskAction::CopyMachine(id.clone())),
                    )
                })),
            ));
        }
        content
            .child(secondary(
                look,
                "Copied into the channel's shared folder. The Desk keeps its copy.",
            ))
            .children(dialog.error.clone().map(|error| {
                widgets::error_banner(ui, "desk-copy-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(div().flex_1())
                    .child(button(ui, look, "desk-copy-cancel", "Cancel", AppAction::Escape))
                    .child(primary(
                        ui,
                        look,
                        "desk-copy-confirm",
                        if busy { "Copying…" } else { "Copy" },
                        AppAction::Desk(DeskAction::ConfirmCopy),
                        busy,
                    )),
            )
            .focus_trap("desk-copy-trap", &self.sheet)
            .into_any_element()
    }
}
