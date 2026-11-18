//! A move's machine script triggers (D681): `session-leaving` on the old
//! machine before its agent stops, `session-arrived` on the new one once
//! the move is done. Scripts never fail or hold a move.

use std::time::Duration;

use humata2_machines::Trigger;

use super::Mover;
use crate::machine_scripts::{Context, SessionContext, leaving};

/// The longest a move waits for the old machine's leaving rules.
const LEAVING_WAIT: Duration = Duration::from_secs(60);

impl Mover {
    fn script_context(&self, machine: &str, trigger: Trigger) -> Context {
        // The session's place under the machine's channels folder.
        let folder = if self.channel.folder.is_empty() {
            self.folder.clone()
        } else {
            format!("{}/{}", self.channel.folder, self.folder)
        };
        Context {
            machine: machine.to_owned(),
            trigger,
            session: Some(SessionContext {
                session: self.session.clone(),
                channel: self.channel.name.clone(),
                folder,
                from: self.from.clone(),
                to: self.to.clone(),
            }),
        }
    }

    /// Runs the old machine's `session-leaving` rules and waits for them,
    /// for a minute at most. This Mac has no triggers.
    pub(super) async fn script_leaving(&self) {
        if self.carry.is_host(&self.from) {
            return;
        }
        let Some(scripts) = self.svc.machine_scripts() else {
            return;
        };
        let context = self.script_context(&self.from, Trigger::SessionLeaving);
        if !leaving(&scripts, context, LEAVING_WAIT).await {
            crate::log::say!(
                "humatad2: the session-leaving scripts on {} were still running after a minute; session {}'s move went on without them",
                self.from,
                self.session
            );
        }
    }

    /// Fires the new machine's `session-arrived` rules in the background.
    pub(super) fn script_arrived(&self) {
        if self.carry.is_host(&self.to) {
            return;
        }
        if let Some(scripts) = self.svc.machine_scripts() {
            scripts.fire(self.script_context(&self.to, Trigger::SessionArrived));
        }
    }
}
