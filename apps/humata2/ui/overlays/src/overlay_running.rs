//! The machine page's running list asks over the whole window (D363): End
//! for good… names the agent and says it can't be undone, and Adopt… gives a
//! terminal no channel has a channel or the Desk.

use super::Overlays;
use super::fork::{card, heading};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome::agent_label;
use humata2_ui_core::theme::{Look, RED_STATUS};
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::RunningHolder;
use humata2_clients::model::MachineKind;
use humata2_hosts::page::MachinePageAction;
use humata2_hosts::{AdoptTarget, RunningAction};
use std::rc::Rc;

/// What the open question asks, read from the app's state.
#[derive(Clone)]
pub(super) enum RunningView {
    End { holder: RunningHolder, machine: String },
    Adopt { holder: RunningHolder, machine: String, channels: Vec<(humata2_clients::model::ChannelId, String)> },
}

impl RunningView {
    /// The open question, if there is one.
    pub(super) fn read(state: &AppState) -> Option<Self> {
        let running = &state.machine_page.running;
        let name = |holder: &RunningHolder| {
            state.workspace.machine(&holder.machine).map_or_else(
                || holder.machine.to_string(),
                |found| {
                    if matches!(found.kind, MachineKind::ThisMac) {
                        "This Mac".to_owned()
                    } else {
                        found.name.clone()
                    }
                },
            )
        };
        if let Some(holder) = running.confirm_end.clone() {
            let machine = name(&holder);
            return Some(Self::End { holder, machine });
        }
        let holder = running.adopting.clone()?;
        let machine = name(&holder);
        let channels = state
            .workspace
            .channels
            .iter()
            .map(|channel| (channel.id.clone(), format!("# {}", channel.name)))
            .collect();
        Some(Self::Adopt { holder, machine, channels })
    }

    pub(super) const fn is_end(&self) -> bool {
        matches!(self, Self::End { .. })
    }
}

fn send(action: RunningAction) -> AppAction {
    AppAction::MachinePage(MachinePageAction::Running(action))
}

/// What End for good… asks (D784, Fable A7): a shell is "this shell", an
/// agent goes by its title or name.
pub(crate) fn end_question(holder: &RunningHolder) -> String {
    if holder.agent == humata2_clients::model::Agent::Shell {
        "End this shell for good?".to_owned()
    } else if holder.title.is_empty() {
        format!("End {} for good?", agent_label(holder.agent))
    } else {
        format!("End {} for good?", holder.title)
    }
}

/// What End for good… says: which process ends (folder, machine, pid), and
/// that it can't be undone (D784, Fable A7).
pub(crate) fn end_words(holder: &RunningHolder, machine: &str) -> String {
    let agent = agent_label(holder.agent);
    let mut place = if holder.folder.is_empty() {
        format!("on {machine}")
    } else {
        format!("in {} on {machine}", holder.folder)
    };
    if let Some(pid) = holder.pid {
        place.push_str(&format!(", pid {pid}"));
    }
    if holder.is_orphan() || holder.agent == humata2_clients::model::Agent::Shell {
        format!("{agent} {place} ends now, with everything it started. It can't be resumed or undone.")
    } else {
        format!("{agent} {place} ends and the session is archived. It can't be resumed or undone.")
    }
}

impl Overlays {
    /// The End for good… or Adopt… question.
    pub(super) fn running_dialog(&self, view: &RunningView, look: Look, ui: &Rc<Ui>) -> AnyElement {
        let dismiss = ui.clone();
        let (cancel, body) = match view {
            RunningView::End { holder, machine } => {
                (
                    RunningAction::CancelEnd,
                    card("running-end-dialog", "End for good", 440., look)
                        .child(heading(end_question(holder)))
                        .child(div().text_color(rgb(look.secondary)).child(end_words(holder, machine)))
                        .child(
                            row()
                                .gap(px(8.))
                                .pt(px(4.))
                                .justify_end()
                                .child(widgets::push_quiet(ui, "running-end-cancel", "Cancel", send(RunningAction::CancelEnd), look))
                                .child(
                                    widgets::push_quiet(ui, "running-end-confirm", "End for good", send(RunningAction::ConfirmEnd), look)
                                        .border_color(rgb(RED_STATUS))
                                        .text_color(rgb(RED_STATUS)),
                                ),
                        ),
                )
            }
            RunningView::Adopt { holder, machine, channels } => {
                let place = |id: String, text: String, target: AdoptTarget| {
                    widgets::push_quiet(ui, SharedString::from(id), text, send(RunningAction::ConfirmAdopt(target)), look)
                        .w_full()
                };
                let mut list = col().gap(px(6.)).child(place("running-adopt-desk".to_owned(), "Desk".to_owned(), AdoptTarget::Desk));
                for (id, name) in channels {
                    list = list.child(place(format!("running-adopt-{id}"), name.clone(), AdoptTarget::Channel(id.clone())));
                }
                (
                    RunningAction::CancelAdopt,
                    card("running-adopt-dialog", "Adopt", 440., look)
                        .child(heading(format!("Adopt {} on {machine}", agent_label(holder.agent))))
                        .child(
                            div()
                                .text_color(rgb(look.secondary))
                                .child("Give it to a channel or the Desk. It keeps running."),
                        )
                        .child(list)
                        .child(
                            row()
                                .pt(px(4.))
                                .justify_end()
                                .child(widgets::push_quiet(ui, "running-adopt-cancel", "Cancel", send(RunningAction::CancelAdopt), look)),
                        ),
                )
            }
        };
        let body = body
            .track_focus(&self.sheet)
            .on_action(move |_: &humata2_ui_core::keys::Dismiss, _, cx| {
                cx.stop_propagation();
                dismiss.send(send(cancel.clone()), cx);
            })
            .test_support()
            .focus_trap("running-trap", &self.sheet);
        div().id("running-dialog").role(Role::Group).aria_label("running dialog").test_support().child(body).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::core::prelude::v1::test;
    use humata2_clients::model::{Agent, MachineId};

    fn holder(agent: Agent) -> RunningHolder {
        RunningHolder {
            tab: None,
            session: None,
            title: String::new(),
            channel: None,
            desk: false,
            agent,
            pid: Some(48_213),
            folder: "~/src/ledger".to_owned(),
            started_unix_ms: None,
            attached: false,
            machine: MachineId::new("devbox-1"),
            stopped: false,
            agent_tab: false,
            archived: false,
            paused_for_move: false,
        }
    }

    #[test]
    fn ending_names_the_folder_machine_and_pid() {
        let words = end_words(&holder(Agent::Shell), "devbox-1");
        assert!(words.contains("~/src/ledger"));
        assert!(words.contains("devbox-1"));
        assert!(words.contains("48213"));
    }

    #[test]
    fn a_shell_is_asked_about_as_a_shell_not_by_an_empty_title() {
        let shell = end_question(&holder(Agent::Shell));
        let claude = end_question(&holder(Agent::Claude));
        assert_ne!(shell, claude);
    }
}
