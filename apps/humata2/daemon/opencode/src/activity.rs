//! What OpenCode's events mean for the agent's turn (D201).

use std::collections::{BTreeMap, BTreeSet};

use crate::event::Event;

/// What the agent's turn is doing, as the server's events say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Turn {
    /// A session on the server is running.
    Working,
    /// Nothing is running and nothing is asked.
    Idle,
    /// An ask is open, so the user has to answer it.
    NeedsInput,
}

/// The sessions running and the asks open on one server.
#[derive(Default)]
pub struct Activity {
    /// Sessions with a run in progress.
    running: BTreeSet<String>,
    /// Open asks by id, each with the session that asked.
    asks: BTreeMap<String, String>,
    /// The turn last reported, so that only changes are reported.
    reported: Option<Turn>,
}

impl Activity {
    /// Applies one event and returns the turn if it changed.
    pub fn apply(&mut self, event: &Event) -> Option<Turn> {
        match event {
            Event::Started { session } => {
                self.running.insert(session.clone());
            }
            Event::Ended { session } => {
                self.running.remove(session);
                self.asks
                    .retain(|_, asker| asker.as_str() != session.as_str());
            }
            Event::Asked(asked) => {
                self.asks.insert(asked.id.clone(), asked.session.clone());
            }
            Event::Replied { request, .. } => {
                self.asks.remove(request);
            }
            Event::Connected | Event::Other => {}
        }
        self.report()
    }

    /// Replaces both sets with what the server says now, as after a reconnect,
    /// and returns the turn if it changed.
    pub fn reset(
        &mut self,
        running: impl IntoIterator<Item = String>,
        asks: impl IntoIterator<Item = (String, String)>,
    ) -> Option<Turn> {
        self.running = running.into_iter().collect();
        self.asks = asks.into_iter().collect();
        self.report()
    }

    /// The turn the sets describe: waiting on the user, then working, then idle.
    fn turn(&self) -> Turn {
        if !self.asks.is_empty() {
            Turn::NeedsInput
        } else if !self.running.is_empty() {
            Turn::Working
        } else {
            Turn::Idle
        }
    }

    /// Returns the current turn unless it is the one last reported.
    fn report(&mut self) -> Option<Turn> {
        let turn = self.turn();
        if self.reported == Some(turn) {
            None
        } else {
            self.reported = Some(turn);
            Some(turn)
        }
    }
}
