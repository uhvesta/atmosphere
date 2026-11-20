//! Adding, reconnecting and removing SSH machines (HO-1, HO-6, LD-10) over
//! [`Setup`], with `hosts.json` recording how far each got so a restart at
//! any step finishes or undoes it (PER-27):
//!
//! - Add writes the entry as `Adding` before it touches the machine and as
//!   `Ready` only once the service runs. A failed add is undone at once
//!   (humatad2's files and service removed, the host key forgotten, the
//!   entry dropped), so **Try again** is a fresh add whose install skips
//!   files already in place. An add cut short by a restart is undone by
//!   [`Machines::recover`].
//! - Remove writes `Removing` first, then removes humatad2 from the
//!   machine, forgets its host key and drops the entry; one cut short
//!   finishes at the next recover.
//! - An undo or removal that can't reach the machine keeps the entry
//!   (`undo_pending`, or `Removing`) and runs again at the next recover or
//!   reconnect.
//! - Add writes a fresh enrollment key on the machine in its install step
//!   and keeps the host's copy once the rest is set up (GRPC.md §8, D83);
//!   a reconnect writes one for a machine the host has no copy for. Undoing
//!   an add and removing a machine delete the host's copy.
//!
//! No async and no clock: the daemon passes a [`Runner`], the builds, and
//! how to wait, and gets each step's progress pushed to it as it happens
//! (TR-7).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use thiserror::Error;

use crate::check::Facts;
use crate::enroll::{self, Key};
use crate::error::{IdentityError, KnownHostsError, NameError, QuoteError, Remedy, RunError, SetupError, ValuesError};
use crate::hosts::{EntryState, HostEntry, Hosts, HostsError, ProviderMachine};
use crate::identity::{self, SshAccess, SshTarget};
use crate::install::Builds;
use crate::known_hosts;
use crate::link::{Link, BIN_DIR};
use crate::name::MachineName;
use crate::provider::{
    self, parse_list, parse_status_report, Action, DiscoveredInstance, MachineStatus, Provider, Reach,
};
use crate::quote;
use crate::runner::{Exit, Invocation, Output, Purpose, Runner};
use crate::setup::{self, Progress, RetryPolicy, Setup};
use crate::stack_config::{self, StackConfig, StackConfigError};

/// How many output lines a failed step keeps (MP-20).
const TAIL_LINES: usize = 40;
/// Where channels live on a machine unless the user says otherwise (FC-10).
pub const DEFAULT_ROOT: &str = "~/Humata";

/// The steps an operation shows (GRPC.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpStep {
    /// The provider's `create` command (MP-19).
    Create,
    /// The provider's `identity` command (MP-8).
    Identity,
    Connect,
    Check,
    Install,
    Start,
    FindAgents,
    /// Removing humatad2 from the machine (remove, or undoing an add).
    Uninstall,
    /// Forgetting the host key and dropping the entry.
    Forget,
    /// One of the provider's own commands: start, stop, status, or the
    /// delete that follows removing a machine (MP-21).
    Provider,
}

impl OpStep {
    /// The steps of Add machine, in order.
    pub const ADD: [Self; 5] = [Self::Connect, Self::Check, Self::Install, Self::Start, Self::FindAgents];
    /// The steps of Reconnect.
    pub const RECONNECT: [Self; 4] = [Self::Connect, Self::Check, Self::Install, Self::Start];
    /// The steps of Remove machine.
    pub const REMOVE: [Self; 2] = [Self::Uninstall, Self::Forget];
    /// The steps of a provider's start, stop or status.
    pub const PROVIDER_ACTION: [Self; 1] = [Self::Provider];
    /// The steps of a provider machine's Delete: remove it, then the
    /// provider's `delete` (MP-21).
    pub const PROVIDER_DELETE: [Self; 3] = [Self::Uninstall, Self::Forget, Self::Provider];

    /// The steps of Create machine through a provider that reaches its
    /// machines as `reach` says (MP-19).
    pub fn create_steps(reach: Reach) -> Vec<Self> {
        let mut steps = vec![Self::Create];
        if reach == Reach::Identity {
            steps.push(Self::Identity);
        }
        steps.extend([Self::Connect, Self::Check, Self::Install, Self::Start, Self::FindAgents]);
        steps
    }

    /// Stable, as `OperationStep.key`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Identity => "identity",
            Self::Provider => "provider",
            Self::Connect => "connect",
            Self::Check => "check",
            Self::Install => "install",
            Self::Start => "start",
            Self::FindAgents => "find_agents",
            Self::Uninstall => "stop_daemon",
            Self::Forget => "forget",
        }
    }

    /// As shown.
    pub fn title(self) -> &'static str {
        match self {
            Self::Create => "Creating the machine",
            Self::Identity => "Asking the provider how to reach it",
            Self::Provider => "Running the provider's command",
            Self::Connect => "Connecting",
            Self::Check => "Checking the machine",
            Self::Install => "Installing Humata's tools",
            Self::Start => "Starting humatad2",
            Self::FindAgents => "Finding agents",
            Self::Uninstall => "Removing humatad2 from the machine",
            Self::Forget => "Forgetting the machine",
        }
    }

    fn of(step: setup::Step) -> Self {
        match step {
            setup::Step::Create => Self::Create,
            setup::Step::Identity => Self::Identity,
            setup::Step::Connect => Self::Connect,
            setup::Step::Check => Self::Check,
            setup::Step::Install => Self::Install,
            setup::Step::Service => Self::Start,
        }
    }
}

/// What happened to a step, pushed as it happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepEvent {
    Running(OpStep),
    /// The machine didn't answer yet; the next attempt is after `delay`.
    Retrying {
        step: OpStep,
        attempt: u32,
        of: u32,
        delay: Duration,
    },
    Done(OpStep),
    /// What the check step read from the machine (HO-1); pushed before the
    /// check's `Done`.
    Checked(Box<Facts>),
    Skipped(OpStep),
    /// With the step's kept output (MP-20).
    Failed { step: OpStep, output_tail: String },
    /// Undone after a later step failed.
    Undone(OpStep),
}

/// Tells the caller what the check read, as that step finishes.
fn push_checked(facts: Option<&Facts>, step: OpStep, drive: &mut Drive<'_>) {
    if let (OpStep::Check, Some(facts)) = (step, facts) {
        (drive.events)(StepEvent::Checked(Box::new(facts.clone())));
    }
}

/// What the daemon lends an operation: how to run commands, the builds to
/// install, how to wait between connect attempts, and where progress goes.
pub struct Drive<'a> {
    pub runner: &'a dyn Runner,
    pub builds: &'a dyn Builds,
    /// Blocks for the given time; tests pass one that returns at once.
    pub wait: &'a dyn Fn(Duration),
    pub events: &'a mut dyn FnMut(StepEvent),
    /// True once the user cancelled: an add stops at the next step
    /// boundary and is undone (GRPC.md §7). Work that can't be cancelled
    /// never asks.
    pub cancelled: &'a dyn Fn() -> bool,
}

/// A step boundary: the cancel the user asked for, if any.
fn stop_if_cancelled(machine: &MachineName, drive: &Drive<'_>) -> Result<(), ManageError> {
    if (drive.cancelled)() {
        return Err(ManageError::Cancelled {
            machine: machine.as_str().to_owned(),
        });
    }
    Ok(())
}

/// Add machine over SSH (HO-1, MP-10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddSsh {
    /// An alias from the ssh config, or `user@host[:port]`.
    pub ssh_host: String,
    /// Empty: made from the host.
    pub name: String,
    /// Empty: [`DEFAULT_ROOT`].
    pub root: String,
    /// The `SHA256:…` fingerprint the user confirmed; none on the first try,
    /// which fails with [`ManageError::HostKeyUnconfirmed`] carrying the
    /// fingerprint to show.
    pub trust_fingerprint: Option<String>,
}

/// An add that passed every check that needs no machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAdd {
    pub machine: MachineName,
    pub target: SshTarget,
    pub root: String,
}

/// A create that passed every check that needs no machine.
#[derive(Debug, Clone)]
pub struct PlannedCreate {
    pub machine: MachineName,
    pub provider: Provider,
    pub values: BTreeMap<String, String>,
}

/// What [`Machines::recover`] did with an entry left mid-way.
#[derive(Debug)]
pub struct Recovered {
    pub machine: String,
    /// The state it was found in.
    pub found: EntryState,
    /// Ok when it was undone or finished; the error when it waits for the
    /// machine (its entry is kept for the next try).
    pub outcome: Result<(), ManageError>,
}

#[derive(Debug, Error)]
pub enum ManageError {
    #[error(
        "`{value}` isn't an SSH host Humata can use: {reason}. Type an alias from ~/.ssh/config, or user@host, or user@host:port"
    )]
    InvalidSshHost { value: String, reason: &'static str },
    #[error("`{value}` can't name a machine: {source}")]
    BadName {
        value: String,
        #[source]
        source: NameError,
    },
    #[error(
        "a machine named {machine} is already added. Pick another name, or remove {machine} first"
    )]
    NameTaken { machine: String },
    /// The name is still the machine's whose removal was asked for and
    /// hasn't finished (it may be waiting for the machine to come back).
    #[error(
        "{machine} is still being removed, so its name isn't free yet. Wait for the removal to finish (Hosts shows it until then; one waiting for the machine finishes once the machine is back or Humata restarts), then add it again. Picking another name adds it now, but that is a second machine, not this one added again"
    )]
    BeingRemoved { machine: String },
    #[error("there is no machine named {machine}. List the machines to see their names")]
    NotFound { machine: String },
    #[error(
        "{machine} is busy with another add, reconnect or remove. Wait for it to finish, then try again"
    )]
    Busy { machine: String },
    #[error(
        "{machine} is still being added or removed. Wait for that to finish (or for the machine to come back so it can), then try again"
    )]
    NotReady { machine: String },
    #[error(
        "{machine} presents its {algorithm} host key, {fingerprint}, which Humata hasn't seen before. Compare it with the machine's own (`ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` on it), then add the machine again with this fingerprint to trust it"
    )]
    HostKeyUnconfirmed {
        machine: String,
        /// The key type, such as `ssh-ed25519`.
        algorithm: String,
        fingerprint: String,
    },
    #[error(
        "{machine} presents host key {presented}, not the {expected} you confirmed, so nothing was sent to it. Someone may be intercepting the connection: check the fingerprint on the machine itself before trusting a different key"
    )]
    HostKeyMismatch {
        machine: String,
        expected: String,
        presented: String,
    },
    #[error("{step:?} failed on {machine}")]
    Step {
        machine: String,
        step: OpStep,
        output_tail: String,
        remedies: Vec<Remedy>,
        #[source]
        source: SetupError,
    },
    #[error(
        "{machine} asks for a password, passphrase or code. Make key login work without one (ssh-copy-id, or load the key into ssh-agent), then try again"
    )]
    LoginNeeded {
        machine: String,
        output_tail: String,
        /// Why: the prompt it printed, or that it hung (D167).
        reason: setup::LoginReason,
    },
    #[error(
        "couldn't remove humatad2 from {machine} ({exit}). It is listed until that works; reconnect when the machine is back to finish, or forget it if it is gone for good"
    )]
    UninstallFailed {
        machine: String,
        exit: Exit,
        output_tail: String,
    },
    #[error("couldn't run the command for {machine}: {source}")]
    Run {
        machine: String,
        #[source]
        source: RunError,
    },
    #[error("couldn't build the command for {machine}: {source}")]
    Quote {
        machine: String,
        #[source]
        source: QuoteError,
    },
    #[error(
        "adding {machine} was cancelled, and what it had done there was undone. Add the machine again when you want it"
    )]
    Cancelled { machine: String },
    #[error(
        "couldn't save the peer link's key for {machine} in {}, so the add was undone: {source}. Check that the folder is yours and the disk isn't full, then add the machine again",
        .path.display()
    )]
    SavePeerKey {
        machine: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(
        "couldn't delete the peer link's key for {machine} at {}: {source}. Delete that file by hand; until then the key stays on this Mac",
        .path.display()
    )]
    DeletePeerKey {
        machine: String,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("can't use the params given for the machine: {source}")]
    Params {
        #[source]
        source: ValuesError,
    },
    #[error(
        "the {provider} provider runs its commands on another machine (`run_on`), which this build of humatad2 can't do yet. Remove `run_on` from the provider to run its commands on this Mac"
    )]
    RunOnUnsupported { provider: String },
    #[error(
        "{machine} needs a login only a person can give, and Create machine can't ask for one. Add the machine with its provider's key login set up, or make the provider's connect command log in without a prompt"
    )]
    ProviderLoginNeeded {
        machine: String,
        output_tail: String,
        /// Why: the prompt it printed, or that it hung (D167).
        reason: setup::LoginReason,
    },
    #[error("{machine} isn't a machine the {provider} provider made. Pick a machine of that provider")]
    NotProviderMachine { machine: String, provider: String },
    #[error(
        "the {provider} provider has no `{}` command, so there is nothing to run. Add `{}` to the provider in Settings › Machines › Providers",
        .action.field(), .action.field()
    )]
    ActionUndefined { provider: String, action: Action },
    #[error(
        "the provider's {} command for {machine} failed ({exit}). The step's output says why; fix the command in Settings › Machines › Providers or the machine itself, then run it again",
        .action.field()
    )]
    ActionFailed {
        machine: String,
        action: Action,
        exit: Exit,
        output_tail: String,
    },
    #[error(
        "{machine} was removed from Humata, but the provider's delete command failed ({exit}), so what the provider made may still exist and cost money. Delete it by hand with the provider's own tool"
    )]
    DeleteFailed {
        machine: String,
        exit: Exit,
        output_tail: String,
    },
    #[error(
        "Humata lost how to reach {machine} (its provider's link is missing from the machine list). Remove the machine and add it again"
    )]
    ProviderLinkLost { machine: String },
    #[error(
        "`{name}` can't be a variable name for a machine script. Use capital letters, digits and `_`, not starting with a digit"
    )]
    BadEnvName { name: String },
    #[error(transparent)]
    Hosts(#[from] HostsError),
    #[error(transparent)]
    KnownHosts(#[from] KnownHostsError),
}

/// What a machine script runs (MACHINESCRIPTS).
#[derive(Clone, PartialEq, Eq)]
pub enum ScriptBody {
    /// A bash script, sent on stdin.
    Bash(Arc<[u8]>),
    /// The machine's own `humata` with these arguments.
    Humata(Vec<String>),
}

impl std::fmt::Debug for ScriptBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bash(bytes) => write!(f, "Bash({} bytes)", bytes.len()),
            Self::Humata(words) => f.debug_tuple("Humata").field(words).finish(),
        }
    }
}

/// Where the machine list and Humata's own `known_hosts` live, and the ssh
/// config aliases come from.
#[derive(Debug, Clone)]
pub struct MachinesConfig {
    pub hosts_file: PathBuf,
    pub known_hosts: PathBuf,
    /// `~/.ssh/config` in the app; a file of the test's own in tests. A
    /// missing file means no aliases.
    pub ssh_config: PathBuf,
    /// The folder of the host's copies of machines' enrollment keys.
    pub peer_keys: PathBuf,
    pub retry: RetryPolicy,
}

/// The host's machines.
#[derive(Debug)]
pub struct Machines {
    hosts: Hosts,
    known_hosts: PathBuf,
    ssh_config: PathBuf,
    peer_keys: PathBuf,
    retry: RetryPolicy,
    busy: Mutex<BTreeSet<String>>,
    /// Machines whose removal was asked for and hasn't ended: an add of
    /// the same name is told so, not that the name is taken.
    removing: Mutex<BTreeSet<String>>,
}

/// Marks a machine busy until dropped.
pub(crate) struct BusyGuard<'a> {
    busy: &'a Mutex<BTreeSet<String>>,
    machine: String,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.busy
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.machine);
    }
}

impl Machines {
    pub fn open(config: MachinesConfig) -> Self {
        Self {
            hosts: Hosts::open(&config.hosts_file),
            known_hosts: config.known_hosts,
            ssh_config: config.ssh_config,
            peer_keys: config.peer_keys,
            retry: config.retry,
            busy: Mutex::new(BTreeSet::new()),
            removing: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn list(&self) -> Result<Vec<HostEntry>, ManageError> {
        Ok(self.hosts.list()?)
    }

    /// Makes `root` the folder new channels use on `machine` (FC-10).
    /// Channels already there stay where they are.
    pub fn set_root(&self, machine: &str, root: &str) -> Result<(), ManageError> {
        if self.hosts.set_root(machine, root)? {
            Ok(())
        } else {
            Err(ManageError::NotFound {
                machine: machine.to_owned(),
            })
        }
    }

    pub fn get(&self, machine: &str) -> Result<HostEntry, ManageError> {
        self.hosts.get(machine)?.ok_or_else(|| ManageError::NotFound {
            machine: machine.to_owned(),
        })
    }

    /// Pins the host key `machine` presents now in place of the one pinned,
    /// when its fingerprint is `fingerprint`, the one the user checked on the
    /// machine itself (MP-9). A machine that presents another key (one that
    /// changed again, or an interception) fails with
    /// [`ManageError::HostKeyMismatch`] and nothing is pinned.
    pub fn trust_new_host_key(&self, machine: &str, fingerprint: &str, runner: &dyn Runner) -> Result<(), ManageError> {
        let _busy = self.claim(machine)?;
        let entry = self.get(machine)?;
        let link = need_link(self.link_if_any(&entry)?, &entry)?;
        let presented = Setup::existing(link.clone(), &self.known_hosts)
            .scan_host_key(runner)
            .map_err(|failure| ManageError::Step {
                machine: machine.to_owned(),
                step: OpStep::Connect,
                output_tail: failure.output_tail,
                remedies: failure.remedies,
                source: failure.error,
            })?;
        let expected = fingerprint.trim();
        if expected != presented.fingerprint() {
            return Err(ManageError::HostKeyMismatch {
                machine: machine.to_owned(),
                expected: expected.to_owned(),
                presented: presented.fingerprint().to_owned(),
            });
        }
        known_hosts::replace(&self.known_hosts, &known_hosts::alias(link.machine()), &presented)?;
        Ok(())
    }

    /// Discovers existing instances via a provider's `list` command.
    pub fn list_instances(
        &self,
        provider: &Provider,
        runner: &dyn Runner,
    ) -> Result<Vec<DiscoveredInstance>, ManageError> {
        let invocation = provider
            .list_invocation()
            .ok_or_else(|| ManageError::ActionUndefined {
                provider: provider.id().to_owned(),
                action: Action::List,
            })?;
        let output = runner.run(&invocation).map_err(|source| ManageError::Run {
            machine: provider.id().to_owned(),
            source,
        })?;
        let secret = identity::has_key_material(&output.stdout)
            || identity::has_key_material(&output.stderr);
        if secret {
            return Err(ManageError::Step {
                machine: provider.id().to_owned(),
                step: OpStep::Provider,
                output_tail: String::new(),
                remedies: vec![Remedy::TryAgain, Remedy::FixProvider],
                source: SetupError::Identity(IdentityError::KeyMaterial),
            });
        }
        if !output.exit.success() {
            return Err(ManageError::ActionFailed {
                machine: provider.id().to_owned(),
                action: Action::List,
                exit: output.exit,
                output_tail: tail(&output),
            });
        }
        parse_list(&output.stdout).map_err(|err| ManageError::Step {
            machine: provider.id().to_owned(),
            step: OpStep::Provider,
            output_tail: tail(&output),
            remedies: vec![Remedy::TryAgain, Remedy::FixProvider],
            source: SetupError::Identity(err),
        })
    }

    /// Refreshes how Humata reaches a provider machine: runs `refresh` (if
    /// declared), `wake` (if sleeping, or stopped with `wake_on_demand`), and
    /// `identity` (if declared), atomically updates `StoredLink` in
    /// `hosts.json`, and updates attested `known_hosts`.
    pub fn refresh_reach(
        &self,
        provider: &Provider,
        name: &str,
        runner: &dyn Runner,
    ) -> Result<Link, ManageError> {
        let _busy = self.claim(name)?;
        let entry = self.get(name)?;
        let record = provider_record(&entry, provider)?;
        let machine = machine_name(&entry)?;
        let params = record.params.clone();

        let run_action = |action: Action| -> Result<Output, ManageError> {
            let invocation = provider
                .invocation(action, &machine, &params)
                .map_err(|source| ManageError::Params { source })?
                .ok_or_else(|| ManageError::ActionUndefined {
                    provider: provider.id().to_owned(),
                    action,
                })?;
            let output = runner.run(&invocation).map_err(|source| ManageError::Run {
                machine: name.to_owned(),
                source,
            })?;
            if !output.exit.success() {
                let secret = identity::has_key_material(&output.stdout)
                    || identity::has_key_material(&output.stderr);
                if secret {
                    return Err(ManageError::Step {
                        machine: name.to_owned(),
                        step: OpStep::Identity,
                        output_tail: String::new(),
                        remedies: vec![Remedy::TryAgain, Remedy::FixProvider],
                        source: SetupError::Identity(IdentityError::KeyMaterial),
                    });
                }
                return Err(ManageError::ActionFailed {
                    machine: name.to_owned(),
                    action,
                    exit: output.exit,
                    output_tail: tail(&output),
                });
            }
            Ok(output)
        };

        let parse_id = |output: &Output| -> Result<identity::Identity, ManageError> {
            let secret = identity::has_key_material(&output.stdout)
                || identity::has_key_material(&output.stderr);
            if secret {
                return Err(ManageError::Step {
                    machine: name.to_owned(),
                    step: OpStep::Identity,
                    output_tail: String::new(),
                    remedies: vec![Remedy::TryAgain, Remedy::FixProvider],
                    source: SetupError::Identity(IdentityError::KeyMaterial),
                });
            }
            identity::parse_identity(&output.stdout, &machine).map_err(|err| ManageError::Step {
                machine: name.to_owned(),
                step: OpStep::Identity,
                output_tail: tail(output),
                remedies: vec![Remedy::TryAgain, Remedy::FixProvider],
                source: SetupError::Identity(err),
            })
        };

        let should_wake = |state: &MachineStatus| -> bool {
            *state == MachineStatus::Sleeping
                || (*state == MachineStatus::Stopped && provider.wake_on_demand())
        };

        // 1. Declarative refresh before checking/waking/reconnecting.
        if provider.has(Action::Refresh) {
            run_action(Action::Refresh)?;
        }

        // 2. Check status & wake if sleeping (or stopped with wake_on_demand).
        let mut woke = false;
        let mut status_ssh: Option<SshTarget> = None;
        if provider.has(Action::Status) {
            let output = run_action(Action::Status)?;
            let report = parse_status_report(&output.stdout).map_err(|err| ManageError::Step {
                machine: name.to_owned(),
                step: OpStep::Provider,
                output_tail: if matches!(err, IdentityError::KeyMaterial) {
                    String::new()
                } else {
                    tail(&output)
                },
                remedies: vec![Remedy::TryAgain, Remedy::FixProvider],
                source: SetupError::Identity(err),
            })?;
            status_ssh = report.ssh;
            if provider.has(Action::Wake) && should_wake(&report.state) {
                run_action(Action::Wake)?;
                woke = true;
            }
        }

        // 3. Refresh identity (and wake if identity itself reports sleeping).
        let link = if provider.has(Action::Identity) {
            let mut id = parse_id(&run_action(Action::Identity)?)?;
            if !woke
                && provider.has(Action::Wake)
                && id.state.as_ref().is_some_and(should_wake)
            {
                run_action(Action::Wake)?;
                id = parse_id(&run_action(Action::Identity)?)?;
            }
            if let Some(ref key) = id.ssh.host_key {
                known_hosts::replace(&self.known_hosts, &known_hosts::alias(&machine), key)?;
            }
            match provider
                .link(&machine, &params)
                .map_err(|source| ManageError::Params { source })?
            {
                Some(connect_link) => connect_link,
                None => Link::ssh(machine.clone(), id.ssh, &self.known_hosts),
            }
        } else if let Some(connect_link) = provider
            .link(&machine, &params)
            .map_err(|source| ManageError::Params { source })?
        {
            connect_link
        } else if let Some(ssh) = status_ssh {
            if let Some(ref key) = ssh.host_key {
                known_hosts::replace(&self.known_hosts, &known_hosts::alias(&machine), key)?;
            }
            Link::ssh(machine.clone(), ssh, &self.known_hosts)
        } else {
            need_link(self.link_if_any(&entry)?, &entry)?
        };

        // 4. Atomically update StoredLink in hosts.json.
        self.hosts.update_provider_link(name, link.stored())?;
        Ok(link)
    }

    /// Checks `request` without touching any machine: the host parses, the
    /// name is valid and free. The daemon calls it before it answers with a
    /// running operation, so these come back as the call's error.
    pub fn plan_add(&self, request: &AddSsh) -> Result<PlannedAdd, ManageError> {
        let target = parse_ssh_host(&request.ssh_host, &self.ssh_config)?;
        let name = if request.name.trim().is_empty() {
            name_from_host(&target.host)
        } else {
            request.name.trim().to_owned()
        };
        let machine = MachineName::new(&name).map_err(|source| ManageError::BadName {
            value: name.clone(),
            source,
        })?;
        self.name_free(machine.as_str())?;
        let root = match request.root.trim() {
            "" => DEFAULT_ROOT.to_owned(),
            root => root.to_owned(),
        };
        Ok(PlannedAdd { machine, target, root })
    }

    /// Adds the machine `plan` names: checks its host key against the one
    /// the user confirmed, then connect, check, install, start. Undone on
    /// failure. Returns the registered entry.
    pub fn add(&self, request: &AddSsh, plan: PlannedAdd, drive: &mut Drive<'_>) -> Result<HostEntry, ManageError> {
        let name = plan.machine.as_str().to_owned();
        let _busy = self.claim(&name)?;
        let entry = HostEntry {
            name: name.clone(),
            ssh_host: request.ssh_host.trim().to_owned(),
            root: plan.root.clone(),
            state: EntryState::Adding,
            os: String::new(),
            arch: String::new(),
            undo_pending: false,
            provider: None,
        };
        // Again under the claim: a removal of the name may have started, or
        // still be letting its sessions go, since the add was planned (D783).
        self.not_being_removed(&name)?;
        if !self.hosts.insert_new(entry.clone())? {
            return Err(ManageError::NameTaken { machine: name });
        }
        let link = Link::ssh(plan.machine.clone(), plan.target, &self.known_hosts);
        let key = Key::new();
        let result = self
            .confirm_host_key(&link, request.trust_fingerprint.as_deref(), drive)
            .and_then(|()| stop_if_cancelled(link.machine(), drive))
            .and_then(|()| self.set_up(&link, Some(&key), drive));
        match result {
            Ok(ready) => {
                (drive.events)(StepEvent::Skipped(OpStep::FindAgents));
                let entry = registered(entry, &ready);
                self.hosts.put(entry.clone())?;
                Ok(entry)
            }
            Err(error) => {
                // What was done is undone; the add's own error is what the
                // user needs, so an undo that has to wait is only recorded.
                let _ = self.undo_add(link.machine(), Some(&link), drive);
                Err(error)
            }
        }
    }

    /// Checks `request` for a machine the provider will make, without
    /// running anything: the provider runs on this Mac, the name is valid
    /// and free, and the params are exactly the declared ones (a missing
    /// one without a default, or an undeclared one, is refused naming it).
    pub fn plan_create(
        &self,
        provider: &Provider,
        name: &str,
        values: BTreeMap<String, String>,
    ) -> Result<PlannedCreate, ManageError> {
        let machine = MachineName::new(name.trim()).map_err(|source| ManageError::BadName {
            value: name.trim().to_owned(),
            source,
        })?;
        if provider.run_on().is_some() {
            return Err(ManageError::RunOnUnsupported {
                provider: provider.id().to_owned(),
            });
        }
        provider
            .environment(&machine, &values)
            .map_err(|source| ManageError::Params { source })?;
        self.name_free(machine.as_str())?;
        Ok(PlannedCreate {
            machine,
            provider: provider.clone(),
            values,
        })
    }

    /// Create machine through a provider (MP-3, MP-19): its `create`, then
    /// `identity` or `connect`, then the same check, install and start as
    /// an SSH add, then the machine is listed with its provider. The entry
    /// is written `Adding` first, and its link as soon as it is known, so
    /// a restart at any step undoes it. A failure undoes what the setup
    /// did on the machine and leaves what `create` made, for the
    /// provider's `delete` to remove ([`Machines::delete_made`]).
    pub fn create(&self, plan: PlannedCreate, drive: &mut Drive<'_>) -> Result<HostEntry, ManageError> {
        let name = plan.machine.as_str().to_owned();
        let _busy = self.claim(&name)?;
        let key = Key::new();
        let setup = Setup::new(
            plan.provider.clone(),
            plan.machine.clone(),
            plan.values.clone(),
            &self.known_hosts,
        )
        .map_err(|source| ManageError::Params { source })?
        .with_retry(self.retry)
        .with_enrollment(key.clone());
        let entry = HostEntry {
            name: name.clone(),
            ssh_host: format!("via {}", plan.provider.id()),
            root: DEFAULT_ROOT.to_owned(),
            state: EntryState::Adding,
            os: String::new(),
            arch: String::new(),
            undo_pending: false,
            provider: Some(ProviderMachine {
                id: plan.provider.id().to_owned(),
                params: plan.values.clone(),
                link: None,
            }),
        };
        // Again under the claim: a removal of the name may have started, or
        // still be letting its sessions go, since the add was planned (D783).
        self.not_being_removed(&name)?;
        if !self.hosts.insert_new(entry.clone())? {
            return Err(ManageError::NameTaken { machine: name });
        }
        match self.run_setup(setup, Some(&key), drive, true) {
            Ok(ready) => {
                (drive.events)(StepEvent::Skipped(OpStep::FindAgents));
                let mut entry = registered(entry, &ready);
                if let Some(provider) = &mut entry.provider {
                    provider.link = Some(ready.link.stored());
                }
                self.hosts.put(entry.clone())?;
                Ok(entry)
            }
            Err(error) => {
                let error = match error {
                    ManageError::LoginNeeded { machine, output_tail, reason } => {
                        ManageError::ProviderLoginNeeded { machine, output_tail, reason }
                    }
                    other => other,
                };
                // Failing before the machine answered means humatad2 never
                // got there, so there is nothing to uninstall.
                let reached = !matches!(
                    error,
                    ManageError::ProviderLoginNeeded { .. }
                        | ManageError::Step {
                            step: OpStep::Create | OpStep::Identity | OpStep::Connect,
                            ..
                        }
                );
                let link = self
                    .hosts
                    .get(&name)
                    .ok()
                    .flatten()
                    .filter(|_| reached)
                    .and_then(|entry| self.link_if_any(&entry).ok().flatten());
                let _ = self.undo_add(&plan.machine, link.as_ref(), drive);
                Err(error)
            }
        }
    }

    /// Runs the provider's `action` for the machine it made, with the
    /// params it was made with (MP-21): start, stop or status. Returns what
    /// the command printed.
    pub fn run_action(
        &self,
        provider: &Provider,
        machine: &str,
        action: Action,
        drive: &mut Drive<'_>,
    ) -> Result<Output, ManageError> {
        let _busy = self.claim(machine)?;
        let entry = self.get(machine)?;
        let record = provider_record(&entry, provider)?;
        if entry.state != EntryState::Ready {
            return Err(ManageError::NotReady {
                machine: machine.to_owned(),
            });
        }
        self.run_provider_command(provider, machine, &record.params, action, drive)
    }

    /// Delete on a provider's machine (MP-21): removes the machine as
    /// Remove machine does, then runs the provider's `delete`. Refused
    /// before anything is removed when the provider has no `delete`.
    pub fn delete_machine(&self, provider: &Provider, machine: &str, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let _busy = self.claim(machine)?;
        let entry = self.get(machine)?;
        let params = provider_record(&entry, provider)?.params.clone();
        if !provider.has(Action::Delete) {
            return Err(ManageError::ActionUndefined {
                provider: provider.id().to_owned(),
                action: Action::Delete,
            });
        }
        self.remove_claimed(machine, entry, drive)?;
        self.run_provider_command(provider, machine, &params, Action::Delete, drive)
            .map(|_| ())
            .map_err(|error| match error {
                ManageError::ActionFailed {
                    machine,
                    exit,
                    output_tail,
                    ..
                } => ManageError::DeleteFailed {
                    machine,
                    exit,
                    output_tail,
                },
                other => other,
            })
    }

    /// The provider's `delete` for what a failed Create machine made
    /// (MP-20), run with the params the create was given. The machine is
    /// not listed, so nothing else is touched.
    pub fn delete_made(
        &self,
        provider: &Provider,
        machine: &str,
        values: &BTreeMap<String, String>,
        drive: &mut Drive<'_>,
    ) -> Result<(), ManageError> {
        self.run_provider_command(provider, machine, values, Action::Delete, drive)
            .map(|_| ())
    }

    fn run_provider_command(
        &self,
        provider: &Provider,
        machine: &str,
        values: &BTreeMap<String, String>,
        action: Action,
        drive: &mut Drive<'_>,
    ) -> Result<Output, ManageError> {
        let name = MachineName::new(machine).map_err(|source| ManageError::BadName {
            value: machine.to_owned(),
            source,
        })?;
        let invocation = provider
            .invocation(action, &name, values)
            .map_err(|source| ManageError::Params { source })?
            .ok_or_else(|| ManageError::ActionUndefined {
                provider: provider.id().to_owned(),
                action,
            })?;
        (drive.events)(StepEvent::Running(OpStep::Provider));
        let output = drive.runner.run(&invocation).map_err(|source| {
            (drive.events)(StepEvent::Failed {
                step: OpStep::Provider,
                output_tail: String::new(),
            });
            ManageError::Run {
                machine: machine.to_owned(),
                source,
            }
        })?;
        if output.exit.success() {
            (drive.events)(StepEvent::Done(OpStep::Provider));
            return Ok(output);
        }
        let output_tail = tail(&output);
        (drive.events)(StepEvent::Failed {
            step: OpStep::Provider,
            output_tail: output_tail.clone(),
        });
        Err(ManageError::ActionFailed {
            machine: machine.to_owned(),
            action,
            exit: output.exit,
            output_tail,
        })
    }

    /// Where `hosts.json` is, for the files kept beside it.
    pub fn hosts_file(&self) -> &Path {
        self.hosts.path()
    }

    /// The machine list, for the interactive login's own registering.
    pub(crate) fn hosts(&self) -> &Hosts {
        &self.hosts
    }

    /// Humata's own known_hosts file (MP-9).
    pub(crate) fn known_hosts_file(&self) -> &Path {
        &self.known_hosts
    }

    /// Reconnect now (HO-6): connect, check, install what differs (TS-1),
    /// start. For an entry whose add is still to be undone, runs the undo
    /// instead and reports [`ManageError::NotReady`].
    pub fn reconnect(&self, machine: &str, drive: &mut Drive<'_>) -> Result<HostEntry, ManageError> {
        let _busy = self.claim(machine)?;
        let entry = self.get(machine)?;
        let link = self.link_if_any(&entry)?;
        match entry.state {
            EntryState::Ready => {}
            EntryState::Adding => {
                self.undo_add(&machine_name(&entry)?, link.as_ref(), drive)?;
                return Err(ManageError::NotReady {
                    machine: machine.to_owned(),
                });
            }
            EntryState::Removing => {
                self.finish_remove(&need_link(link, &entry)?, drive)?;
                return Err(ManageError::NotFound {
                    machine: machine.to_owned(),
                });
            }
        }
        let link = need_link(link, &entry)?;
        // A machine added before the host kept keys gets one now; one whose
        // host copy exists restores it if missing on an ephemeral machine.
        let (new_key, restore_key) = match enroll::read_host(&self.peer_keys, machine) {
            Ok(Some(existing)) => (None, Some(existing)),
            Ok(None) => (Some(Key::new()), None),
            Err(_) => (None, None),
        };
        let ready = self.set_up_with_restore(&link, new_key.as_ref(), restore_key, drive)?;
        let entry = registered(entry, &ready);
        self.hosts.put(entry.clone())?;
        Ok(entry)
    }

    /// Remove machine (LD-10) once the daemon checked the plan and the
    /// user's presence: removes humatad2 and its service from the machine,
    /// forgets the host key, drops the entry.
    pub fn remove(&self, machine: &str, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let result = self.remove_holding_name(machine, drive);
        self.removal_done(machine);
        result
    }

    /// As [`Self::remove`], but a removal that goes through keeps the name
    /// being removed until [`Self::removal_done`], so what the host still
    /// does for the machine it removed (D783: archiving its sessions,
    /// withdrawing its requests) ends before an add of the name can start.
    /// One that fails lets the name go at once: it is still `Removing` in
    /// hosts.json when it couldn't reach the machine, which `name_free` reads.
    pub fn remove_holding_name(
        &self,
        machine: &str,
        drive: &mut Drive<'_>,
    ) -> Result<(), ManageError> {
        let result = self.claim(machine).and_then(|_busy| {
            let entry = self.get(machine)?;
            self.remove_claimed(machine, entry, drive)
        });
        if result.is_err() {
            self.removal_done(machine);
        }
        result
    }

    /// The removal of `machine` is over: an add of its name goes ahead.
    pub fn removal_done(&self, machine: &str) {
        lock_set(&self.removing).remove(machine);
    }

    /// Notes that removing `machine` was asked for and [`Self::remove`] will
    /// run, so an add of its name until then is told it is being removed.
    pub fn removing(&self, machine: &str) {
        lock_set(&self.removing).insert(machine.to_owned());
    }

    /// `name` can be added: no machine has it and no removal holds it. A
    /// removal holds the name after its entry has left hosts.json, until
    /// [`Self::removal_done`] (D783).
    fn name_free(&self, name: &str) -> Result<(), ManageError> {
        self.not_being_removed(name)?;
        let Some(entry) = self.hosts.get(name)? else {
            return Ok(());
        };
        if entry.state == EntryState::Removing {
            return Err(ManageError::BeingRemoved {
                machine: name.to_owned(),
            });
        }
        Err(ManageError::NameTaken {
            machine: name.to_owned(),
        })
    }

    /// Refuses `name` while a removal of it is under way (D760, D783).
    fn not_being_removed(&self, name: &str) -> Result<(), ManageError> {
        if lock_set(&self.removing).contains(name) {
            return Err(ManageError::BeingRemoved {
                machine: name.to_owned(),
            });
        }
        Ok(())
    }

    /// LD-12: forgets a machine that is lost, on this host only, without
    /// reaching it: its peer key, its entry and its pinned host key go, in
    /// the order a removal forgets them. Humata stays on the machine; adding
    /// it again makes a new machine.
    pub fn forget(&self, machine: &str) -> Result<(), ManageError> {
        let _busy = self.claim(machine)?;
        let entry = self.get(machine)?;
        let name = machine_name(&entry)?;
        self.forget_peer_key(machine)?;
        self.hosts.remove(machine)?;
        known_hosts::unpin(&self.known_hosts, &name)?;
        Ok(())
    }

    fn remove_claimed(&self, machine: &str, mut entry: HostEntry, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let link = self.link_if_any(&entry)?;
        if entry.state == EntryState::Adding {
            return self.undo_add(&machine_name(&entry)?, link.as_ref(), drive);
        }
        let link = need_link(link, &entry)?;
        debug_assert_eq!(link.machine().as_str(), machine);
        entry.state = EntryState::Removing;
        self.hosts.put(entry)?;
        self.finish_remove(&link, drive)
    }

    /// At start: undoes every add and finishes every removal a restart cut
    /// short (PER-27). Entries whose machine can't be reached are kept for
    /// the next try and reported.
    pub fn recover(&self, drive: &mut Drive<'_>) -> Result<Vec<Recovered>, ManageError> {
        let mut recovered = Vec::new();
        for entry in self.hosts.list()? {
            if entry.state == EntryState::Ready {
                continue;
            }
            let outcome = self.claim(&entry.name).and_then(|_busy| {
                let link = self.link_if_any(&entry)?;
                match entry.state {
                    EntryState::Adding => self.undo_add(&machine_name(&entry)?, link.as_ref(), drive),
                    EntryState::Removing => self.finish_remove(&need_link(link, &entry)?, drive),
                    EntryState::Ready => Ok(()),
                }
            });
            recovered.push(Recovered {
                machine: entry.name.clone(),
                found: entry.state,
                outcome,
            });
        }
        Ok(recovered)
    }

    pub(crate) fn claim(&self, machine: &str) -> Result<BusyGuard<'_>, ManageError> {
        let mut busy = lock_set(&self.busy);
        if !busy.insert(machine.to_owned()) {
            return Err(ManageError::Busy {
                machine: machine.to_owned(),
            });
        }
        Ok(BusyGuard {
            busy: &self.busy,
            machine: machine.to_owned(),
        })
    }

    /// Where the host keeps its copy of `machine`'s enrollment key; there
    /// is none until an add or reconnect set the machine up.
    pub fn peer_key(&self, machine: &str) -> PathBuf {
        enroll::host_path(&self.peer_keys, machine)
    }

    fn forget_peer_key(&self, machine: &str) -> Result<(), ManageError> {
        enroll::remove_host(&self.peer_keys, machine).map_err(|source| ManageError::DeletePeerKey {
            machine: machine.to_owned(),
            path: self.peer_key(machine),
            source,
        })
    }

    /// The command that opens `machine`'s peer link.
    pub fn peer_link(&self, machine: &str) -> Result<Invocation, ManageError> {
        let entry = self.get(machine)?;
        let link = need_link(self.link_if_any(&entry)?, &entry)?;
        link.peer().map_err(|source| ManageError::Quote {
            machine: machine.to_owned(),
            source,
        })
    }

    /// Puts `config`, the host's stack settings, on `machine` unless its
    /// copy is already that (D411), over the same SSH path that installs the
    /// tools. For a link that has just come up; setup does the same when it
    /// installs. Blocks while the machine answers.
    pub fn push_stack_config(
        &self,
        machine: &str,
        config: &StackConfig,
        runner: &dyn Runner,
    ) -> Result<(), StackConfigError> {
        let on = |source: ManageError| StackConfigError::Machine {
            machine: machine.to_owned(),
            source: Box::new(source),
        };
        let entry = self.get(machine).map_err(on)?;
        let link = need_link(self.link_if_any(&entry).map_err(on)?, &entry).map_err(on)?;
        stack_config::push(&link, runner, config)
    }

    /// The one-off command that runs `humatad2 stop` on `machine` (STOP,
    /// D262), for a stop the peer link can't carry.
    pub fn stop_sessions_command(
        &self,
        machine: &str,
        stop_id: &str,
        by: &str,
        reason: &str,
    ) -> Result<Invocation, ManageError> {
        let entry = self.get(machine)?;
        let link = need_link(self.link_if_any(&entry)?, &entry)?;
        link.stop_sessions(stop_id, by, reason).map_err(|source| ManageError::Quote {
            machine: machine.to_owned(),
            source,
        })
    }

    /// The command that runs a machine script on `machine` (MACHINESCRIPTS):
    /// a bash script streamed on stdin to `bash -l -s -- args`, or the
    /// machine's own `humata` with the arguments. `env` is exported first;
    /// its names must look like `[A-Z_][A-Z0-9_]*`. The command is killed
    /// after `timeout`.
    pub fn machine_script(
        &self,
        machine: &str,
        body: &ScriptBody,
        args: &[String],
        env: &[(String, String)],
        timeout: Duration,
    ) -> Result<Invocation, ManageError> {
        let entry = self.get(machine)?;
        let link = need_link(self.link_if_any(&entry)?, &entry)?;
        let quoted = |source| ManageError::Quote {
            machine: machine.to_owned(),
            source,
        };
        let mut script = String::new();
        if !env.is_empty() {
            script.push_str("export");
            for (name, value) in env {
                if !is_env_name(name) {
                    return Err(ManageError::BadEnvName { name: name.clone() });
                }
                script.push(' ');
                script.push_str(name);
                script.push('=');
                script.push_str(&quote::quote("a variable value", value).map_err(quoted)?);
            }
            script.push_str("; ");
        }
        let stdin = match body {
            ScriptBody::Bash(bytes) => {
                script.push_str("exec bash -l -s --");
                for arg in args {
                    script.push(' ');
                    script.push_str(&quote::quote("a script argument", arg).map_err(quoted)?);
                }
                Some(Arc::clone(bytes))
            }
            ScriptBody::Humata(words) => {
                script.push_str(&format!("exec \"$HOME/{BIN_DIR}/humata\""));
                for word in words.iter().chain(args) {
                    script.push(' ');
                    script.push_str(&quote::quote("a humata argument", word).map_err(quoted)?);
                }
                None
            }
        };
        link.script(&script, Purpose::MachineScript, Some(timeout), stdin)
            .map_err(quoted)
    }

    /// The link to `entry`'s machine: from the provider's record, or the
    /// SSH host typed in. None for a provider machine whose create was
    /// cut short before it had one.
    pub(crate) fn link_if_any(&self, entry: &HostEntry) -> Result<Option<Link>, ManageError> {
        let machine = machine_name(entry)?;
        if let Some(provider) = &entry.provider {
            return Ok(provider
                .link
                .as_ref()
                .map(|stored| Link::from_stored(machine, stored, &self.known_hosts)));
        }
        self.ssh_link(entry, machine).map(Some)
    }

    fn ssh_link(&self, entry: &HostEntry, machine: MachineName) -> Result<Link, ManageError> {
        let target = parse_ssh_host(&entry.ssh_host, &self.ssh_config)?;
        Ok(Link::ssh(machine, target, &self.known_hosts))
    }

    /// Pins the key the machine presents when it is the one the user
    /// confirmed; otherwise fails with the fingerprint to show.
    fn confirm_host_key(&self, link: &Link, confirmed: Option<&str>, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let machine = link.machine().as_str().to_owned();
        (drive.events)(StepEvent::Running(OpStep::Connect));
        let setup = Setup::existing(link.clone(), &self.known_hosts);
        let presented = setup.scan_host_key(drive.runner).map_err(|failure| {
            (drive.events)(StepEvent::Failed {
                step: OpStep::Connect,
                output_tail: failure.output_tail.clone(),
            });
            ManageError::Step {
                machine: machine.clone(),
                step: OpStep::Connect,
                output_tail: failure.output_tail,
                remedies: failure.remedies,
                source: failure.error,
            }
        })?;
        let fail = |drive: &mut Drive<'_>, error: ManageError| {
            (drive.events)(StepEvent::Failed {
                step: OpStep::Connect,
                output_tail: String::new(),
            });
            Err(error)
        };
        match confirmed.map(str::trim) {
            None | Some("") => fail(
                drive,
                ManageError::HostKeyUnconfirmed {
                    machine,
                    algorithm: presented.algorithm().to_owned(),
                    fingerprint: presented.fingerprint().to_owned(),
                },
            ),
            Some(expected) if expected != presented.fingerprint() => fail(
                drive,
                ManageError::HostKeyMismatch {
                    machine,
                    expected: expected.to_owned(),
                    presented: presented.fingerprint().to_owned(),
                },
            ),
            Some(_) => {
                known_hosts::pin(&self.known_hosts, link.machine(), &presented)?;
                Ok(())
            }
        }
    }

    /// Runs connect through service, pushing each step's progress.
    /// With `key`, the install step writes it on the machine and, once the
    /// rest is set up, the host keeps its copy.
    fn set_up(&self, link: &Link, key: Option<&Key>, drive: &mut Drive<'_>) -> Result<setup::Ready, ManageError> {
        self.set_up_with_restore(link, key, None, drive)
    }

    fn set_up_with_restore(
        &self,
        link: &Link,
        key: Option<&Key>,
        restore: Option<Key>,
        drive: &mut Drive<'_>,
    ) -> Result<setup::Ready, ManageError> {
        let mut setup = Setup::existing(link.clone(), &self.known_hosts).with_retry(self.retry);
        if let Some(key) = key {
            setup = setup.with_enrollment(key.clone());
        } else if let Some(restore) = restore {
            setup = setup.with_restore_key(restore);
        }
        self.run_setup(setup, key, drive, false)
    }

    /// Drives `setup` to its end. With `record`, the link is written to the
    /// machine's entry as soon as a step makes one (a provider's create),
    /// so a restart can undo what reached the machine.
    fn run_setup(
        &self,
        mut setup: Setup,
        key: Option<&Key>,
        drive: &mut Drive<'_>,
        record: bool,
    ) -> Result<setup::Ready, ManageError> {
        let name = setup.machine().clone();
        let machine = name.as_str().to_owned();
        let mut recorded = false;
        let mut current = OpStep::of(setup.current());
        (drive.events)(StepEvent::Running(current));
        loop {
            let progress = setup.step(drive.runner, drive.builds);
            match progress {
                Progress::Continue => {
                    if record && !recorded {
                        if let Some(link) = setup.link() {
                            self.record_link(&machine, link)?;
                            recorded = true;
                        }
                    }
                    let next = OpStep::of(setup.current());
                    if next != current {
                        push_checked(setup.facts(), current, drive);
                        (drive.events)(StepEvent::Done(current));
                        stop_if_cancelled(&name, drive)?;
                        current = next;
                        (drive.events)(StepEvent::Running(current));
                    }
                }
                Progress::RetryAfter { delay, attempt, of } => {
                    (drive.events)(StepEvent::Retrying {
                        step: current,
                        attempt,
                        of,
                        delay,
                    });
                    (drive.wait)(delay);
                    stop_if_cancelled(&name, drive)?;
                }
                Progress::NeedsLogin(login) => {
                    (drive.events)(StepEvent::Failed {
                        step: current,
                        output_tail: login.output_tail.clone(),
                    });
                    return Err(ManageError::LoginNeeded {
                        machine,
                        output_tail: login.output_tail,
                        reason: login.reason,
                    });
                }
                Progress::Ready(ready) => {
                    if let Some(key) = key {
                        enroll::write_host(&self.peer_keys, &name, key).map_err(|source| {
                            (drive.events)(StepEvent::Failed {
                                step: current,
                                output_tail: String::new(),
                            });
                            ManageError::SavePeerKey {
                                machine: machine.clone(),
                                path: self.peer_key(&machine),
                                source,
                            }
                        })?;
                    }
                    push_checked(Some(&ready.facts), current, drive);
                    (drive.events)(StepEvent::Done(current));
                    return Ok(ready);
                }
                Progress::Failed(failure) => {
                    let step = OpStep::of(failure.step);
                    (drive.events)(StepEvent::Failed {
                        step,
                        output_tail: failure.output_tail.clone(),
                    });
                    return Err(ManageError::Step {
                        machine,
                        step,
                        output_tail: failure.output_tail,
                        remedies: failure.remedies,
                        source: failure.error,
                    });
                }
            }
        }
    }

    /// Writes `link` in `machine`'s entry (a provider machine being made).
    fn record_link(&self, machine: &str, link: &Link) -> Result<(), ManageError> {
        if let Some(mut entry) = self.hosts.get(machine)? {
            if let Some(provider) = &mut entry.provider {
                provider.link = Some(link.stored());
                self.hosts.put(entry)?;
            }
        }
        Ok(())
    }

    /// Undoes an add: without a pinned key nothing reached the machine, so
    /// only the entry goes; otherwise humatad2 is removed first, and when
    /// that can't be done the entry stays with `undo_pending`.
    fn undo_add(&self, machine: &MachineName, link: Option<&Link>, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let pinned = known_hosts::pinned(&self.known_hosts, machine)?.is_some();
        // A provider's `connect` pins nothing, so having a link is what
        // says humatad2 may have reached the machine.
        if let Some(link) = link.filter(|link| pinned || link.is_connect()) {
            if let Err(error) = self.uninstall(link, drive) {
                if let Some(mut entry) = self.hosts.get(machine.as_str())? {
                    entry.undo_pending = true;
                    self.hosts.put(entry)?;
                }
                return Err(error);
            }
            (drive.events)(StepEvent::Undone(OpStep::Install));
        }
        known_hosts::unpin(&self.known_hosts, machine)?;
        self.forget_peer_key(machine.as_str())?;
        self.hosts.remove(machine.as_str())?;
        Ok(())
    }

    fn finish_remove(&self, link: &Link, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let machine = link.machine();
        if link.is_connect() || known_hosts::pinned(&self.known_hosts, machine)?.is_some() {
            self.uninstall(link, drive)?;
        } else {
            // The key is forgotten last, so without one the machine was
            // already cleaned up.
            (drive.events)(StepEvent::Skipped(OpStep::Uninstall));
        }
        (drive.events)(StepEvent::Running(OpStep::Forget));
        // The key goes first: a removal cut short after it finishes at the
        // next recover all the same.
        self.forget_peer_key(machine.as_str())?;
        // The entry goes before the key: an entry without a key is known
        // clean (above), a key without an entry would be a stray pin.
        self.hosts.remove(machine.as_str())?;
        known_hosts::unpin(&self.known_hosts, machine)?;
        (drive.events)(StepEvent::Done(OpStep::Forget));
        Ok(())
    }

    fn uninstall(&self, link: &Link, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let machine = link.machine().as_str().to_owned();
        (drive.events)(StepEvent::Running(OpStep::Uninstall));
        let invocation = link.remove_daemon().map_err(|source| ManageError::Quote {
            machine: machine.clone(),
            source,
        })?;
        let output = drive.runner.run(&invocation).map_err(|source| ManageError::Run {
            machine: machine.clone(),
            source,
        })?;
        if output.exit.success() {
            (drive.events)(StepEvent::Done(OpStep::Uninstall));
            return Ok(());
        }
        let output_tail = tail(&output);
        (drive.events)(StepEvent::Failed {
            step: OpStep::Uninstall,
            output_tail: output_tail.clone(),
        });
        Err(ManageError::UninstallFailed {
            machine,
            exit: output.exit,
            output_tail,
        })
    }
}

fn machine_name(entry: &HostEntry) -> Result<MachineName, ManageError> {
    MachineName::new(&entry.name).map_err(|source| ManageError::BadName {
        value: entry.name.clone(),
        source,
    })
}

/// Whether `name` is a variable name a script may get: `[A-Z_][A-Z0-9_]*`.
fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_uppercase() || first == '_')
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn need_link(link: Option<Link>, entry: &HostEntry) -> Result<Link, ManageError> {
    link.ok_or_else(|| ManageError::ProviderLinkLost {
        machine: entry.name.clone(),
    })
}

/// `entry`'s provider record, when `provider` is the one that made it.
fn provider_record<'a>(entry: &'a HostEntry, provider: &Provider) -> Result<&'a ProviderMachine, ManageError> {
    entry
        .provider
        .as_ref()
        .filter(|record| record.id == provider.id())
        .ok_or_else(|| ManageError::NotProviderMachine {
            machine: entry.name.clone(),
            provider: provider.id().to_owned(),
        })
}

fn tail(output: &Output) -> String {
    output.tail(TAIL_LINES)
}

fn registered(mut entry: HostEntry, ready: &setup::Ready) -> HostEntry {
    entry.state = EntryState::Ready;
    entry.undo_pending = false;
    entry.os = ready.facts.platform.os.to_string();
    entry.arch = ready.facts.platform.arch.to_string();
    entry
}

/// `user@host`, `user@host:port`, `user@[v6]:port` reach the host directly;
/// anything else is an alias the ssh config resolves.
fn parse_ssh_host(value: &str, ssh_config: &Path) -> Result<SshTarget, ManageError> {
    let value = value.trim();
    let invalid = |reason| ManageError::InvalidSshHost {
        value: value.to_owned(),
        reason,
    };
    if value.is_empty() {
        return Err(invalid("it is empty"));
    }
    if value.starts_with('-') {
        return Err(invalid("it starts with `-`"));
    }
    let Some((user, rest)) = value.split_once('@') else {
        if !value.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
            return Err(invalid(
                "an alias has only letters, digits, `.`, `_` and `-`; a port needs user@host:port",
            ));
        }
        let config = if ssh_config.is_file() {
            ssh_config.display().to_string()
        } else {
            "/dev/null".to_owned()
        };
        return Ok(SshTarget {
            host: value.to_owned(),
            access: SshAccess::Config { ssh_config: config },
            host_key: None,
        });
    };
    if user.is_empty() || !user.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return Err(invalid("the user name has only letters, digits, `.`, `_` and `-`"));
    }
    let (host, port) = if let Some(bracketed) = rest.strip_prefix('[') {
        let Some((host, after)) = bracketed.split_once(']') else {
            return Err(invalid("an IPv6 address needs its closing `]`"));
        };
        match after {
            "" => (host, None),
            _ => match after.strip_prefix(':') {
                Some(port) => (host, Some(port)),
                None => return Err(invalid("only `:port` may follow `]`")),
            },
        }
    } else {
        match rest.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, Some(port)),
            Some(_) => return Err(invalid("write an IPv6 address in brackets: user@[address]:port")),
            None => (rest, None),
        }
    };
    if host.is_empty()
        || host.starts_with('-')
        || !host.chars().all(|c| c.is_ascii_alphanumeric() || ".-_:".contains(c))
    {
        return Err(invalid("the host has only letters, digits, `.`, `-` and `_`"));
    }
    let port = match port {
        None => 22,
        Some(port) => match port.parse::<u16>() {
            Ok(port) if port > 0 => port,
            _ => return Err(invalid("the port is a number from 1 to 65535")),
        },
    };
    Ok(SshTarget {
        host: host.to_owned(),
        access: SshAccess::Direct {
            port,
            user: user.to_owned(),
            identity_file: None,
        },
        host_key: None,
    })
}

/// `devbox-1.internal` → `devbox-1`; anything else is lowercased and every
/// character a name can't hold becomes `-`.
fn name_from_host(host: &str) -> String {
    let first = match host.parse::<std::net::IpAddr>() {
        Ok(_) => host,
        Err(_) => host.split('.').next().unwrap_or(host),
    };
    let name: String = first
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c if c.is_ascii_lowercase() || c.is_ascii_digit() => c,
            _ => '-',
        })
        .take(63)
        .collect();
    name.trim_matches('-').to_owned()
}

fn lock_set(set: &Mutex<BTreeSet<String>>) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
    set.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct(host: &str, port: u16, user: &str) -> SshTarget {
        SshTarget {
            host: host.to_owned(),
            access: SshAccess::Direct {
                port,
                user: user.to_owned(),
                identity_file: None,
            },
            host_key: None,
        }
    }

    #[test]
    fn hosts_typed_in_reach_the_machine_directly_or_through_the_config() {
        let folder = tempfile::tempdir().unwrap();
        let config = folder.path().join("config");
        let none = parse_ssh_host("devbox", &config).unwrap();
        assert_eq!(
            none.access,
            SshAccess::Config {
                ssh_config: "/dev/null".to_owned()
            }
        );
        std::fs::write(&config, "Host devbox\n").unwrap();
        let alias = parse_ssh_host(" devbox ", &config).unwrap();
        assert_eq!(alias.host, "devbox");
        assert_eq!(
            alias.access,
            SshAccess::Config {
                ssh_config: config.display().to_string()
            }
        );

        assert_eq!(parse_ssh_host("me@box.internal", &config).unwrap(), direct("box.internal", 22, "me"));
        assert_eq!(parse_ssh_host("me@10.0.0.2:2222", &config).unwrap(), direct("10.0.0.2", 2222, "me"));
        assert_eq!(parse_ssh_host("me@[fd00::2]:2200", &config).unwrap(), direct("fd00::2", 2200, "me"));
    }

    #[test]
    fn hosts_ssh_would_misread_are_refused() {
        let config = Path::new("/nonexistent");
        for value in [
            "",
            "-oProxyCommand=x",
            "me@-oProxyCommand=x",
            "box:22",
            "me@box:0",
            "me@box:http",
            "@box",
            "me@fd00::2",
            "me@box; rm -rf",
            "me@[fd00::2",
        ] {
            assert!(
                matches!(parse_ssh_host(value, config), Err(ManageError::InvalidSshHost { .. })),
                "{value} was accepted"
            );
        }
    }

    #[test]
    fn a_name_is_made_from_the_host_when_none_is_given() {
        assert_eq!(name_from_host("devbox-1.internal"), "devbox-1");
        assert_eq!(name_from_host("Build_Box"), "build-box");
        assert_eq!(name_from_host("10.0.0.2"), "10-0-0-2");
    }
}
