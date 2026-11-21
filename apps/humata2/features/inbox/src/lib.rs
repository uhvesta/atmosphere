//! The Inbox (IN-1–10): what sessions ask the person, newest first. Each
//! request shows why it is asked, what it would do and what the agent was
//! doing, and is answered with Approve, Always…, Deny, or Deny and reply.
//!
//! A Migrate item (UP-1, D281) comes from Humata, not a session: it is VIP,
//! sits in its own "Humata" group, and is answered only with Migrate. An old local daemon can't hold one, so
//! the app holds its own for it until the daemon does (D374).

mod notice;
pub use notice::{Notice, notification, to_do_notification};

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{
    Answer, ChannelId, MachineId, MigrateCard, MigrateFailure, MigratePhase, MigrateThing, Preview,
    Release, Request, RequestId, RequestKind, SessionId,
};
use humata2_clients::{Clients, DaemonError, Workspace, is_this_mac};
use tca::{Effect, Reducer};

/// Which requests show.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    /// Every channel's.
    #[default]
    All,
    /// One channel's.
    Channel(ChannelId),
    /// One session's, as its side panel shows them.
    Session(SessionId),
}

/// Which answers are included in the wide Inbox list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum History {
    Waiting,
    Answered,
    All,
}

/// The app's own Migrate item, for a local daemon that predates UPGRADE
/// (D374): its ServerInfo has no `build` and a release is staged beside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OldLocalDaemon {
    /// This Mac's machine.
    pub machine: MachineId,
    /// The old daemon's version, as it reported it.
    pub running: String,
    /// The release staged beside it.
    pub staged: Release,
}

/// A heading of the wide Inbox list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Group {
    /// Humata's own items: every Migrate item.
    Humata,
    /// One channel's requests.
    Channel(ChannelId),
}

/// The answers a request's card offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Offer {
    /// Allow.
    Allow,
    /// Allow this session's browser reads for 30 minutes (BROWSER).
    /// Offered only for `RequestKind::BrowserRead` with `supports_30m_grant`.
    AllowFor30Min,
    /// Always allow.
    AlwaysAllow,
    /// Deny.
    Deny,
    /// Reply.
    Reply,
    /// Migrate.
    Migrate,
    /// Migrate again, after a failure.
    MigrateAgain,
    /// Dismiss, on a question read off its agent's screen: closes the item
    /// and presses nothing (D781). Its choices show beside it.
    Dismiss,
}

/// Where a Migrate item is, as its card shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrateStatus {
    /// Waiting for Migrate.
    Waiting,
    /// Migrating.
    Migrating,
    /// The last try failed.
    Failed(MigrateFailure),
    /// Migrated.
    Migrated {
        /// The release it is at now.
        to: Release,
        /// When, as people read it.
        at: String,
    },
}

/// How long the app waits for This Mac's daemon to come back at the staged
/// release after migrating it.
const LOCAL_DAEMON_WAIT: Duration = Duration::from_secs(60);

/// Effect ids of the two halves of that wait; each cancels the other.
const BACK_WAIT: &str = "inbox-local-daemon-back";
const LATE_WAIT: &str = "inbox-local-daemon-late";

/// How many "What changes" lines show while a card is folded.
const FOLDED_LINES: usize = 3;

/// A Migrate card's "What changes" list as it shows (D379).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangesFold {
    /// The lines that show.
    pub shown: Vec<String>,
    /// How many more lines a folded card hides; 0 when none are hidden.
    pub more: usize,
}

/// The app's own item's id: fixed and never sent to a daemon.
const OWN_ID: &str = "humata:migrate-local-daemon";

/// The Inbox's state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InboxState {
    /// Where the requests come from.
    pub workspace: Arc<Workspace>,
    /// Which show.
    pub filter: Filter,
    /// Whether answered requests show too.
    pub show_answered: bool,
    /// Limit the history to answered requests.
    pub answered_only: bool,
    /// The request whose details show.
    pub selected: Option<RequestId>,
    /// A reply being written to deny with, and to which request.
    pub reply: Option<(RequestId, String)>,
    /// Answers on their way.
    pub pending: BTreeSet<RequestId>,
    /// The requests whose "Allow this for the rest of the session" box is ticked (IN-14).
    pub always: BTreeSet<RequestId>,
    /// What went wrong last.
    pub error: Option<String>,
    /// The old local daemon the app holds its own Migrate item for.
    pub old_local: Option<OldLocalDaemon>,
    /// That item, with the phase it shows.
    pub own: Option<Request>,
    /// Migrate items clicked whose answer isn't back yet (or, for the app's
    /// own, whose daemon hasn't arrived).
    pub migrating: BTreeSet<RequestId>,
    /// The Migrate items whose "What changes" list is unfolded. A list of
    /// more than 3 lines is folded unless it is here.
    pub unfolded: BTreeSet<RequestId>,
    /// The copy cards whose Details, the real paths, are open.
    pub details: BTreeSet<RequestId>,
    /// The requests Ask why was sent for and whose request is not back yet.
    pub asking: BTreeSet<RequestId>,
    /// Whether the app should automatically trigger a waiting local daemon
    /// migration for This Mac (enabled by the live desktop app on launch).
    pub auto_migrate_local: bool,
    /// `(machine, staged_bundle)` pairs already auto-migrated in this run so a
    /// failure surfaces in the Inbox instead of looping infinitely.
    pub auto_migrated: BTreeSet<(MachineId, String)>,
}

/// How many requests wait for an answer. The sidebar's Inbox badge and Home's
/// "Needs you" both show this count, so they cannot disagree.
#[must_use]
pub fn waiting(workspace: &Workspace) -> usize {
    workspace
        .requests
        .iter()
        .filter(|request| request.answer.is_none() && !request.is_to_do_ask())
        .count()
}

/// The empty Agent inbox's summary line (frame 200, IN-8). Omits the
/// rule-followed count when `allowed_without_asking` is 0 (such as from a
/// daemon that doesn't report it yet).
#[must_use]
pub fn today_summary(answered: usize, allowed_without_asking: u32) -> String {
    if allowed_without_asking == 0 {
        format!("Today: {answered} answered ·")
    } else {
        let noun = if allowed_without_asking == 1 { "action" } else { "actions" };
        format!(
            "Today: {answered} answered · {allowed_without_asking} {noun} followed your rules without asking ·"
        )
    }
}

impl InboxState {
    /// The app's own Migrate item, unless the daemon holds one for the same
    /// thing and machine, which wins (D374).
    fn own_item(&self) -> Option<&Request> {
        let own = self.own.as_ref()?;
        let card = own.migrate()?;
        let held = self.workspace.requests.iter().any(|request| {
            request.answer.is_none()
                && request
                    .migrate()
                    .is_some_and(|other| other.thing == card.thing && other.machine == card.machine)
        });
        (!held).then_some(own)
    }

    /// The request with `id`, the app's own item included.
    fn find(&self, id: &RequestId) -> Option<&Request> {
        self.workspace
            .requests
            .iter()
            .find(|request| &request.id == id)
            .or_else(|| self.own_item().filter(|own| &own.id == id))
    }

    fn is_migrate(&self, id: &RequestId) -> bool {
        self.find(id)
            .is_some_and(|request| request.kind == RequestKind::Migrate)
    }

    fn is_own(&self, id: &RequestId) -> bool {
        self.own.as_ref().is_some_and(|own| &own.id == id)
    }

    /// Sets the phase a waiting Migrate item shows.
    fn set_phase(&mut self, id: &RequestId, phase: MigratePhase) {
        if let Some(own) = self.own.as_mut().filter(|own| &own.id == id) {
            if let Preview::Migrate(card) = &mut own.preview {
                card.phase = phase;
            }
            return;
        }
        let changes = self.workspace.requests.iter().any(|request| {
            &request.id == id
                && request.answer.is_none()
                && request.migrate().is_some_and(|card| card.phase != phase)
        });
        if !changes {
            return;
        }
        let found = Arc::make_mut(&mut self.workspace)
            .requests
            .iter_mut()
            .find(|request| &request.id == id);
        if let Some(Request { preview: Preview::Migrate(card), .. }) = found {
            card.phase = phase;
        }
    }

    /// Whether a request is VIP: a waiting Migrate item, or a waiting
    /// request of a VIP session.
    #[must_use]
    pub fn vip(&self, request: &Request) -> bool {
        request.answer.is_none()
            && (request.kind == RequestKind::Migrate
                || self
                    .workspace
                    .session(&request.session)
                    .is_some_and(|session| session.vip))
    }

    /// The shown requests by heading: Humata's group first (every Migrate
    /// item), then each channel in workspace order. Empty groups are left
    /// out.
    #[must_use]
    pub fn groups(&self) -> Vec<(Group, Vec<&Request>)> {
        let shown = self.shown();
        let mut groups = Vec::new();
        let humata: Vec<&Request> = shown
            .iter()
            .copied()
            .filter(|request| request.kind == RequestKind::Migrate)
            .collect();
        if !humata.is_empty() {
            groups.push((Group::Humata, humata));
        }
        for channel in &self.workspace.channels {
            let requests: Vec<&Request> = shown
                .iter()
                .copied()
                .filter(|request| request.kind != RequestKind::Migrate && request.channel == channel.id)
                .collect();
            if !requests.is_empty() {
                groups.push((Group::Channel(channel.id.clone()), requests));
            }
        }
        // A request of a channel the workspace doesn't list still shows.
        for request in shown.iter().copied().filter(|request| {
            request.kind != RequestKind::Migrate && self.workspace.channel(&request.channel).is_none()
        }) {
            let group = Group::Channel(request.channel.clone());
            match groups.iter_mut().find(|(found, _)| *found == group) {
                Some((_, requests)) => requests.push(request),
                None => groups.push((group, vec![request])),
            }
        }
        groups
    }

    /// The answers the request's card offers.
    #[must_use]
    pub fn offers(&self, id: &RequestId) -> Vec<Offer> {
        if let Some(status) = self.migrate_status(id) {
            return match status {
                MigrateStatus::Waiting => vec![Offer::Migrate],
                MigrateStatus::Failed(_) => vec![Offer::MigrateAgain],
                MigrateStatus::Migrating | MigrateStatus::Migrated { .. } => Vec::new(),
            };
        }
        let Some(request) = self.find(id).filter(|request| request.answer.is_none()) else {
            return Vec::new();
        };
        if request.on_screen.is_some() || request.dismiss_only {
            return vec![Offer::Dismiss];
        }
        if request.kind == RequestKind::Question {
            return vec![Offer::Reply];
        }
        if request.kind == RequestKind::BrowserRead {
            let allow_30m = request
                .browser()
                .is_some_and(|preview| preview.supports_30m_grant)
                .then_some(Offer::AllowFor30Min);
            return [Some(Offer::Allow), allow_30m, Some(Offer::Deny)]
                .into_iter()
                .flatten()
                .collect();
        }
        if request.kind == RequestKind::BrowserWrite {
            return vec![Offer::Allow, Offer::Deny];
        }
        let always = (!request.rule.is_empty()).then_some(Offer::AlwaysAllow);
        [Some(Offer::Allow), always, Some(Offer::Deny), Some(Offer::Reply)]
            .into_iter()
            .flatten()
            .collect()
    }

    /// Waiting requests in the current view that `Allow all` approves.
    /// Excludes `Migrate`, `BrowserWrite`, dismiss-only cards and provider imports.
    #[must_use]
    pub fn bulk_approvable(&self) -> Vec<&Request> {
        self.shown()
            .into_iter()
            .filter(|request| request.answer.is_none() && request.answered_in_bulk())
            .collect()
    }

    /// Waiting requests in the current view that `Deny all` denies.
    /// Excludes `Migrate`, dismiss-only cards and provider imports; includes `BrowserWrite`.
    #[must_use]
    pub fn bulk_deniable(&self) -> Vec<&Request> {
        self.shown()
            .into_iter()
            .filter(|request| {
                request.answer.is_none()
                    && request.kind != RequestKind::Migrate
                    && !request.dismiss_only
                    && request.operation != humata2_clients::model::PROVIDER_IMPORT_OPERATION
            })
            .collect()
    }

    /// Where a Migrate item is: `None` for any other request and for an id
    /// the Inbox no longer holds. An answered item still reads Migrated
    /// whatever the history filter shows.
    #[must_use]
    pub fn migrate_status(&self, id: &RequestId) -> Option<MigrateStatus> {
        let request = self.find(id)?;
        let card = request.migrate()?;
        if request.answer == Some(Answer::Approved) {
            return Some(MigrateStatus::Migrated {
                to: card.to.clone(),
                at: card.migrated.clone().unwrap_or_default(),
            });
        }
        if self.migrating.contains(id) {
            return Some(MigrateStatus::Migrating);
        }
        Some(match &card.phase {
            MigratePhase::Waiting => MigrateStatus::Waiting,
            MigratePhase::Migrating => MigrateStatus::Migrating,
            MigratePhase::Failed(failure) => MigrateStatus::Failed(failure.clone()),
        })
    }

    /// A Migrate item's "What changes" list: the first 3 lines and how many
    /// more while it is folded, all of them once unfolded. `None` for any
    /// other request and for an id the Inbox doesn't hold.
    #[must_use]
    pub fn changes(&self, id: &RequestId) -> Option<ChangesFold> {
        let card = self.find(id)?.migrate()?;
        let hidden = card.changes.len().saturating_sub(FOLDED_LINES);
        if hidden == 0 || self.unfolded.contains(id) {
            return Some(ChangesFold { shown: card.changes.clone(), more: 0 });
        }
        Some(ChangesFold {
            shown: card.changes.iter().take(FOLDED_LINES).cloned().collect(),
            more: hidden,
        })
    }

    /// Ends the app's own item Migrated: This Mac's daemon runs the staged
    /// release. Does nothing unless the item was migrating.
    fn own_migrated(&mut self) -> bool {
        let id = RequestId::new(OWN_ID);
        if !self.migrating.remove(&id) {
            return false;
        }
        if let Some(own) = &mut self.own {
            own.answer = Some(Answer::Approved);
            if let Preview::Migrate(card) = &mut own.preview {
                card.migrated = Some("just now".to_owned());
            }
        }
        self.select_visible();
        true
    }

    /// Every Migrate item the Inbox knows, the app's own included, with the
    /// phase it shows (a click in flight reads Migrating).
    #[must_use]
    pub fn migrations(&self) -> Vec<Request> {
        self.own_item()
            .into_iter()
            .chain(self.workspace.requests.iter())
            .filter(|request| request.kind == RequestKind::Migrate)
            .cloned()
            .map(|mut request| {
                if request.answer.is_none() && self.migrating.contains(&request.id) {
                    if let Preview::Migrate(card) = &mut request.preview {
                        card.phase = MigratePhase::Migrating;
                    }
                }
                request
            })
            .collect()
    }

    /// The channel chips over the cards: every channel that has a request,
    /// in workspace order, with how many of its requests wait.
    #[must_use]
    pub fn chips(&self) -> Vec<(ChannelId, usize)> {
        let mut channels: Vec<ChannelId> = self
            .workspace
            .channels
            .iter()
            .map(|channel| channel.id.clone())
            .filter(|id| {
                self.workspace
                    .requests
                    .iter()
                    .any(|request| &request.channel == id && !request.is_to_do_ask())
            })
            .collect();
        for request in &self.workspace.requests {
            if request.kind != RequestKind::Migrate
                && !request.is_to_do_ask()
                && !channels.contains(&request.channel)
            {
                channels.push(request.channel.clone());
            }
        }
        channels
            .into_iter()
            .map(|id| {
                let waiting = self
                    .workspace
                    .requests
                    .iter()
                    .filter(|request| {
                        request.channel == id
                            && request.answer.is_none()
                            && !request.is_to_do_ask()
                    })
                    .count();
                (id, waiting)
            })
            .collect()
    }

    /// The requests that show, newest first.
    #[must_use]
    pub fn shown(&self) -> Vec<&Request> {
        self.own_item()
            .into_iter()
            .chain(self.workspace.requests.iter())
            .filter(|request| !request.is_to_do_ask())
            .filter(|request| self.show_answered || request.answer.is_none())
            .filter(|request| !self.answered_only || request.answer.is_some())
            .filter(|request| match &self.filter {
                Filter::All => true,
                Filter::Channel(channel) => &request.channel == channel,
                Filter::Session(session) => &request.session == session,
            })
            .collect()
    }

    /// The selected request.
    #[must_use]
    pub fn selection(&self) -> Option<&Request> {
        let id = self.selected.as_ref()?;
        self.find(id)
    }

    /// The terminal the open item's detail shows: the session and tab of
    /// a question on its agent's screen, while it waits (D781). The open
    /// item is the selection when it shows, else the first shown.
    #[must_use]
    pub fn terminal(&self) -> Option<(SessionId, usize)> {
        let shown = self.shown();
        let open = self
            .selection()
            .filter(|selected| shown.iter().any(|request| request.id == selected.id))
            .or_else(|| shown.first().copied())?;
        let tab = open.on_screen.filter(|_| open.answer.is_none())?;
        Some((open.session.clone(), tab))
    }

    fn select_visible(&mut self) {
        let shown = self.shown();
        if !shown.iter().any(|r| Some(&r.id) == self.selected.as_ref()) {
            self.selected = shown.first().map(|r| r.id.clone());
        }
    }
}

/// What the Inbox asks its parent to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delegate {
    /// Open the session that asked (IN-4).
    OpenSession(SessionId),
    /// Open the session that asked this request and select the request (IN-11).
    OpenRequest {
        /// The session that asked.
        session: SessionId,
        /// The request to show there.
        request: RequestId,
    },
    /// Open the Files page on this machine, where a copy that asks to land
    /// would land: the user picks another place before answering (D467).
    OpenDestination(MachineId),
    /// Open Settings, Agent access, where the rule that asked is set.
    OpenAgentAccess,
    /// Say this in a toast.
    Toast(String),
    /// This Mac's daemon was replaced: connect to the new one.
    Reconnect,
}

/// What happens in the Inbox.
#[derive(Clone, Debug, PartialEq)]
pub enum InboxAction {
    /// The data changed.
    Show(Arc<Workspace>),
    /// Show these requests.
    SetFilter(Filter),
    /// Show or hide answered requests.
    ToggleAnswered,
    /// The History link: the answered requests instead of the waiting ones,
    /// and back.
    ToggleHistory,
    /// Ask why on a waiting request: the agent says why it asks, and the
    /// request stays held.
    AskWhy(RequestId),
    /// The daemon took Ask why (the request as it is now), or didn't.
    AskedWhy(RequestId, Result<Request, DaemonError>),
    /// "Change this rule": open Settings, Agent access.
    ChangeRule,
    /// Pick Waiting, Answered, or All.
    SetHistory(History),
    /// Show a request's details.
    Select(RequestId),
    /// Answer a request.
    Answer(RequestId, Answer),
    /// Approve every waiting request that `answered_in_bulk` allows (skips `BrowserWrite`).
    ApproveAll,
    /// Deny every waiting request that bulk deny allows.
    DenyAll,
    /// Approve and make the request's rule for next time.
    Always(RequestId),
    /// Start writing a reply to deny with.
    StartReply(RequestId),
    /// The reply's text changed.
    ReplyText(String),
    /// The text of the reply field on this request changed.
    ReplyDraft(RequestId, String),
    /// Deny with the reply.
    SendReply,
    /// Drop the reply.
    CancelReply,
    /// The daemon took an answer, or didn't.
    Answered(RequestId, Result<(), DaemonError>),
    /// Open the session that asked the selected request.
    OpenRequester,
    /// Tick or untick "Allow this for the rest of the session" on a request.
    ToggleAlways(RequestId),
    /// Answer a question with one of its choices.
    Choose(RequestId, String),
    /// Dismiss a question on its agent's screen: a Deny with no reply, which
    /// types nothing into the terminal (D781).
    Dismiss(RequestId),
    /// Open the session that asked this request, with the request selected.
    OpenRequest(RequestId),
    /// Migrate, or Migrate again, on a Migrate item. Does nothing unless the
    /// item waits or failed.
    Migrate(RequestId),
    /// From the app: the local daemon is too old to hold a Migrate item and
    /// a release is staged (`Some`), or it no longer needs the app's own
    /// item (`None`).
    OldLocalDaemon(Option<OldLocalDaemon>),
    /// From the live desktop app on launch: automatically trigger a waiting
    /// local daemon migration for This Mac once per `(machine, staged_bundle)`.
    EnableAutoMigrateLocal,
    /// Show or hide the rest of a Migrate card's "What changes" list.
    ToggleChanges(RequestId),
    /// "Somewhere else…" on a copy card: open the Files page on the machine
    /// the files would land on, to put them where the user wants. The
    /// request stays waiting. There is no way yet to approve a copy with
    /// another destination, so the user copies by hand and denies this one.
    SomewhereElse(RequestId),
    /// Show or hide the real paths behind a copy card's Details.
    ToggleDetails(RequestId),
    /// Copy a path a copy card shows (PL-10).
    CopyPath(String),
    /// Show a path a copy card shows in Finder (PL-10). Does nothing unless
    /// the path's machine is This Mac.
    Reveal(MachineId, String),
    /// Finder could not show the path; the message says why.
    RevealFailed(String),
    /// This Mac's daemon answers at the staged release.
    LocalDaemonBack,
    /// This Mac's daemon did not answer at the staged release in time.
    LocalDaemonLate,
    /// For the parent.
    Delegate(Delegate),
}

/// The Inbox's reducer.
#[derive(Clone)]
pub struct Inbox {
    clients: Clients,
}

impl Inbox {
    /// An Inbox that answers with `clients`.
    #[must_use]
    pub const fn new(clients: Clients) -> Self {
        Self { clients }
    }

    fn answer(&self, state: &mut InboxState, id: RequestId, answer: Answer) -> Effect<InboxAction> {
        if !state.pending.insert(id.clone()) {
            return Effect::none();
        }
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            let result = daemon.answer(&id, answer).await;
            InboxAction::Answered(id, result)
        })
    }

    fn migrate(&self, state: &mut InboxState, id: RequestId) -> Effect<InboxAction> {
        if !matches!(
            state.migrate_status(&id),
            Some(MigrateStatus::Waiting | MigrateStatus::Failed(_))
        ) {
            return Effect::none();
        }
        state.migrating.insert(id.clone());
        if state.is_own(&id) {
            // The old daemon can't take an answer: migrate it on This Mac.
            let bundle = state
                .own
                .as_ref()
                .and_then(Request::migrate)
                .map(|card| card.to.bundle.clone())
                .unwrap_or_default();
            let local = self.clients.local_daemon.clone();
            return Effect::future(async move {
                let result = local.migrate_old_daemon(&bundle).await;
                InboxAction::Answered(id, result)
            });
        }
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            let result = daemon.answer(&id, Answer::Approved).await;
            InboxAction::Answered(id, result)
        })
    }

    /// Triggers a waiting local daemon migration for This Mac once per
    /// `(machine, staged_bundle)` when auto-migration is enabled.
    fn auto_migrate_local_if_needed(&self, state: &mut InboxState) -> Effect<InboxAction> {
        if !state.auto_migrate_local {
            return Effect::none();
        }
        // 1. The app's own item for an old local daemon (!reports_build).
        let candidate = state
            .own_item()
            .and_then(|request| {
                let card = request.migrate()?;
                (request.answer.is_none()
                    && card.thing == MigrateThing::Daemon
                    && card.phase == MigratePhase::Waiting
                    && !state.migrating.contains(&request.id)
                    && !card.to.bundle.is_empty()
                    && !state
                        .auto_migrated
                        .contains(&(card.machine.clone(), card.to.bundle.clone())))
                .then(|| (request.id.clone(), card.machine.clone(), card.to.bundle.clone()))
            })
            // 2. A daemon-held Migrate item for This Mac's daemon from an older
            // running daemon that doesn't auto-migrate itself.
            .or_else(|| {
                state.workspace.requests.iter().find_map(|request| {
                    if request.kind != RequestKind::Migrate
                        || request.answer.is_some()
                        || state.migrating.contains(&request.id)
                    {
                        return None;
                    }
                    let card = request.migrate()?;
                    if card.thing != MigrateThing::Daemon
                        || card.phase != MigratePhase::Waiting
                        || card.to.bundle.is_empty()
                        || card.from.bundle == card.to.bundle
                        || !state.workspace.machine(&card.machine).is_some_and(is_this_mac)
                        || state
                            .auto_migrated
                            .contains(&(card.machine.clone(), card.to.bundle.clone()))
                    {
                        return None;
                    }
                    Some((request.id.clone(), card.machine.clone(), card.to.bundle.clone()))
                })
            });
        let Some((id, machine, bundle)) = candidate else {
            return Effect::none();
        };
        state.auto_migrated.insert((machine, bundle));
        self.migrate(state, id)
    }

    fn reveal_path(&self, state: &InboxState, machine: &MachineId, path: String) -> Effect<InboxAction> {
        if !state.workspace.machine(machine).is_some_and(is_this_mac) {
            return Effect::none();
        }
        let reveal = self.clients.reveal.clone();
        Effect::run(move |send| async move {
            if let Err(error) = reveal.reveal(&path) {
                send.send(InboxAction::RevealFailed(error.to_string()));
            }
        })
    }

    /// The wait for This Mac's daemon to answer at the staged release, and
    /// the bound on it.
    fn wait_for_local_daemon(&self, state: &InboxState) -> Effect<InboxAction> {
        let bundle = state
            .own
            .as_ref()
            .and_then(Request::migrate)
            .map(|card| card.to.bundle.clone())
            .unwrap_or_default();
        let local = self.clients.local_daemon.clone();
        let sleep = self.clients.clock.sleep(LOCAL_DAEMON_WAIT);
        Effect::merge([
            Effect::future(async move {
                match local.back_at(&bundle).await {
                    Ok(()) => InboxAction::LocalDaemonBack,
                    Err(_) => InboxAction::LocalDaemonLate,
                }
            })
            .cancellable(BACK_WAIT, true),
            Effect::future(async move {
                sleep.await;
                InboxAction::LocalDaemonLate
            })
            .cancellable(LATE_WAIT, true),
        ])
    }
}

/// How the app's own migrate failed when the daemon never came back.
fn late_failure() -> MigrateFailure {
    MigrateFailure {
        message: "This Mac's daemon did not come back at the new release.".to_owned(),
        fix: "Open Activity Monitor and quit any humata daemon, or run \
              `launchctl kickstart -k gui/$(id -u)/app.humata2.humatad2`, then click Migrate again. \
              If that doesn't help, quit Humata and open it again."
            .to_owned(),
    }
}

/// The app's own item for an old local daemon, waiting.
fn own_request(old: &OldLocalDaemon) -> Request {
    Request {
        id: RequestId::new(OWN_ID),
        session: SessionId::new(""),
        channel: ChannelId::new(""),
        kind: RequestKind::Migrate,
        title: "Migrate the daemon on This Mac".to_owned(),
        reason: String::new(),
        preview: Preview::Migrate(MigrateCard {
            thing: MigrateThing::Daemon,
            machine: old.machine.clone(),
            from: Release {
                version: old.running.clone(),
                bundle: String::new(),
            },
            to: old.staged.clone(),
            changes: vec!["Daemon".to_owned()],
            keeps_running: Vec::new(),
            phase: MigratePhase::Waiting,
            waiting_sessions: Vec::new(),
            migrated: None,
            steps: Vec::new(),
            holders: Vec::new(),
            moved: None,
        }),
        doing: String::new(),
        rule: String::new(),
        undoable: false,
        asked: "now".to_owned(),
        answer: None,
        operation: String::new(),
        steps: Vec::new(),
        why_you: String::new(),
        asked_by: None,
        undo_note: None,
        choices: Vec::new(),
        on_screen: None,
        dismiss_only: false,
        facts: Vec::new(),
        affects: Vec::new(),
        undo_text: String::new(),
        working_on: String::new(),
        asked_why: None,
        rule_label: String::new(),
        to_do: None,
    }
}

/// How the app's own migrate failed, from the error it got.
fn failure(error: &DaemonError) -> MigrateFailure {
    MigrateFailure {
        message: error.to_string(),
        fix: "Click Migrate again. If it fails again, quit Humata and open it again."
            .to_owned(),
    }
}

impl Reducer for Inbox {
    type State = InboxState;
    type Action = InboxAction;

    fn reduce(&self, state: &mut InboxState, action: InboxAction) -> Effect<InboxAction> {
        match action {
            InboxAction::Show(workspace) => {
                state.workspace = workspace;
                state.select_visible();
                return self.auto_migrate_local_if_needed(state);
            }
            InboxAction::SetFilter(filter) => {
                state.filter = filter;
                state.select_visible();
            }
            InboxAction::ToggleAnswered => {
                state.show_answered = !state.show_answered;
                state.answered_only = false;
                state.select_visible();
            }
            InboxAction::ToggleHistory => {
                state.show_answered = !state.show_answered;
                state.answered_only = state.show_answered;
                state.select_visible();
            }
            InboxAction::ChangeRule => {
                return Effect::send(InboxAction::Delegate(Delegate::OpenAgentAccess));
            }
            InboxAction::AskWhy(id) => {
                let askable = state
                    .workspace
                    .requests
                    .iter()
                    .any(|request| request.id == id && request.answer.is_none() && request.kind != RequestKind::Migrate);
                if !askable || !state.asking.insert(id.clone()) {
                    return Effect::none();
                }
                let daemon = self.clients.daemon.clone();
                return Effect::future(async move {
                    let result = daemon.ask_why(&id).await;
                    InboxAction::AskedWhy(id, result)
                });
            }
            InboxAction::AskedWhy(id, result) => {
                state.asking.remove(&id);
                match result {
                    Ok(request) => {
                        let found = Arc::make_mut(&mut state.workspace)
                            .requests
                            .iter_mut()
                            .find(|found| found.id == id);
                        if let Some(found) = found {
                            *found = request;
                        }
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
            InboxAction::SetHistory(history) => {
                state.show_answered = history != History::Waiting;
                state.answered_only = history == History::Answered;
                state.select_visible();
            }
            InboxAction::Select(id) => state.selected = Some(id),
            InboxAction::Migrate(id) => return self.migrate(state, id),
            InboxAction::OldLocalDaemon(old) => {
                match old {
                    Some(old) => {
                        if state.old_local.as_ref() != Some(&old) {
                            state.migrating.remove(&RequestId::new(OWN_ID));
                            state.own = Some(own_request(&old));
                            state.old_local = Some(old);
                        }
                    }
                    None => {
                        let own = RequestId::new(OWN_ID);
                        let was_migrating = state.own_migrated();
                        let migrated = state
                            .own
                            .as_ref()
                            .is_some_and(|own| own.answer == Some(Answer::Approved));
                        state.migrating.remove(&own);
                        state.old_local = None;
                        if !migrated {
                            state.own = None;
                        }
                        state.select_visible();
                        if was_migrating {
                            return Effect::merge([Effect::cancel(BACK_WAIT), Effect::cancel(LATE_WAIT)]);
                        }
                        return Effect::none();
                    }
                }
                state.select_visible();
                return self.auto_migrate_local_if_needed(state);
            }
            InboxAction::EnableAutoMigrateLocal => {
                state.auto_migrate_local = true;
                return self.auto_migrate_local_if_needed(state);
            }
            InboxAction::ToggleChanges(id) => {
                let foldable = state
                    .find(&id)
                    .and_then(Request::migrate)
                    .is_some_and(|card| card.changes.len() > FOLDED_LINES);
                if foldable && !state.unfolded.remove(&id) {
                    state.unfolded.insert(id);
                }
            }
            InboxAction::SomewhereElse(id) => {
                if let Some(card) = state.find(&id).and_then(Request::copy) {
                    return Effect::send(InboxAction::Delegate(Delegate::OpenDestination(
                        card.to.clone(),
                    )));
                }
            }
            InboxAction::ToggleDetails(id) => {
                if state.find(&id).and_then(Request::copy).is_some() && !state.details.remove(&id) {
                    state.details.insert(id);
                }
            }
            InboxAction::CopyPath(path) => {
                self.clients.clipboard.copy(&path);
            }
            InboxAction::Reveal(machine, path) => return self.reveal_path(state, &machine, path),
            InboxAction::RevealFailed(message) => state.error = Some(message),
            InboxAction::LocalDaemonBack => {
                if state.own_migrated() {
                    return Effect::merge([
                        Effect::cancel(LATE_WAIT),
                        Effect::send(InboxAction::Delegate(Delegate::Reconnect)),
                    ]);
                }
            }
            InboxAction::LocalDaemonLate => {
                let own = RequestId::new(OWN_ID);
                if state.migrating.remove(&own) {
                    state.set_phase(&own, MigratePhase::Failed(late_failure()));
                    return Effect::merge([Effect::cancel(BACK_WAIT), Effect::cancel(LATE_WAIT)]);
                }
            }
            InboxAction::Answer(id, _)
            | InboxAction::ToggleAlways(id)
            | InboxAction::Choose(id, _)
            | InboxAction::Dismiss(id)
            | InboxAction::Always(id)
            | InboxAction::StartReply(id)
                if state.is_migrate(&id) => {}
            InboxAction::Answer(id, answer) => {
                if let Some(request) = state.find(&id) {
                    if matches!(answer, Answer::AllowFor30Min)
                        && !(request.kind == RequestKind::BrowserRead
                            && request.browser().is_some_and(|preview| preview.supports_30m_grant))
                    {
                        return Effect::none();
                    }
                    if matches!(answer, Answer::Always { .. })
                        && matches!(
                            request.kind,
                            RequestKind::BrowserRead | RequestKind::BrowserWrite
                        )
                    {
                        return Effect::none();
                    }
                }
                let answer = match answer {
                    Answer::Approved if state.always.contains(&id) => {
                        match state.workspace.requests.iter().find(|r| r.id == id) {
                            Some(request)
                                if !request.rule.is_empty()
                                    && !matches!(
                                        request.kind,
                                        RequestKind::BrowserRead | RequestKind::BrowserWrite
                                    ) =>
                            {
                                Answer::Always {
                                    rule: request.rule.clone(),
                                }
                            }
                            _ => Answer::Approved,
                        }
                    }
                    other => other,
                };
                return self.answer(state, id, answer);
            }
            InboxAction::ApproveAll => {
                let ids: Vec<RequestId> = state
                    .bulk_approvable()
                    .into_iter()
                    .map(|request| request.id.clone())
                    .collect();
                let effects: Vec<Effect<InboxAction>> = ids
                    .into_iter()
                    .map(|id| self.answer(state, id, Answer::Approved))
                    .collect();
                return Effect::merge(effects);
            }
            InboxAction::DenyAll => {
                let ids: Vec<RequestId> = state
                    .bulk_deniable()
                    .into_iter()
                    .map(|request| request.id.clone())
                    .collect();
                let effects: Vec<Effect<InboxAction>> = ids
                    .into_iter()
                    .map(|id| self.answer(state, id, Answer::Denied { reply: None }))
                    .collect();
                return Effect::merge(effects);
            }
            InboxAction::ToggleAlways(id) => {
                let grantable = state.workspace.requests.iter().any(|r| {
                    r.id == id
                        && !r.rule.is_empty()
                        && r.answer.is_none()
                        && !matches!(r.kind, RequestKind::BrowserRead | RequestKind::BrowserWrite)
                });
                if grantable && !state.always.remove(&id) {
                    state.always.insert(id);
                }
            }
            InboxAction::Choose(id, text) => {
                return self.answer(state, id, Answer::Replied { text });
            }
            InboxAction::Dismiss(id) => {
                return self.answer(state, id, Answer::Denied { reply: None });
            }
            InboxAction::OpenRequest(id) => {
                if let Some(request) = state.workspace.requests.iter().find(|r| r.id == id) {
                    return Effect::send(InboxAction::Delegate(Delegate::OpenRequest {
                        session: request.session.clone(),
                        request: id,
                    }));
                }
            }
            InboxAction::Always(id) => {
                if state.find(&id).is_some_and(|r| {
                    matches!(r.kind, RequestKind::BrowserRead | RequestKind::BrowserWrite)
                }) {
                    return Effect::none();
                }
                let rule = state
                    .workspace
                    .requests
                    .iter()
                    .find(|request| request.id == id)
                    .map(|request| request.rule.clone())
                    .unwrap_or_default();
                return self.answer(state, id, Answer::Always { rule });
            }
            InboxAction::StartReply(id) => state.reply = Some((id, String::new())),
            InboxAction::ReplyDraft(id, text) => state.reply = Some((id, text)),
            InboxAction::ReplyText(text) => {
                if let Some(reply) = &mut state.reply {
                    reply.1 = text;
                }
            }
            InboxAction::SendReply => {
                if let Some((id, text)) = state.reply.take() {
                    let text = text.trim().to_owned();
                    let question = state.workspace.requests.iter().any(|request| {
                        request.id == id
                            && request.kind == humata2_clients::model::RequestKind::Question
                    });
                    let answer = if question {
                        Answer::Replied { text }
                    } else {
                        Answer::Denied {
                            reply: (!text.is_empty()).then_some(text),
                        }
                    };
                    return self.answer(state, id, answer);
                }
            }
            InboxAction::CancelReply => state.reply = None,
            InboxAction::Answered(id, result) => {
                state.pending.remove(&id);
                state.always.remove(&id);
                let migrate = state.is_migrate(&id);
                let own = state.is_own(&id);
                // The app's own item stays Migrating until This Mac's daemon
                // answers at the staged release, or the wait runs out.
                if own && result.is_ok() {
                    return self.wait_for_local_daemon(state);
                }
                state.migrating.remove(&id);
                match result {
                    Err(error) => {
                        if migrate {
                            state.set_phase(&id, MigratePhase::Failed(failure(&error)));
                        }
                        state.error = Some(error.to_string());
                    }
                    // The daemon took it: the item is still pending, so it
                    // stays shown and selected, migrating, until the daemon
                    // says how it ended.
                    Ok(()) if migrate => state.set_phase(&id, MigratePhase::Migrating),
                    Ok(()) => {
                        if state.selected.as_ref() == Some(&id) && !state.show_answered {
                            state.selected = None;
                        }
                    }
                }
            }
            InboxAction::OpenRequester => {
                if let Some(request) = state.selection() {
                    return Effect::send(InboxAction::Delegate(Delegate::OpenSession(
                        request.session.clone(),
                    )));
                }
            }
            InboxAction::Delegate(_) => {}
        }
        Effect::none()
    }
}
