//! Setting a machine up, one step at a time (MP-19): create, identity,
//! connect with retry while it boots, check, install, service. The caller
//! drives it, sleeping when told to retry, so the steps stay testable with a
//! fake runner and no clock.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::check::{self, Facts};
use crate::error::{Remedy, SetupError, ValuesError};
use crate::identity::{self, HostKey, Identity};
use crate::install::{self, InstallProblem, Installed, InstalledFile, Payload};
use crate::known_hosts;
use crate::link::Link;
use crate::name::MachineName;
use crate::platform::{Os, Platform};
use crate::provider::{Action, Provider, Reach};
use crate::runner::{Exit, Output, Purpose, Runner};
use crate::service::{self, ServiceKind, ServiceOutcome};

/// How many output lines a failed step keeps (MP-20).
const TAIL_LINES: usize = 40;
/// The least free space a machine needs for humatad2, clones and logs.
const NEED_FREE_MIB: u64 = 2 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
const INSTALL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SERVICE_TIMEOUT: Duration = Duration::from_secs(2 * 60);

/// Words that mean a command is waiting for someone to log in (MP-13).
const LOGIN_PROMPTS: [&str; 10] = [
    "password:",
    "password for",
    "passphrase",
    "verification code",
    "(yes/no",
    "one-time",
    "device login",
    "enter the code",
    "enter code",
    "login code",
];

/// The steps, in order. Register (hosts.json) belongs to the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    Create,
    Identity,
    Connect,
    Check,
    Install,
    Service,
}

/// How the connect step retries while a machine boots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub attempts: u32,
    pub first_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 10,
            first_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    /// The wait after failed attempt `attempt` (from 1): doubling, capped.
    fn delay(&self, attempt: u32) -> Duration {
        let doublings = attempt.saturating_sub(1).min(16);
        self.first_delay
            .saturating_mul(1 << doublings)
            .min(self.max_delay)
    }
}

/// What the caller does next.
#[derive(Debug)]
pub enum Progress {
    /// Call [`Setup::step`] again for the next step.
    Continue,
    /// The machine isn't answering yet; wait `delay`, then call again.
    RetryAfter {
        delay: Duration,
        attempt: u32,
        of: u32,
    },
    /// Reaching the machine needs a person: offer Open interactively
    /// (MP-13). Calling again tries non-interactively once more.
    NeedsLogin(LoginNeeded),
    /// The machine is set up; the daemon registers it (MP-19).
    Ready(Ready),
    /// The step failed and stays current: calling again is Try again.
    Failed(Failure),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginNeeded {
    pub machine: MachineName,
    pub reason: LoginReason,
    pub output_tail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginReason {
    /// It printed a password, code, `(yes/no)?` or device-login prompt.
    Prompted,
    /// It ran until its timeout without finishing.
    Hung,
}

/// A failed step, its output, and what the person can do (MP-20).
#[derive(Debug)]
pub struct Failure {
    pub step: Step,
    pub error: SetupError,
    pub output_tail: String,
    pub remedies: Vec<Remedy>,
}

/// A machine ready to register.
#[derive(Debug, Clone)]
pub struct Ready {
    pub link: Link,
    pub facts: Facts,
    pub service: ServiceKind,
    /// Files whose new copy waits beside the old one because a holder runs
    /// (TS-1); the next connect after the holders end swaps them in.
    pub staged: Vec<InstalledFile>,
}

/// One file the install step still has to do.
#[derive(Debug, Clone)]
enum Pending {
    /// Send the bytes.
    Send(Payload),
    /// A verified copy is already staged there; move it in.
    Promote(Payload),
}

impl Pending {
    fn payload(&self) -> &Payload {
        match self {
            Self::Send(payload) | Self::Promote(payload) => payload,
        }
    }
}

/// One machine's setup.
pub struct Setup {
    machine: MachineName,
    provider: Option<Provider>,
    values: BTreeMap<String, String>,
    known_hosts: PathBuf,
    retry: RetryPolicy,
    step: Step,
    attempt: u32,
    created: bool,
    link: Option<Link>,
    identity: Option<Identity>,
    facts: Option<Facts>,
    pending: Vec<Pending>,
    staged: Vec<InstalledFile>,
    service: Option<ServiceKind>,
}

/// A step's failure before remedies are worked out.
struct Failed {
    error: SetupError,
    tail: String,
}

impl Failed {
    fn new(error: impl Into<SetupError>) -> Self {
        Self {
            error: error.into(),
            tail: String::new(),
        }
    }

    fn with_output(error: impl Into<SetupError>, output: &Output) -> Self {
        Self {
            error: error.into(),
            tail: output.tail(TAIL_LINES),
        }
    }
}

/// What a step did when it didn't fail.
enum Done {
    Next(Step),
    Retry(Duration),
    Login(LoginReason, String),
    Ready,
}

impl Setup {
    /// Makes `machine` with `provider`, from Create. `values` are the params
    /// typed into Add machine; `known_hosts` is Humata's own file (MP-9).
    pub fn new(
        provider: Provider,
        machine: MachineName,
        values: BTreeMap<String, String>,
        known_hosts: &Path,
    ) -> Result<Self, ValuesError> {
        provider.environment(&machine, &values)?;
        Ok(Self::empty(machine, Some(provider), values, known_hosts, Step::Create))
    }

    /// Sets up a machine that already exists and is reached through `link`
    /// (an SSH host typed in, MP-10, or a provider machine being repaired),
    /// from Connect.
    pub fn existing(link: Link, known_hosts: &Path) -> Self {
        let mut setup = Self::empty(
            link.machine().clone(),
            None,
            BTreeMap::new(),
            known_hosts,
            Step::Connect,
        );
        setup.link = Some(link);
        setup
    }

    fn empty(
        machine: MachineName,
        provider: Option<Provider>,
        values: BTreeMap<String, String>,
        known_hosts: &Path,
        step: Step,
    ) -> Self {
        Self {
            machine,
            provider,
            values,
            known_hosts: known_hosts.to_owned(),
            retry: RetryPolicy::default(),
            step,
            attempt: 0,
            created: false,
            link: None,
            identity: None,
            facts: None,
            pending: Vec::new(),
            staged: Vec::new(),
            service: None,
        }
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn machine(&self) -> &MachineName {
        &self.machine
    }

    /// The step the next call runs.
    pub fn current(&self) -> Step {
        self.step
    }

    /// The link, once create (or identity) has made one.
    pub fn link(&self) -> Option<&Link> {
        self.link.as_ref()
    }

    /// The provider's `delete` for what `create` made, for **Delete what
    /// was made** (MP-20); none before create ran or without `delete`.
    pub fn delete_what_was_made(&self) -> Option<crate::runner::Invocation> {
        let provider = self.provider.as_ref()?;
        if !self.created {
            return None;
        }
        provider
            .invocation(Action::Delete, &self.machine, &self.values)
            .ok()
            .flatten()
    }

    /// Learns the host key the machine presents, without offering it any
    /// credential, for **Confirm host key** (MP-10): the person compares its
    /// fingerprint with the machine's own before [`Setup::trust_host_key`].
    /// Only for SSH Humata runs itself; a provider's `connect` checks keys
    /// in its own tool.
    pub fn scan_host_key(&self, runner: &dyn Runner) -> Result<HostKey, Failure> {
        let machine = self.machine.clone();
        let fail = |error: SetupError, output_tail: String| Failure {
            step: Step::Connect,
            error,
            output_tail,
            remedies: vec![Remedy::TryAgain],
        };
        let link = self
            .require_link()
            .map_err(|failed| fail(failed.error, String::new()))?;
        let Some(invocation) = link.scan_host_key() else {
            return Err(fail(SetupError::ConnectHostKey { machine }, String::new()));
        };
        let output = runner
            .run(&invocation)
            .map_err(|error| fail(error.into(), String::new()))?;
        known_hosts::keys_for(&String::from_utf8_lossy(&output.stdout), &machine)
            .into_iter()
            .next()
            .ok_or_else(|| {
                // stdout holds only keys; the tail is ssh's own words.
                let tail = Output {
                    stdout: Vec::new(),
                    ..output.clone()
                }
                .tail(TAIL_LINES);
                fail(
                    SetupError::HostKeyScanFailed {
                        machine: machine.clone(),
                        exit: output.exit,
                    },
                    tail,
                )
            })
    }

    /// Pins `key` as the machine's host key in Humata's `known_hosts`,
    /// replacing any it had, once the person confirmed its fingerprint or
    /// chose to trust a changed key (MP-9). Then try the step again.
    pub fn trust_host_key(&self, key: &HostKey) -> Result<(), SetupError> {
        let machine = self.machine.clone();
        let path = match &self.link {
            Some(link) if link.is_connect() => return Err(SetupError::ConnectHostKey { machine }),
            Some(link) => link.known_hosts().unwrap_or(self.known_hosts.as_path()),
            None => self.known_hosts.as_path(),
        };
        known_hosts::pin(path, &machine, key)?;
        Ok(())
    }

    /// Use `setsid` and crontab instead of systemd from now on (PER-19),
    /// when lingering or the user's service manager can't be had.
    pub fn use_setsid(&mut self) {
        self.service = Some(ServiceKind::Setsid);
    }

    /// Runs the current step.
    pub fn step(&mut self, runner: &dyn Runner, builds: &dyn install::Builds) -> Progress {
        let step = self.step;
        let result = match step {
            Step::Create => self.create(runner),
            Step::Identity => self.identify(runner),
            Step::Connect => self.connect(runner),
            Step::Check => self.check(runner, builds),
            Step::Install => self.install(runner),
            Step::Service => self.start_service(runner),
        };
        match result {
            Ok(Done::Next(next)) => {
                self.step = next;
                self.attempt = 0;
                Progress::Continue
            }
            Ok(Done::Retry(delay)) => Progress::RetryAfter {
                delay,
                attempt: self.attempt,
                of: self.retry.attempts,
            },
            Ok(Done::Login(reason, output_tail)) => {
                self.attempt = 0;
                Progress::NeedsLogin(LoginNeeded {
                    machine: self.machine.clone(),
                    reason,
                    output_tail,
                })
            }
            Ok(Done::Ready) => {
                let ready = match (&self.link, &self.facts, self.service) {
                    (Some(link), Some(facts), Some(service)) => Some(Ready {
                        link: link.clone(),
                        facts: facts.clone(),
                        service,
                        staged: self.staged.clone(),
                    }),
                    _ => None,
                };
                match ready {
                    Some(ready) => {
                        // Calling again re-checks from connect.
                        self.step = Step::Connect;
                        self.attempt = 0;
                        Progress::Ready(ready)
                    }
                    None => {
                        let machine = self.machine.clone();
                        self.fail(step, Failed::new(SetupError::BadProbe { machine }))
                    }
                }
            }
            Err(failed) => self.fail(step, failed),
        }
    }

    fn fail(&mut self, step: Step, failed: Failed) -> Progress {
        self.attempt = 0;
        let remedies = self.remedies(step, &failed.error);
        Progress::Failed(Failure {
            step,
            error: failed.error,
            output_tail: failed.tail,
            remedies,
        })
    }

    fn remedies(&self, step: Step, error: &SetupError) -> Vec<Remedy> {
        let mut remedies = vec![Remedy::TryAgain];
        match error {
            SetupError::CommandFailed { .. }
            | SetupError::Identity(_)
            | SetupError::Values(_)
            | SetupError::Quote(_)
            | SetupError::ToolMissing { .. }
            | SetupError::BadProbe { .. }
            | SetupError::IdentityMismatch { .. } => {
                if self.provider.is_some() {
                    remedies.push(Remedy::FixProvider);
                }
            }
            SetupError::HostKeyUnknown { .. } | SetupError::HostKeyChanged { .. } => {
                if self.link.as_ref().is_some_and(|link| !link.is_connect()) {
                    remedies.push(Remedy::ConfirmHostKey);
                }
            }
            SetupError::LingerUnavailable { admin_command, .. }
            | SetupError::UserManagerUnavailable { admin_command, .. } => {
                remedies.push(Remedy::RunAsAdmin {
                    command: admin_command.clone(),
                });
                if self.facts.as_ref().is_some_and(|facts| facts.setsid && facts.crontab) {
                    remedies.push(Remedy::StartWithoutSystemd);
                }
            }
            _ => {}
        }
        // Any connect step also offers Open interactively (MP-12).
        if step == Step::Connect && !matches!(error, SetupError::HostKeyChanged { .. }) {
            remedies.push(Remedy::OpenInteractively);
        }
        if self.delete_what_was_made().is_some() {
            remedies.push(Remedy::DeleteWhatWasMade);
        }
        remedies
    }

    fn create(&mut self, runner: &dyn Runner) -> Result<Done, Failed> {
        let Some(provider) = &self.provider else {
            return Ok(Done::Next(Step::Connect));
        };
        let invocation = provider
            .invocation(Action::Create, &self.machine, &self.values)
            .map_err(Failed::new)?;
        if let Some(invocation) = invocation {
            // From here on something may exist to delete, even if it fails.
            self.created = true;
            let output = runner.run(&invocation).map_err(Failed::new)?;
            if !output.exit.success() {
                return Err(Failed::with_output(
                    SetupError::CommandFailed {
                        action: Action::Create,
                        exit: output.exit,
                    },
                    &output,
                ));
            }
        }
        match provider.reach() {
            Reach::Connect => {
                self.link = provider
                    .link(&self.machine, &self.values)
                    .map_err(Failed::new)?;
                Ok(Done::Next(Step::Connect))
            }
            Reach::Identity => Ok(Done::Next(Step::Identity)),
        }
    }

    fn identify(&mut self, runner: &dyn Runner) -> Result<Done, Failed> {
        let Some(provider) = &self.provider else {
            return Ok(Done::Next(Step::Connect));
        };
        let Some(invocation) = provider
            .invocation(Action::Identity, &self.machine, &self.values)
            .map_err(Failed::new)?
        else {
            return Ok(Done::Next(Step::Connect));
        };
        let output = runner.run(&invocation).map_err(Failed::new)?;
        // Key material is neither kept nor shown, not even as output.
        let secret = identity::has_key_material(&output.stdout)
            || identity::has_key_material(&output.stderr);
        let failed = |error: SetupError| Failed {
            error,
            tail: if secret {
                String::new()
            } else {
                output.tail(TAIL_LINES)
            },
        };
        if secret {
            return Err(failed(crate::error::IdentityError::KeyMaterial.into()));
        }
        if !output.exit.success() {
            return Err(failed(SetupError::CommandFailed {
                action: Action::Identity,
                exit: output.exit,
            }));
        }
        let identity = identity::parse_identity(&output.stdout, &self.machine)
            .map_err(|error| failed(error.into()))?;
        // A key the provider gave is pinned with no prompt (MP-9),
        // replacing the one an earlier machine of this name had.
        if let Some(key) = &identity.ssh.host_key {
            known_hosts::pin(&self.known_hosts, &self.machine, key).map_err(Failed::new)?;
        }
        self.link = Some(Link::ssh(
            self.machine.clone(),
            identity.ssh.clone(),
            &self.known_hosts,
        ));
        self.identity = Some(identity);
        Ok(Done::Next(Step::Connect))
    }

    fn connect(&mut self, runner: &dyn Runner) -> Result<Done, Failed> {
        let link = self.require_link()?;
        let invocation = link
            .script("uname -sm", Purpose::Probe, Some(CONNECT_TIMEOUT), None)
            .map_err(Failed::new)?;
        let output = runner.run(&invocation).map_err(Failed::new)?;
        let machine = self.machine.clone();
        if output.exit.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return match Platform::from_uname(&stdout) {
                Some(_) => Ok(Done::Next(Step::Check)),
                None => {
                    let error = match Platform::unknown_uname(&stdout) {
                        Some(uname) => SetupError::UnsupportedPlatform { machine, uname },
                        None => SetupError::BadProbe { machine },
                    };
                    Err(Failed::with_output(error, &output))
                }
            };
        }
        let text = output.text();
        let lower = text.to_lowercase();
        let changed = text.contains("REMOTE HOST IDENTIFICATION HAS CHANGED");
        if changed
            || lower.contains("host key verification failed")
            || lower.contains("no host key is known")
        {
            let error = match link.known_hosts() {
                // A provider's own tool refused the key, not Humata's pin.
                None => SetupError::ConnectHostKey { machine },
                Some(path) if changed => SetupError::HostKeyChanged {
                    expected_fingerprint: known_hosts::pinned(path, &machine)
                        .ok()
                        .flatten()
                        .map(|key| key.fingerprint().to_owned()),
                    presented_fingerprint: presented_fingerprint(&text),
                    machine,
                },
                Some(_) => SetupError::HostKeyUnknown { machine },
            };
            return Err(Failed::with_output(error, &output));
        }
        if output.exit == Exit::TimedOut {
            return Ok(Done::Login(LoginReason::Hung, output.tail(TAIL_LINES)));
        }
        if LOGIN_PROMPTS.iter().any(|prompt| lower.contains(prompt)) {
            return Ok(Done::Login(LoginReason::Prompted, output.tail(TAIL_LINES)));
        }
        if output.exit == Exit::Code(127) {
            return Err(Failed::with_output(SetupError::ToolMissing { machine }, &output));
        }
        // Still booting, sshd not up yet, or the key not installed yet.
        self.attempt += 1;
        if self.attempt >= self.retry.attempts {
            return Err(Failed::with_output(
                SetupError::Unreachable {
                    machine,
                    attempts: self.attempt,
                    exit: output.exit,
                },
                &output,
            ));
        }
        Ok(Done::Retry(self.retry.delay(self.attempt)))
    }

    fn check(&mut self, runner: &dyn Runner, builds: &dyn install::Builds) -> Result<Done, Failed> {
        let link = self.require_link()?;
        let invocation = link
            .script(&check::script(), Purpose::Check, Some(CHECK_TIMEOUT), None)
            .map_err(Failed::new)?;
        let output = runner.run(&invocation).map_err(Failed::new)?;
        let machine = self.machine.clone();
        let facts = match (output.exit.success(), check::parse(&output.stdout)) {
            (true, Some(facts)) => facts,
            _ => {
                return Err(Failed::with_output(
                    SetupError::CheckFailed {
                        machine,
                        exit: output.exit,
                    },
                    &output,
                ));
            }
        };
        let found = facts.platform;
        if let Some(identity) = &self.identity {
            if identity.platform != found {
                return Err(Failed::new(SetupError::IdentityMismatch {
                    machine,
                    said: identity.platform,
                    found,
                }));
            }
        }
        if found.os != Os::Linux {
            return Err(Failed::new(SetupError::Unsupported {
                machine,
                platform: found,
            }));
        }
        if !facts.git {
            return Err(Failed::new(SetupError::GitMissing { machine }));
        }
        if let Some(free_kib) = facts.disk_free_kib {
            if free_kib / 1024 < NEED_FREE_MIB {
                return Err(Failed::new(SetupError::DiskLow {
                    machine,
                    free_mib: free_kib / 1024,
                    need_mib: NEED_FREE_MIB,
                }));
            }
        }
        let service = match self.service {
            Some(ServiceKind::Setsid) => ServiceKind::Setsid,
            _ => ServiceKind::for_machine(&facts)
                .ok_or_else(|| Failed::new(SetupError::NoServiceManager { machine: machine.clone() }))?,
        };
        let payloads = builds.payloads(found).map_err(Failed::new)?;
        self.pending.clear();
        self.staged.clear();
        for payload in payloads {
            let file = payload.file();
            if facts.installed(file) == Some(payload.sha256()) {
                continue;
            }
            if facts.staged(file) == Some(payload.sha256()) {
                if facts.holders && file.waits_for_holders() {
                    // Still in use: it stays staged, and nothing is sent.
                    self.staged.push(file);
                } else {
                    self.pending.push(Pending::Promote(payload));
                }
                continue;
            }
            self.pending.push(Pending::Send(payload));
        }
        self.service = Some(service);
        self.facts = Some(facts);
        Ok(Done::Next(if self.pending.is_empty() {
            Step::Service
        } else {
            Step::Install
        }))
    }

    /// Installs the next file that differs from the build, or moves in a
    /// staged copy; one per call.
    fn install(&mut self, runner: &dyn Runner) -> Result<Done, Failed> {
        let Some(pending) = self.pending.first().cloned() else {
            return Ok(Done::Next(Step::Service));
        };
        let payload = pending.payload();
        let link = self.require_link()?;
        let invocation = match &pending {
            Pending::Send(payload) => link.script(
                &install::script(payload, &install::nonce()),
                Purpose::Install,
                Some(INSTALL_TIMEOUT),
                Some(Arc::clone(payload.bytes())),
            ),
            Pending::Promote(payload) => link.script(
                &install::promote_script(payload.file(), payload.sha256()),
                Purpose::Promote,
                Some(CHECK_TIMEOUT),
                None,
            ),
        }
        .map_err(Failed::new)?;
        let output = runner.run(&invocation).map_err(Failed::new)?;
        let file = payload.file();
        match Installed::from_exit(output.exit) {
            Some(Installed::Swapped) => {}
            Some(Installed::Staged) => self.staged.push(file),
            None => {
                let problem = InstallProblem::from_exit(output.exit);
                if matches!(pending, Pending::Promote(_)) {
                    // The staged copy went or was damaged: send it whole.
                    self.pending[0] = Pending::Send(payload.clone());
                }
                return Err(Failed::with_output(
                    SetupError::Install {
                        machine: self.machine.clone(),
                        file,
                        problem,
                    },
                    &output,
                ));
            }
        }
        self.pending.remove(0);
        Ok(Done::Next(if self.pending.is_empty() {
            Step::Service
        } else {
            Step::Install
        }))
    }

    fn start_service(&mut self, runner: &dyn Runner) -> Result<Done, Failed> {
        let link = self.require_link()?;
        let kind = self.service.unwrap_or(ServiceKind::SystemdUser);
        let invocation = link
            .script(&service::script(kind), Purpose::Service, Some(SERVICE_TIMEOUT), None)
            .map_err(Failed::new)?;
        let output = runner.run(&invocation).map_err(Failed::new)?;
        let machine = self.machine.clone();
        match ServiceOutcome::from_exit(output.exit) {
            ServiceOutcome::Running => {
                self.service = Some(kind);
                Ok(Done::Ready)
            }
            ServiceOutcome::LingerUnavailable => {
                let user = self.user();
                let admin_command = service::linger_command(&user);
                Err(Failed::with_output(
                    SetupError::LingerUnavailable {
                        machine,
                        user,
                        admin_command,
                    },
                    &output,
                ))
            }
            ServiceOutcome::UserManagerUnavailable => {
                let user = self.user();
                let admin_command = service::user_manager_command(&user);
                Err(Failed::with_output(
                    SetupError::UserManagerUnavailable {
                        machine,
                        user,
                        admin_command,
                    },
                    &output,
                ))
            }
            ServiceOutcome::DidNotStart => Err(Failed::with_output(
                SetupError::ServiceDidNotStart { machine },
                &output,
            )),
            ServiceOutcome::Failed(exit) => Err(Failed::with_output(
                SetupError::ServiceFailed { machine, exit },
                &output,
            )),
        }
    }

    /// The user the check found, for an admin command.
    fn user(&self) -> String {
        self.facts
            .as_ref()
            .map_or_else(String::new, |facts| facts.user.clone())
    }

    /// The link, which create, identity or [`Setup::existing`] always set
    /// before the steps that use it.
    fn require_link(&self) -> Result<&Link, Failed> {
        self.link.as_ref().ok_or_else(|| {
            Failed::new(SetupError::BadProbe {
                machine: self.machine.clone(),
            })
        })
    }
}

/// The `SHA256:…` fingerprint ssh says the machine presented, from its
/// changed-key warning.
fn presented_fingerprint(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|word| word.starts_with("SHA256:"))
        .map(|word| word.trim_end_matches('.').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_backs_off_doubling_up_to_the_cap() {
        let retry = RetryPolicy::default();
        let delays: Vec<u64> = (1..=6).map(|attempt| retry.delay(attempt).as_secs()).collect();
        assert_eq!(delays, [2, 4, 8, 16, 30, 30]);
    }
}
