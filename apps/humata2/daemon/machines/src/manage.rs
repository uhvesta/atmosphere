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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use thiserror::Error;

use crate::enroll::{self, Key};
use crate::error::{KnownHostsError, NameError, QuoteError, Remedy, RunError, SetupError};
use crate::hosts::{EntryState, HostEntry, Hosts, HostsError};
use crate::identity::{SshAccess, SshTarget};
use crate::install::{Builds, InstalledFile};
use crate::known_hosts;
use crate::link::Link;
use crate::name::MachineName;
use crate::runner::{Exit, Invocation, Output, Runner};
use crate::setup::{self, Progress, RetryPolicy, Setup};

/// How many output lines a failed step keeps (MP-20).
const TAIL_LINES: usize = 40;
/// Where channels live on a machine unless the user says otherwise (FC-10).
pub const DEFAULT_ROOT: &str = "~/Humata";

/// The steps an operation shows (GRPC.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OpStep {
    Connect,
    Check,
    Install,
    Start,
    FindAgents,
    /// Removing humatad2 from the machine (remove, or undoing an add).
    Uninstall,
    /// Forgetting the host key and dropping the entry.
    Forget,
}

impl OpStep {
    /// The steps of Add machine, in order.
    pub const ADD: [Self; 5] = [Self::Connect, Self::Check, Self::Install, Self::Start, Self::FindAgents];
    /// The steps of Reconnect.
    pub const RECONNECT: [Self; 4] = [Self::Connect, Self::Check, Self::Install, Self::Start];
    /// The steps of Remove machine.
    pub const REMOVE: [Self; 2] = [Self::Uninstall, Self::Forget];

    /// Stable, as `OperationStep.key`.
    pub fn key(self) -> &'static str {
        match self {
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
            setup::Step::Create | setup::Step::Identity | setup::Step::Connect => Self::Connect,
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
    Skipped(OpStep),
    /// With the step's kept output (MP-20).
    Failed { step: OpStep, output_tail: String },
    /// Undone after a later step failed.
    Undone(OpStep),
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
fn stop_if_cancelled(link: &Link, drive: &Drive<'_>) -> Result<(), ManageError> {
    if (drive.cancelled)() {
        return Err(ManageError::Cancelled {
            machine: link.machine().as_str().to_owned(),
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
        "{machine} presents host key {fingerprint}, which Humata hasn't seen before. Compare it with the machine's own (`ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` on it), then add the machine again with this fingerprint to trust it"
    )]
    HostKeyUnconfirmed { machine: String, fingerprint: String },
    #[error(
        "{machine} presents host key {presented}, not the {expected} you confirmed, so nothing was sent to it. Someone may be intercepting the connection: check the fingerprint on the machine itself before trusting a different key"
    )]
    HostKeyMismatch {
        machine: String,
        expected: String,
        presented: String,
    },
    #[error("{step:?} failed on {machine}: {source}")]
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
    LoginNeeded { machine: String, output_tail: String },
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
    #[error(transparent)]
    Hosts(#[from] HostsError),
    #[error(transparent)]
    KnownHosts(#[from] KnownHostsError),
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
}

/// Marks a machine busy until dropped.
struct BusyGuard<'a> {
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
        }
    }

    pub fn list(&self) -> Result<Vec<HostEntry>, ManageError> {
        Ok(self.hosts.list()?)
    }

    pub fn get(&self, machine: &str) -> Result<HostEntry, ManageError> {
        self.hosts.get(machine)?.ok_or_else(|| ManageError::NotFound {
            machine: machine.to_owned(),
        })
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
        if self.hosts.get(machine.as_str())?.is_some() {
            return Err(ManageError::NameTaken { machine: name });
        }
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
            staged: Vec::new(),
            undo_pending: false,
        };
        if !self.hosts.insert_new(entry.clone())? {
            return Err(ManageError::NameTaken { machine: name });
        }
        let link = Link::ssh(plan.machine.clone(), plan.target, &self.known_hosts);
        let key = Key::new();
        let result = self
            .confirm_host_key(&link, request.trust_fingerprint.as_deref(), drive)
            .and_then(|()| stop_if_cancelled(&link, drive))
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
                let _ = self.undo_add(&link, drive);
                Err(error)
            }
        }
    }

    /// Reconnect now (HO-6): connect, check, install what differs (TS-1),
    /// start. For an entry whose add is still to be undone, runs the undo
    /// instead and reports [`ManageError::NotReady`].
    pub fn reconnect(&self, machine: &str, drive: &mut Drive<'_>) -> Result<HostEntry, ManageError> {
        let _busy = self.claim(machine)?;
        let entry = self.get(machine)?;
        let link = self.link(&entry)?;
        match entry.state {
            EntryState::Ready => {}
            EntryState::Adding => {
                self.undo_add(&link, drive)?;
                return Err(ManageError::NotReady {
                    machine: machine.to_owned(),
                });
            }
            EntryState::Removing => {
                self.finish_remove(&link, drive)?;
                return Err(ManageError::NotFound {
                    machine: machine.to_owned(),
                });
            }
        }
        // A machine added before the host kept keys gets one now.
        let key = (!self.peer_key(machine).exists()).then(Key::new);
        let ready = self.set_up(&link, key.as_ref(), drive)?;
        let entry = registered(entry, &ready);
        self.hosts.put(entry.clone())?;
        Ok(entry)
    }

    /// Remove machine (LD-10) once the daemon checked the plan and the
    /// user's presence: removes humatad2 and its service from the machine,
    /// forgets the host key, drops the entry.
    pub fn remove(&self, machine: &str, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let _busy = self.claim(machine)?;
        let mut entry = self.get(machine)?;
        let link = self.link(&entry)?;
        if entry.state == EntryState::Adding {
            return self.undo_add(&link, drive);
        }
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
                let link = self.link(&entry)?;
                match entry.state {
                    EntryState::Adding => self.undo_add(&link, drive),
                    EntryState::Removing => self.finish_remove(&link, drive),
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

    fn claim(&self, machine: &str) -> Result<BusyGuard<'_>, ManageError> {
        let mut busy = self.busy.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
        let link = self.link(&self.get(machine)?)?;
        link.peer().map_err(|source| ManageError::Quote {
            machine: machine.to_owned(),
            source,
        })
    }

    fn link(&self, entry: &HostEntry) -> Result<Link, ManageError> {
        let machine = MachineName::new(&entry.name).map_err(|source| ManageError::BadName {
            value: entry.name.clone(),
            source,
        })?;
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
        let machine = link.machine().as_str().to_owned();
        let mut setup = Setup::existing(link.clone(), &self.known_hosts).with_retry(self.retry);
        if let Some(key) = key {
            setup = setup.with_enrollment(key.clone());
        }
        let mut current = OpStep::of(setup.current());
        (drive.events)(StepEvent::Running(current));
        loop {
            let progress = setup.step(drive.runner, drive.builds);
            match progress {
                Progress::Continue => {
                    let next = OpStep::of(setup.current());
                    if next != current {
                        (drive.events)(StepEvent::Done(current));
                        stop_if_cancelled(link, drive)?;
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
                    stop_if_cancelled(link, drive)?;
                }
                Progress::NeedsLogin(login) => {
                    (drive.events)(StepEvent::Failed {
                        step: current,
                        output_tail: login.output_tail.clone(),
                    });
                    return Err(ManageError::LoginNeeded {
                        machine,
                        output_tail: login.output_tail,
                    });
                }
                Progress::Ready(ready) => {
                    if let Some(key) = key {
                        enroll::write_host(&self.peer_keys, link.machine(), key).map_err(|source| {
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

    /// Undoes an add: without a pinned key nothing reached the machine, so
    /// only the entry goes; otherwise humatad2 is removed first, and when
    /// that can't be done the entry stays with `undo_pending`.
    fn undo_add(&self, link: &Link, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let machine = link.machine();
        if known_hosts::pinned(&self.known_hosts, machine)?.is_some() {
            if let Err(error) = self.uninstall(link, drive) {
                if let Some(mut entry) = self.hosts.get(machine.as_str())? {
                    entry.undo_pending = true;
                    self.hosts.put(entry)?;
                }
                return Err(error);
            }
            (drive.events)(StepEvent::Undone(OpStep::Install));
            known_hosts::unpin(&self.known_hosts, machine)?;
        }
        self.forget_peer_key(machine.as_str())?;
        self.hosts.remove(machine.as_str())?;
        Ok(())
    }

    fn finish_remove(&self, link: &Link, drive: &mut Drive<'_>) -> Result<(), ManageError> {
        let machine = link.machine();
        if known_hosts::pinned(&self.known_hosts, machine)?.is_some() {
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

fn tail(output: &Output) -> String {
    output.tail(TAIL_LINES)
}

fn registered(mut entry: HostEntry, ready: &setup::Ready) -> HostEntry {
    entry.state = EntryState::Ready;
    entry.undo_pending = false;
    entry.os = ready.facts.platform.os.to_string();
    entry.arch = ready.facts.platform.arch.to_string();
    entry.staged = ready
        .staged
        .iter()
        .map(|file: &InstalledFile| file.file_name().to_owned())
        .collect();
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
