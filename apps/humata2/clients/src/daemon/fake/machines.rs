use super::FakeDaemon;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{MachineId, Recovery, RootPlan, SessionId, Status, Turn};

pub(super) fn reconnect(
    daemon: &FakeDaemon,
    machine: &MachineId,
) -> Reply<Vec<(SessionId, Recovery)>> {
    let id = machine.clone();
    daemon.reply(
        Some(Operation::Reconnect),
        format!("reconnect {machine}"),
        move |inner| {
            let found = inner
                .workspace
                .machines
                .iter_mut()
                .find(|found| found.id == id)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Host {id}"),
                })?;
            found.online = true;
            found.last_seen = "now".to_owned();
            found.lost_at = None;
            let machine = found.clone();
            inner.emit(Event::Machine(machine));
            let mut outcomes = Vec::new();
            let recoveries = inner.recoveries.clone();
            let ids: Vec<SessionId> = inner
                .workspace
                .sessions
                .iter()
                .filter(|session| session.machine == id && !session.archived)
                .map(|session| session.id.clone())
                .collect();
            for session in ids {
                let recovery = recoveries
                    .get(&session)
                    .cloned()
                    .unwrap_or(Recovery::Reattached);
                if let Some(found) = inner
                    .workspace
                    .sessions
                    .iter_mut()
                    .find(|found| found.id == session)
                {
                    found.status = match recovery {
                        Recovery::Reattached => Status::Working,
                        Recovery::Resumed => Status::Idle,
                        Recovery::Recoverable { .. } => Status::Exited { code: 137 },
                    };
                    found.turn = Turn::Ended;
                    found.recovery = Some(recovery.clone());
                }
                inner.changed(&session);
                outcomes.push((session, recovery));
            }
            if let Some(health) = inner.workspace.machine_health.get_mut(&id) {
                health.stopped = false;
            }
            inner.emit(Event::Resync);
            Ok(outcomes)
        },
    )
}

pub(super) fn add_machine(daemon: &FakeDaemon, address: &str) -> Reply<MachineId> {
    let address = address.trim().to_owned();
    daemon.reply(None, "add SSH machine".into(), move |inner| {
        if address.is_empty() || address.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(DaemonError::Refused {
                what: "Adding a machine".into(),
                reason: "type one SSH host or user@host without spaces".into(),
            });
        }
        if let Some(machine) = inner.workspace.machines.iter().find(
            |m| matches!(&m.kind,crate::model::MachineKind::Ssh{address:old} if old==&address),
        ) {
            return Ok(machine.id.clone());
        }
        let id = MachineId::new(inner.id("ssh"));
        inner.workspace.machines.push(crate::model::Machine {
            id: id.clone(),
            name: address.rsplit('@').next().unwrap_or(&address).into(),
            kind: crate::model::MachineKind::Ssh { address },
            online: true,
            detail: "Ubuntu 24.04 · ssh".into(),
            last_seen: "now".into(),
            lost_at: None,
            stop: None,
            root: crate::desk::DEFAULT_ROOT.into(),
        });
        inner.workspace.machine_health.insert(
            id.clone(),
            crate::model::MachineHealth {
                system: "Ubuntu 24.04 · ssh".into(),
                daemon: "2.0.3".into(),
                uptime: "just started".into(),
                ..Default::default()
            },
        );
        inner.emit(Event::Resync);
        Ok(id)
    })
}

impl FakeDaemon {
    /// What changing `machine`'s root to `root` would reach in the fake
    /// workspace: the channels that use the machine, and its sessions that
    /// run. Changes nothing.
    pub(crate) fn plan_root(&self, machine: &MachineId, root: &str) -> Result<RootPlan, DaemonError> {
        let inner = self.inner();
        let found = inner.workspace.machine(machine).ok_or_else(|| DaemonError::NotFound {
            what: format!("Host {machine}"),
        })?;
        let running = inner
            .workspace
            .sessions
            .iter()
            .filter(|session| {
                &session.machine == machine
                    && !session.archived
                    && matches!(
                        session.status,
                        Status::Working | Status::NeedsYou | Status::Idle | Status::ProbablyIdle
                    )
            })
            .map(|session| session.id.clone())
            .collect();
        Ok(RootPlan {
            machine: machine.clone(),
            token: format!("plan-{machine}-{root}"),
            old_root: found.root.clone(),
            new_root: root.to_owned(),
            channels: inner
                .workspace
                .channels
                .iter()
                .filter(|channel| channel.machines.contains(machine))
                .map(|channel| channel.id.clone())
                .collect(),
            running,
        })
    }

    /// Makes `root` the machine's Humata folder and says so on the event
    /// feed. The fake keeps no folders, so moving channels changes nothing
    /// more than the root.
    pub(crate) fn apply_root(&self, machine: &MachineId, root: &str) -> Result<(), DaemonError> {
        let mut inner = self.inner();
        let found = inner
            .workspace
            .machines
            .iter_mut()
            .find(|found| &found.id == machine)
            .ok_or_else(|| DaemonError::NotFound { what: format!("Host {machine}") })?;
        found.root = root.to_owned();
        let changed = found.clone();
        inner.emit(Event::Machine(changed));
        Ok(())
    }
}
