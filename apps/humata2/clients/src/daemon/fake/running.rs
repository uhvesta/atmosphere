//! The holders running on each machine in the fake (D784): a test seeds
//! them; a list reads one machine's; Stop keeps a shell to resume, Resume
//! revives it and End removes it. An agent's tab is refused, as the daemon
//! refuses it.

use super::{FakeDaemon, Inner};
use crate::daemon::{DaemonError, Event, HolderTab, Operation, Reply, RunningHolder};
use crate::model::{Agent, ChannelId, MachineId, SessionId, Status};

impl FakeDaemon {
    /// Makes the machines list exactly `holders` from now on.
    pub fn set_running_holders(&self, holders: Vec<RunningHolder>) {
        self.inner().running = holders;
    }

    /// The holders every machine lists now.
    #[must_use]
    pub fn running_holders_now(&self) -> Vec<RunningHolder> {
        self.inner().running.clone()
    }
}

/// An offline machine answers nothing; one the workspace doesn't know is
/// taken to answer, as a test may seed holders without the machine.
fn reachable(inner: &Inner, machine: &MachineId) -> Result<(), DaemonError> {
    match inner.workspace.machine(machine) {
        Some(found) if !found.online => Err(DaemonError::Offline { machine: machine.clone() }),
        _ => Ok(()),
    }
}

/// Where the shell `tab` names is in the list, `None` for a session's shell
/// tab the test didn't seed. An agent's tab is refused, as the daemon
/// refuses it.
fn shell(inner: &Inner, machine: &MachineId, tab: &HolderTab, what: &str) -> Result<Option<usize>, DaemonError> {
    let agent_tab = || DaemonError::Refused {
        what: what.to_owned(),
        reason: "it is an agent's tab; stop, resume or archive its session instead".to_owned(),
    };
    match tab {
        HolderTab::Listed(id) => {
            let at = inner
                .running
                .iter()
                .position(|holder| &holder.machine == machine && holder.tab.as_deref() == Some(id.as_str()))
                .ok_or_else(|| DaemonError::NotFound {
                    what: "That terminal".to_owned(),
                })?;
            if inner.running[at].session.is_some() && inner.running[at].agent != Agent::Shell {
                return Err(agent_tab());
            }
            Ok(Some(at))
        }
        HolderTab::Session { session, index } => {
            let found = inner.workspace.session(session).ok_or_else(|| DaemonError::NotFound {
                what: format!("Session {session}"),
            })?;
            if *index == 0 && found.agent != Agent::Shell {
                return Err(agent_tab());
            }
            Ok(inner.running.iter().position(|holder| {
                &holder.machine == machine && holder.session.as_ref() == Some(session) && holder.agent == Agent::Shell
            }))
        }
    }
}

pub(super) fn running_holders(daemon: &FakeDaemon, machine: &MachineId) -> Reply<Vec<RunningHolder>> {
    let machine = machine.clone();
    daemon.reply(
        Some(Operation::RunningHolders),
        format!("list running holders on {machine}"),
        move |inner| {
            reachable(inner, &machine)?;
            let mut listed: Vec<_> = inner.running.iter().filter(|h| h.machine == machine).cloned().collect();
            // Running sessions by title, stopped sessions by title, then
            // orphans by start.
            let group = |holder: &RunningHolder| match (holder.is_orphan(), holder.stopped) {
                (false, false) => 0,
                (false, true) => 1,
                (true, _) => 2,
            };
            listed.sort_by(|a, b| {
                group(a).cmp(&group(b)).then_with(|| {
                    if a.is_orphan() {
                        a.started_unix_ms.cmp(&b.started_unix_ms)
                    } else {
                        a.title.cmp(&b.title)
                    }
                })
            });
            Ok(listed)
        },
    )
}

pub(super) fn stop_running_holder(daemon: &FakeDaemon, machine: &MachineId, tab: &HolderTab) -> Reply<()> {
    let (machine, tab) = (machine.clone(), tab.clone());
    daemon.reply(
        Some(Operation::StopRunningHolder),
        format!("stop running holder {tab:?} on {machine}"),
        move |inner| {
            reachable(inner, &machine)?;
            if let Some(at) = shell(inner, &machine, &tab, "Stopping that terminal")? {
                let holder = &mut inner.running[at];
                holder.stopped = true;
                holder.pid = None;
                holder.started_unix_ms = None;
                holder.attached = false;
            }
            Ok(())
        },
    )
}

pub(super) fn resume_running_holder(daemon: &FakeDaemon, machine: &MachineId, tab: &HolderTab) -> Reply<()> {
    let (machine, tab) = (machine.clone(), tab.clone());
    daemon.reply(
        Some(Operation::ResumeRunningHolder),
        format!("resume running holder {tab:?} on {machine}"),
        move |inner| {
            reachable(inner, &machine)?;
            if let Some(at) = shell(inner, &machine, &tab, "Resuming that terminal")? {
                inner.running[at].stopped = false;
            }
            Ok(())
        },
    )
}

pub(super) fn end_running_holder(daemon: &FakeDaemon, machine: &MachineId, tab: &HolderTab) -> Reply<()> {
    let (machine, tab) = (machine.clone(), tab.clone());
    daemon.reply(
        Some(Operation::EndRunningHolder),
        format!("end running holder {tab:?} on {machine}"),
        move |inner| {
            reachable(inner, &machine)?;
            if let Some(at) = shell(inner, &machine, &tab, "Ending that terminal")? {
                inner.running.remove(at);
            }
            Ok(())
        },
    )
}

/// The orphan `tab` names now names a session, in `channel` or on the
/// Desk; one a session already names is refused, as the daemon refuses it.
pub(super) fn adopt_running_holder(
    daemon: &FakeDaemon,
    machine: &MachineId,
    tab: &str,
    channel: Option<&ChannelId>,
) -> Reply<SessionId> {
    let (machine, tab, channel) = (machine.clone(), tab.to_owned(), channel.cloned());
    daemon.reply(
        Some(Operation::AdoptRunningHolder),
        format!("adopt running holder {tab} on {machine}"),
        move |inner| {
            reachable(inner, &machine)?;
            let holder = inner
                .running
                .iter_mut()
                .find(|holder| holder.machine == machine && holder.tab.as_deref() == Some(tab.as_str()))
                .ok_or_else(|| DaemonError::NotFound {
                    what: "That terminal".to_owned(),
                })?;
            if !holder.is_orphan() {
                return Err(DaemonError::Refused {
                    what: "Adopting that terminal".to_owned(),
                    reason: "a session already has it; open that session instead".to_owned(),
                });
            }
            let session = SessionId::new(format!("adopted-{tab}"));
            holder.session = Some(session.clone());
            holder.desk = channel.is_none();
            holder.channel = channel;
            Ok(session)
        },
    )
}

pub(super) fn restore_session_stopped(daemon: &FakeDaemon, session: &SessionId) -> Reply<()> {
    let (this, id) = (daemon.clone(), session.clone());
    let restored = super::sessions::restore_session(daemon, session);
    Box::pin(async move {
        restored.await?;
        let mut inner = this.inner();
        if let Some(found) = inner.workspace.sessions.iter_mut().find(|found| found.id == id) {
            found.status = Status::Stopped;
            let changed = found.clone();
            inner.emit(Event::Session(changed));
        }
        Ok(())
    })
}
