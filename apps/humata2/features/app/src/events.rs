use std::sync::Arc;

use humata2_clients::{DaemonError, Event};
use humata2_session::SessionAction;
use tca::{Effect, StreamEnd};

use crate::{App, AppAction, AppState, upsert};

impl App {
    pub(super) fn apply(&self, state: &mut AppState, event: Event) -> Effect<AppAction> {
        let workspace = Arc::make_mut(&mut state.workspace);
        let mut snapshot = None;
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
                upsert(&mut workspace.requests, request, |a, b| a.id == b.id);
            }
            Event::Machine(machine) => {
                upsert(&mut workspace.machines, machine, |a, b| a.id == b.id);
            }
            Event::Transfer(transfer) => {
                upsert(&mut workspace.transfers, transfer, |a, b| a.id == b.id);
            }
            Event::Resync => return self.load(),
        }
        let mut effects = vec![self.show(state)];
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
