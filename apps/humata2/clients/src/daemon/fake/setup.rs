//! The fake's channel setup (frame 45, CH-5): folders made, then one clone
//! per repository and host, scripted instead of run.

use super::{FakeDaemon, Inner};
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{Channel, ChannelId, ChannelSetup, MachineId, SetupId, SetupState, SetupStep};

/// A repository whose name holds this fails its first clone, so tests can
/// show the failed row and Retry. A retry succeeds.
const FAILING: &str = "broken";

/// Scripts the setup of channel `name`: pushes its progress, keeps it in the
/// workspace, and says why it stopped if a clone failed. When it works the
/// setup stays at `Done` until [`finish`] pushes it, after the channel
/// exists.
pub(super) fn start(
    inner: &mut Inner,
    name: &str,
    repos: &[String],
    machines: &[MachineId],
) -> Result<(), DaemonError> {
    if machines.is_empty() {
        return Ok(());
    }
    let mut steps = Vec::new();
    let mut running = Vec::new();
    let mut failed = None;
    for machine in machines {
        let step = |repo: Option<String>, state| SetupStep {
            machine: machine.clone(),
            repo,
            state,
        };
        steps.push(step(None, SetupState::Done));
        running.push(step(None, SetupState::Done));
        for repo in repos {
            if repo.contains(FAILING) {
                failed.get_or_insert_with(|| repo.clone());
                steps.push(step(
                    Some(repo.clone()),
                    SetupState::Failed {
                        reason: format!(
                            "{machine} lost its connection to github.com. Check its network, then Retry."
                        ),
                    },
                ));
            } else {
                steps.push(step(Some(repo.clone()), SetupState::Done));
            }
            running.push(step(
                Some(repo.clone()),
                SetupState::Running {
                    percent: 64,
                    detail: "Receiving objects".to_owned(),
                },
            ));
        }
    }
    let id = SetupId(inner.id("setup"));
    let mut setup = ChannelSetup {
        id,
        name: name.to_owned(),
        steps: running,
        cancellable: true,
    };
    inner.emit(Event::ChannelSetup(setup.clone()));
    setup.steps = steps;
    setup.cancellable = failed.is_none();
    inner.workspace.setups.push(setup.clone());
    match failed {
        Some(repo) => {
            inner.emit(Event::ChannelSetup(setup));
            Err(DaemonError::Refused {
                what: format!("Cloning {repo} for #{name}"),
                reason: "it failed. Check the host's network, then Retry".to_owned(),
            })
        }
        None => Ok(()),
    }
}

/// Pushes the finished setup of `name`, once its channel exists, and forgets
/// it.
pub(super) fn finish(inner: &mut Inner, name: &str) {
    let Some(at) = inner
        .workspace
        .setups
        .iter()
        .position(|setup| setup.name == name)
    else {
        return;
    };
    let mut setup = inner.workspace.setups.remove(at);
    setup.cancellable = false;
    inner.emit(Event::ChannelSetup(setup));
}

pub(super) fn cancel_setup(daemon: &FakeDaemon, setup: &SetupId) -> Reply<()> {
    let setup = setup.clone();
    daemon.reply(
        Some(Operation::CancelSetup),
        format!("cancel setup {}", setup.0),
        move |inner| {
            let at = position(inner, &setup)?;
            inner.workspace.setups.remove(at);
            Ok(())
        },
    )
}

pub(super) fn retry_setup(daemon: &FakeDaemon, setup: &SetupId) -> Reply<ChannelId> {
    let setup = setup.clone();
    daemon.reply(
        Some(Operation::RetrySetup),
        format!("retry setup {}", setup.0),
        move |inner| {
            let at = position(inner, &setup)?;
            for step in &mut inner.workspace.setups[at].steps {
                if matches!(step.state, SetupState::Failed { .. }) {
                    step.state = SetupState::Done;
                }
            }
            let mut found = inner.workspace.setups[at].clone();
            found.cancellable = false;
            let name = found.name.clone();
            if inner.workspace.channels.iter().all(|c| c.name != name) {
                let mut repos: Vec<String> = Vec::new();
                let mut machines: Vec<MachineId> = Vec::new();
                for step in &found.steps {
                    if let Some(repo) = &step.repo
                        && !repos.contains(repo)
                    {
                        repos.push(repo.clone());
                    }
                    if !machines.contains(&step.machine) {
                        machines.push(step.machine.clone());
                    }
                }
                inner.workspace.channels.push(Channel {
                    id: ChannelId::new(name.clone()),
                    name: name.clone(),
                    repos,
                    machines,
                    purpose: String::new(),
                    default_agent: None,
                });
            }
            inner.workspace.setups.remove(at);
            inner.emit(Event::ChannelSetup(found));
            inner.emit(Event::Resync);
            Ok(ChannelId::new(name))
        },
    )
}

fn position(inner: &Inner, setup: &SetupId) -> Result<usize, DaemonError> {
    inner
        .workspace
        .setups
        .iter()
        .position(|found| &found.id == setup)
        .ok_or_else(|| DaemonError::NotFound {
            what: "That channel setup".to_owned(),
        })
}
