use std::sync::Arc;

use humata2_clients::{DaemonError, Event};
use humata2_session::SessionAction;
use tca::{Effect, StreamEnd};

use crate::{App, AppAction, AppState, upsert};

impl App {
    pub(super) fn apply(&self, state: &mut AppState, event: Event) -> Effect<AppAction> {
        let workspace = Arc::make_mut(&mut state.workspace);
        let mut snapshot = None;
        let mut left = false;
        let mut permission_alert = None;
        match event {
            Event::Output { session, tab, line } => {
                return self.child(
                    state,
                    AppAction::Session(SessionAction::Output { session, tab, line }),
                );
            }
            Event::Session(session) => {
                // Archived from another window: leave its page here.
                left = session.archived
                    && state.route == crate::Route::Session(session.id.clone());
                snapshot = Some(session.id.clone());
                upsert(&mut workspace.sessions, session, |a, b| a.id == b.id);
            }
            Event::Request(request) => {
                if state.settings.preferences.permission_request_alerts
                    && request.answer.is_none()
                    && !matches!(request.kind, humata2_clients::model::RequestKind::Question)
                    && !workspace
                        .requests
                        .iter()
                        .any(|saved| saved.id == request.id)
                {
                    permission_alert = Some(format!("Permission requested: {}", request.title));
                }
                upsert(&mut workspace.requests, request, |a, b| a.id == b.id);
            }
            Event::RequestGone(id) => {
                workspace.requests.retain(|request| request.id != id);
            }
            Event::Rules { channel, rules } => {
                workspace.rules.insert(channel, rules);
            }
            Event::Machine(machine) => {
                upsert(&mut workspace.machines, machine, |a, b| a.id == b.id);
            }
            Event::Transfer(transfer) => {
                upsert(&mut workspace.transfers, transfer, |a, b| a.id == b.id);
            }
            Event::Item { item, details } => {
                workspace.item_details.insert(item.id.clone(), details);
                upsert(&mut workspace.items, item, |a, b| a.id == b.id);
            }
            Event::ItemGone(id) => {
                workspace.items.retain(|item| item.id != id);
                workspace.item_details.remove(&id);
            }
            Event::Files { session, change } => return self.files_changed(state, session, change),
            // GAP (D438): an open search isn't run again when the index
            // catches up; the next keystroke searches the new entries.
            Event::HistoryIndexed(_) => return Effect::none(),
            // DESKWATCH: the Desk follows what the daemon tells, with no reload.
            Event::Desk(change) => workspace.desk.apply(change),
            // MACHINESCRIPTS (D686): the open machine page reads its runs again, and so does
            // Settings > Machines while it is the page shown. Nothing else reads.
            Event::MachineScriptRun(machine) => {
                let mut effects = Vec::new();
                if state.machine_page.machine.as_ref() == Some(&machine) {
                    effects.push(self.machine_page(
                        state,
                        humata2_hosts::MachinePageAction::ScriptRunChanged(machine),
                    ));
                }
                if state.route == crate::Route::Settings
                    && state.settings.section == humata2_settings::Section::Machines
                {
                    effects.push(self.child(
                        state,
                        AppAction::Settings(humata2_settings::SettingsAction::Section(
                            humata2_settings::Section::Machines,
                        )),
                    ));
                }
                return Effect::merge(effects);
            }
            Event::Restart(restart) => return self.heard_restart(state, restart),
            Event::ChannelSetup(setup) => return self.heard_setup(state, setup),
            Event::Operation(update) => {
                self.new_session_operation(state, update.clone());
                self.add_repo_operation(state, update.clone());
                return self.child(
                    state,
                    AppAction::Session(SessionAction::OperationMoved(update)),
                );
            }
            Event::Resync => return self.load(),
        }
        let mut effects = vec![self.show(state)];
        if left {
            effects.push(self.handle(
                state,
                AppAction::Session(SessionAction::Delegate(humata2_session::Delegate::Left)),
            ));
        }
        if let Some(text) = permission_alert {
            effects.push(state.toast(text, &self.clients.clock));
        }
        // A session event can be the one that marks the open page a review
        // after it was arrived at: follow it now.
        if let Some(id) = snapshot.as_ref().filter(|id| state.route == crate::Route::Session((*id).clone())) {
            effects.push(self.follow_review(state, id));
        }
        if let Some(id) = snapshot {
            effects.push(self.child(state, AppAction::Session(SessionAction::Snapshot(id))));
        }
        Effect::merge(effects)
    }

    pub(super) fn events_ended(
        &self,
        state: &mut AppState,
        end: StreamEnd<DaemonError>,
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        let text = match end {
            StreamEnd::Finished => "The daemon stopped sending updates. Reconnecting.".to_owned(),
            StreamEnd::Failed(error) => {
                format!("Lost updates from the daemon: {error}. Reconnecting.")
            }
        };
        Effect::merge([state.toast(text, &clock), self.load(), self.follow()])
    }
}
