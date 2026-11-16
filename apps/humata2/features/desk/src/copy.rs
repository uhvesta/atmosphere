//! Copy to a channel… (DK-13).

use humata2_clients::DaemonError;
use humata2_clients::desk::Document;
use humata2_clients::model::{ChannelId, MachineId};
use tca::Effect;

use crate::{Delegate, Desk, DeskAction, DeskState};

/// The Copy to a channel… dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CopyDialog {
    /// The document being copied.
    pub document: Document,
    /// The channel picked.
    pub channel: Option<ChannelId>,
    /// The machine picked; set for the channel's only machine.
    pub machine: Option<MachineId>,
    /// The copy is in flight.
    pub copying: bool,
    /// Why it wasn't copied.
    pub error: Option<String>,
}

impl Desk {
    pub(super) fn start_copy(state: &mut DeskState, document: Document) {
        if state.sheet_open() || !state.workspace.desk.documents.contains(&document) {
            return;
        }
        state.menu = None;
        state.copy = Some(CopyDialog {
            document,
            channel: None,
            machine: None,
            copying: false,
            error: None,
        });
    }

    pub(super) fn copy_channel(state: &mut DeskState, channel: ChannelId) {
        let Some(found) = state
            .workspace
            .channels
            .iter()
            .find(|candidate| candidate.id == channel)
        else {
            return;
        };
        let machine = match found.machines.as_slice() {
            [only] => Some(only.clone()),
            _ => None,
        };
        let Some(dialog) = state.copy.as_mut() else {
            return;
        };
        if dialog.copying {
            return;
        }
        dialog.channel = Some(channel);
        dialog.machine = machine;
        dialog.error = None;
    }

    pub(super) fn copy_machine(state: &mut DeskState, machine: MachineId) {
        let allowed = state
            .copy_machines()
            .iter()
            .any(|candidate| candidate.id == machine);
        let Some(dialog) = state.copy.as_mut() else {
            return;
        };
        if allowed && !dialog.copying {
            dialog.machine = Some(machine);
            dialog.error = None;
        }
    }

    pub(super) fn confirm_copy(&self, state: &mut DeskState) -> Effect<DeskAction> {
        let Some(dialog) = state.copy.as_mut() else {
            return Effect::none();
        };
        if dialog.copying {
            return Effect::none();
        }
        let Some(channel) = dialog.channel.clone() else {
            dialog.error = Some("Pick the channel to copy this file to.".to_owned());
            return Effect::none();
        };
        let Some(machine) = dialog.machine.clone() else {
            dialog.error = Some(
                "This channel has several machines. Pick the machine whose shared folder gets the file."
                    .to_owned(),
            );
            return Effect::none();
        };
        dialog.copying = true;
        dialog.error = None;
        let document = dialog.document.clone();
        let client = self.client.clone();
        Effect::future(async move {
            let result = client
                .copy_document(&document, &channel, Some(&machine))
                .await;
            DeskAction::Copied(result)
        })
    }

    pub(super) fn copied(state: &mut DeskState, result: Result<(), DaemonError>) -> Effect<DeskAction> {
        let Some(dialog) = state.copy.as_mut() else {
            return Effect::none();
        };
        match result {
            Ok(()) => {
                let file = dialog
                    .document
                    .path
                    .rsplit('/')
                    .next()
                    .unwrap_or(&dialog.document.path)
                    .to_owned();
                let channel = dialog.channel.clone();
                state.copy = None;
                let name = channel
                    .as_ref()
                    .and_then(|id| state.workspace.channels.iter().find(|c| &c.id == id))
                    .map_or_else(|| "the channel".to_owned(), |c| format!("#{}", c.name));
                Effect::send(DeskAction::Delegate(Delegate::Toast(format!(
                    "Copied {file} to {name}'s shared folder."
                ))))
            }
            Err(error) => {
                dialog.copying = false;
                dialog.error = Some(error.to_string());
                Effect::none()
            }
        }
    }
}
