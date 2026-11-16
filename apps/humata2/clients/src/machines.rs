//! The machines client: adding a machine, logging in to it by hand, and a
//! machine's page (HO-1 to HO-7, HO-13, MP-2, MP-4, MP-12 to MP-17, SK-2,
//! SK-5, SK-8, SK-11). One trait, [`Machines`], that mirrors the daemon's
//! machine, provider and skills services at the app's level;
//! [`fake::FakeMachines`] is the only implementation until the real one is
//! wired in over `api/proto`.

use tokio::sync::mpsc;

use crate::daemon::{DaemonError, Reply};
use crate::model::{Agent, Capability, ChannelId, MachineId, RuleSetting, SessionId};

pub mod fake;

pub use fake::{Call, FakeMachines};

/// An add in progress on a daemon. Asking again with it retries the same add.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId(pub String);

impl OperationId {
    /// The id written as `text`.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

/// A host from `~/.ssh/config` on this computer (HO-1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshHost {
    /// The name in the config file, which is also what is typed to reach it.
    pub name: String,
    /// What the config says it reaches, such as `avesta@10.0.4.22`.
    pub address: String,
}

/// One value a provider asks for (MP-4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderParam {
    /// The name the provider's commands use.
    pub name: String,
    /// What to call it in the form.
    pub label: String,
    /// What it starts as.
    pub default: String,
    /// Whether the add may not start without it.
    pub required: bool,
}

/// A way to make a machine that isn't a plain SSH host (MP-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    /// Its name in the providers file.
    pub id: String,
    /// What to call it in the picker.
    pub label: String,
    /// Whether it always logs in by hand (MP-12).
    pub interactive: bool,
    /// What it asks for.
    pub params: Vec<ProviderParam>,
}

/// What the add form offers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Choices {
    /// Hosts from `~/.ssh/config`.
    pub hosts: Vec<SshHost>,
    /// Providers.
    pub providers: Vec<Provider>,
}

/// What to add.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddTarget {
    /// A plain SSH host, by the name or address typed or picked.
    Ssh {
        /// The host.
        host: String,
    },
    /// A machine a provider makes.
    Provider {
        /// The provider's id.
        provider: String,
        /// The values for its params, by name.
        params: std::collections::BTreeMap<String, String>,
    },
}

/// A request to add a machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddRequest {
    /// What to add.
    pub target: AddTarget,
    /// What to call it; the host's own name if empty.
    pub name: Option<String>,
    /// The host key the person confirmed, so the add trusts it.
    pub trusted_fingerprint: Option<String>,
}

/// One step of an add (HO-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StepKind {
    /// Make the machine; only a provider add has this step, first.
    Create,
    /// Connect over SSH.
    Connect,
    /// Log in by hand in the dialog's terminal. Only the dialog adds this step, once the
    /// login worked (the daemon has no such step), in the place of the connect attempt
    /// that asked for it (MP-15).
    Login,
    /// Check the system: arch, disk, git, gh.
    Check,
    /// Install humatad.
    Install,
    /// Start it and open the link.
    Start,
    /// Find agents.
    FindAgents,
}

impl StepKind {
    /// The step's name as the dialog shows it.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Create => "Create the machine",
            Self::Connect => "Connect over SSH",
            Self::Login => "Log in",
            Self::Check => "Check the system (arch, disk, git, gh)",
            Self::Install => "Install humatad",
            Self::Start => "Start it and open the link",
            Self::FindAgents => "Find agents",
        }
    }
}

/// Where a step is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStatus {
    /// Not reached.
    NotRun,
    /// Going now.
    Running,
    /// Finished.
    Done,
    /// Stopped here.
    Failed,
}

/// A step changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepUpdate {
    /// Which step.
    pub step: StepKind,
    /// Where it is now.
    pub status: StepStatus,
    /// The line under it, such as "devbox-2 answered as avesta@10.0.4.22".
    pub detail: String,
}

/// A command that fixes a failure, and where to run it (HO-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    /// The command.
    pub command: String,
    /// Where to run it, such as "devbox-2".
    pub where_to_run: String,
}

/// Why an add stopped (HO-2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddFailure {
    /// The step that failed.
    pub step: StepKind,
    /// A short title, such as "Install humatad failed on devbox-2".
    pub title: String,
    /// What went wrong and why.
    pub what: String,
    /// What to run to fix it.
    pub fix: Option<Fix>,
}

/// How an add ended. Exactly one comes, last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddOutcome {
    /// The machine is added.
    Added(MachineId),
    /// A step failed.
    Failed(AddFailure),
    /// The host's key isn't known; the person must confirm it.
    HostKey {
        /// The key's fingerprint, to show.
        fingerprint: String,
    },
    /// The connect command waits for a login the person must do (MP-13).
    NeedsLogin {
        /// Why: the prompt it printed, or that it hung (D167).
        reason: LoginReason,
    },
}

/// What a login prompt asks for (D167). The daemon decides it; the app never
/// reads the prompt line to guess.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptKind {
    /// A password.
    Password,
    /// A key's passphrase.
    Passphrase,
    /// A verification, one-time or 2FA code.
    Code,
    /// A yes or no question.
    Confirm,
    /// A URL to sign in at, with a code to enter there.
    Device,
    /// Any other prompt, or none the daemon could name.
    Other,
}

/// Why an add needs the person to log in (MP-13, D167).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginReason {
    /// The command printed a prompt.
    Prompted {
        /// What kind.
        kind: PromptKind,
        /// The line, such as "Verification code:".
        line: String,
    },
    /// The command ran until its timeout without finishing.
    Hung,
}

/// What an add reports, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AddEvent {
    /// The add began: its id and the steps it will take, in order.
    Started {
        /// The operation, for Try again and Cancel.
        operation: OperationId,
        /// The steps.
        steps: Vec<StepKind>,
    },
    /// A step moved on.
    Step(StepUpdate),
    /// It ended.
    Outcome(AddOutcome),
}

/// Follows an add. The stream ends after the outcome.
pub type AddStream = mpsc::UnboundedReceiver<Result<AddEvent, DaemonError>>;

/// What an interactive login belongs to (MP-14).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum LoginTarget {
    /// An add that asked for a login.
    Operation(OperationId),
    /// A machine that needs the person to log in again.
    Machine(MachineId),
}

/// What an interactive login reports, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginEvent {
    /// Bytes the terminal printed.
    Output(Vec<u8>),
    /// The terminal first ended waiting for the person (MP-13, D167).
    Prompt {
        /// What kind of prompt.
        kind: PromptKind,
        /// The line, such as "Verification code:".
        line: String,
    },
    /// A step of the add moved on after the login.
    Step(StepUpdate),
    /// Logged in and humatad is running (MP-15).
    Ready {
        /// The machine.
        machine: MachineId,
        /// Whether reconnects can be made to not need the person (MP-17).
        can_make_unattended: bool,
        /// humatad's version on the machine, when the daemon knew it yet (frame 58).
        daemon_version: Option<String>,
    },
    /// The login didn't work.
    Failed {
        /// Why, and what to do.
        reason: String,
    },
}

/// Follows a login.
pub type LoginStream = mpsc::UnboundedReceiver<Result<LoginEvent, DaemonError>>;

/// How to make reconnects not need the person (MP-17).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnattendedMethod {
    /// Add a Humata key to `authorized_keys`.
    AddKey,
    /// Keep the provider's cached token.
    KeepToken,
}

/// How a machine is reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Over SSH.
    Ssh,
    /// Through a provider's connect command.
    Provider(String),
    /// This computer.
    Local,
}

/// A signed-in `gh` (HO-4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GhLogin {
    /// The user.
    pub user: String,
    /// The token's scopes.
    pub scopes: Vec<String>,
}

/// What a machine's health says (HO-4, SK-8).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineHealth {
    /// `gh`'s user, or none when not signed in.
    pub gh: Option<GhLogin>,
    /// The agents found, with versions.
    pub agents: Vec<(Agent, String)>,
    /// Free disk, in bytes.
    pub disk_free: u64,
    /// Disk in all, in bytes.
    pub disk_total: u64,
    /// Host variable names; values never come back (HO-13).
    pub variables: Vec<String>,
    /// Why the last skills sync failed, if it did (SK-8).
    pub skills_problem: Option<String>,
    /// The version of `git` on the machine, such as `2.43.0`; empty without git (HO-1).
    pub git_version: String,
    /// The version of `gh` on the machine, such as `2.83.0`; empty without gh (HO-4).
    pub gh_version: String,
}

/// A version as a chip shows it: major and minor, so `2.4.1` is `2.4` and `0.98.0` is
/// `0.98` (frames 28 and 59, HO-4). A version without a dot stays as it is.
#[must_use]
pub fn short_version(version: &str) -> String {
    version.trim().split('.').take(2).collect::<Vec<_>>().join(".")
}

/// What a tool's chip says: "claude 2.4", or the name alone when its version is
/// not known (HO-4).
#[must_use]
pub fn tool_label(name: &str, version: &str) -> String {
    let version = short_version(version);
    if version.is_empty() { name.to_owned() } else { format!("{name} {version}") }
}

/// A channel's Humata folder on a machine (FM-11).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelFolder {
    /// The channel.
    pub channel: ChannelId,
    /// The folder, such as `~/Humata/infra/`.
    pub path: String,
    /// How many sessions use it.
    pub sessions: usize,
    /// Its size, in bytes.
    pub bytes: u64,
}

/// What a machine's agents may do through the host, and the rule's setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReachRule {
    /// The rule.
    pub capability: Capability,
    /// What the rule is called, such as "Read files on other machines".
    pub label: String,
    /// What it does.
    pub setting: RuleSetting,
}

/// Everything a machine's page shows (HO-4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineDetail {
    /// The machine.
    pub machine: MachineId,
    /// Its operating system, such as "Ubuntu 24.04".
    pub os: String,
    /// Its architecture.
    pub arch: String,
    /// Where it is reached.
    pub address: String,
    /// How.
    pub reach: Reach,
    /// humatad's version.
    pub daemon_version: String,
    /// Its health.
    pub health: MachineHealth,
    /// Its Humata folders, by channel.
    pub folders: Vec<ChannelFolder>,
    /// The sessions on it.
    pub sessions: Vec<SessionId>,
    /// What its agents can reach.
    pub rules: Vec<ReachRule>,
}

/// A skill on the machine with the same name as a synced one (SK-5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillClash {
    /// The skill's name.
    pub name: String,
}

/// What a sync would do (SK-5, SK-11).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SkillsPreview {
    /// Whether this machine has never synced.
    pub first_sync: bool,
    /// Skills that would be added.
    pub added: Vec<String>,
    /// Skills that would change.
    pub updated: Vec<String>,
    /// Skills that would go.
    pub removed: Vec<String>,
    /// Names the machine already has of its own.
    pub clashes: Vec<SkillClash>,
}

/// Which skill a machine's agents use when two share a name (SK-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClashChoice {
    /// The synced one.
    Synced,
    /// The one made on the machine.
    MachineLocal,
}

/// What the app asks about machines.
pub trait Machines: Send + Sync + 'static {
    /// The SSH hosts and providers to offer (HO-1, MP-2).
    fn choices(&self) -> Reply<Choices>;
    /// Starts an add and follows it.
    fn add(&self, request: AddRequest) -> Reply<AddStream>;
    /// Tries the same add again (HO-2).
    fn retry(&self, operation: &OperationId) -> Reply<AddStream>;
    /// Stops an add that is running.
    fn cancel(&self, operation: &OperationId) -> Reply<()>;
    /// Opens the interactive login and follows it (MP-14).
    fn open_login(&self, target: &LoginTarget) -> Reply<LoginStream>;
    /// Types into the login's terminal.
    fn login_input(&self, target: &LoginTarget, bytes: Vec<u8>) -> Reply<()>;
    /// Tells the login's terminal its size.
    fn login_resize(&self, target: &LoginTarget, columns: u16, rows: u16) -> Reply<()>;
    /// "Done, I'm logged in" (MP-15).
    fn login_done(&self, target: &LoginTarget) -> Reply<()>;
    /// Closes the login without finishing it.
    fn login_cancel(&self, target: &LoginTarget) -> Reply<()>;
    /// Makes reconnects not need the person (MP-17).
    fn make_unattended(&self, machine: &MachineId, method: UnattendedMethod) -> Reply<()>;
    /// What a machine's page shows (HO-4).
    fn detail(&self, machine: &MachineId) -> Reply<MachineDetail>;
    /// What a skills sync would do (SK-5, SK-11).
    fn skills_preview(&self, machine: &MachineId) -> Reply<SkillsPreview>;
    /// Syncs skills to a machine.
    fn sync_skills(&self, machine: &MachineId) -> Reply<()>;
    /// Picks which skill wins a name clash on a machine (SK-5).
    fn resolve_clash(&self, machine: &MachineId, name: &str, choice: ClashChoice) -> Reply<()>;
    /// Sets a host variable. The value goes in and never comes back (HO-13).
    fn set_variable(&self, machine: &MachineId, name: &str, value: &str) -> Reply<()>;
    /// Removes a host variable (HO-13).
    fn remove_variable(&self, machine: &MachineId, name: &str) -> Reply<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_shows_major_and_minor() {
        assert_eq!(tool_label("claude", "2.4.1"), "claude 2.4");
        assert_eq!(tool_label("codex", "0.98.0"), "codex 0.98");
        assert_eq!(tool_label("gh", "2.83"), "gh 2.83");
        assert_eq!(tool_label("codex", "0.98.0-beta.1"), "codex 0.98");
        assert_eq!(tool_label("copilot", "7"), "copilot 7");
    }

    #[test]
    fn a_tool_with_no_known_version_shows_its_name_alone() {
        assert_eq!(tool_label("claude", ""), "claude");
        assert_eq!(tool_label("claude", "  "), "claude");
    }
}
