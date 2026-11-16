use std::sync::Arc;

use humata2_clients::{DaemonError, Event};
use humata2_session::SessionAction;
use tca::{Effect, StreamEnd};

use crate::{App, AppAction, AppState, upsert};

impl App {
    pub(super) fn apply(&self, state: &mut AppState, event: Event) -> Effect<AppAction> {
        let workspace = Arc::make_mut(&mut state.workspace);
        let mut snapshot = None;
        let mut permission_alert = None;
        match event {
            Event::Output { session, tab, line } => {
                return self.child(
                    state,
                    AppAction::Session(SessionAction::Output { session, tab, line }),
                );
            }
            Event::TerminalData {
                session,
                tab,
                offset,
                bytes,
            } => {
                return self.child(
                    state,
                    AppAction::Session(SessionAction::TerminalData {
                        session,
                        tab,
                        offset,
                        bytes,
                    }),
                );
            }
            Event::Session(session) => {
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
            Event::Restart(restart) => return self.heard_restart(state, restart),
            Event::ChannelSetup(setup) => return self.heard_setup(state, setup),
            Event::Resync => return self.load(),
        }
        let mut effects = vec![self.show(state)];
        if let Some(text) = permission_alert {
            effects.push(state.toast(text, &self.clients.clock));
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
