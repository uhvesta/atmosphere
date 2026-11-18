//! A machine's sessions stopped and started in the fake workspace, for
//! [`crate::machines::FakeMachines`] (D260–D265). Add `mod stop;` to
//! `daemon/fake.rs`.

use super::FakeDaemon;
use crate::daemon::Event;
use crate::model::{Machine, MachineId, MachineStop, RequestKind, Status, Turn};

impl FakeDaemon {
    /// The workspace's `machine`, as it is now.
    pub(crate) fn workspace_machine(&self, machine: &MachineId) -> Option<Machine> {
        self.inner().workspace.machine(machine).cloned()
    }

    /// Keeps `stop` on `machine`, or clears it with `None`, and says so on
    /// the event feed. A delivered stop makes the machine's sessions read
    /// Stopped, except those whose process had already exited; an undelivered one leaves them as they were, since the
    /// machine hasn't heard of it. Clearing it brings Stopped sessions back
    /// Idle, as a start resumes them. A delivered stop also withdraws the
    /// machine's sessions' pending Inbox requests, as the daemon does
    /// ("stopped by you", D264); Migrate items stay. An unknown machine changes nothing.
    pub(crate) fn set_machine_stop(&self, machine: &MachineId, stop: Option<MachineStop>) {
        let mut inner = self.inner();
        let Some(found) = inner.workspace.machines.iter_mut().find(|found| &found.id == machine) else {
            return;
        };
        let delivered = stop.as_ref().is_some_and(|stop| stop.delivered);
        found.stop = stop;
        let mut events = vec![Event::Machine(found.clone())];
        for session in inner
            .workspace
            .sessions
            .iter_mut()
            .filter(|session| &session.machine == machine && !session.archived)
        {
            let status = match (delivered, session.status) {
                // A process that had already ended has nothing to stop or bring back.
                (_, Status::Exited { code }) => Status::Exited { code },
                (true, _) => Status::Stopped,
                (false, Status::Stopped) => Status::Idle,
                (false, status) => status,
            };
            if status != session.status {
                session.status = status;
                session.turn = Turn::Ended;
                events.push(Event::Session(session.clone()));
            }
        }
        if delivered {
            let on_machine: Vec<_> = inner
                .workspace
                .sessions
                .iter()
                .filter(|session| &session.machine == machine)
                .map(|session| session.id.clone())
                .collect();
            let withdrawn: Vec<_> = inner
                .workspace
                .requests
                .iter()
                .filter(|request| {
                    request.answer.is_none()
                        && request.kind != RequestKind::Migrate
                        && on_machine.contains(&request.session)
                })
                .map(|request| request.id.clone())
                .collect();
            inner.workspace.requests.retain(|request| !withdrawn.contains(&request.id));
            events.extend(withdrawn.into_iter().map(Event::RequestGone));
        }
        for event in events {
            inner.emit(event);
        }
    }
}
