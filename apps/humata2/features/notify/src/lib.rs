//! Pure desktop notification rules (NOTIFY, D889).
//!
//! Decides whether a live state transition should post a [`DesktopNotice`],
//! suppresses notifications while the user is already looking at that item,
//! deduplicates active episodes, collapses rapid bursts per session and per
//! channel, and computes notification identifiers to withdraw when items are
//! resolved or navigated to.
//!
//! Zero GPUI and zero OS dependencies.

use std::collections::{BTreeMap, BTreeSet};

use humata2_clients::model::{ChannelId, ItemId, MachineId, RequestId, RequestKind, SessionId};
use humata2_clients::{DesktopNotice, NoticeSound, NotifyTarget};
use serde::{Deserialize, Serialize};

/// Default display duration for an in-app Deck notification toast, in seconds.
pub const DECK_TOAST_DURATION_SECS: u64 = 8;

fn default_true() -> bool {
    true
}

/// Logs a notification gate decision to stderr unless `HUMATA2_NOTIFY_TRACE=0`.
pub fn trace_gate(gate: &str, outcome: &str, detail: &str) {
    if std::env::var("HUMATA2_NOTIFY_TRACE").ok().as_deref() == Some("0") {
        return;
    }
    eprintln!("[humata2::notify] gate={gate} outcome={outcome} {detail}");
}

/// Machine-readable outcome of evaluating a notification event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateOutcome {
    /// Posted as an OS desktop notification.
    Posted,
    /// Routed to an in-app Deck toast because the app is frontmost on another view.
    FrontmostDeckToast,
    /// Routed to an in-app Deck toast because OS notification permission is unavailable.
    PermissionFallbackToast,
    /// Suppressed because the master switch is off and no in-app toast channel applies.
    SuppressedMasterOff,
    /// Suppressed because the event's category toggle is off.
    SuppressedCategoryOff,
    /// Suppressed because the request is already answered.
    SuppressedAnswered,
    /// Suppressed because `board.*` operations are handled by `TodoAsk`.
    SuppressedBoardOp,
    /// Suppressed because the user is already viewing the exact item.
    SuppressedExactItem,
    /// Suppressed because the request or episode is already active.
    SuppressedDuplicate,
    /// Suppressed because the session already has a pending Agent-inbox request.
    SuppressedHasPendingInbox,
    /// Suppressed because the process exited with code 0.
    SuppressedZeroExit,
    /// Suppressed because the session is already covered by `resume_failed`.
    SuppressedAlreadyResumeFailed,
    /// Suppressed because `RestartFailed` had zero affected sessions.
    SuppressedEmptyRestart,
    /// Suppressed because a restart notice for this machine was posted within cooldown.
    SuppressedRestartCooldown,
    /// Suppressed because no delivery channel (OS or in-app toast) is enabled.
    SuppressedNoChannel,
}

impl GateOutcome {
    /// Returns a stable snake_case identifier for logging.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Posted => "posted",
            Self::FrontmostDeckToast => "frontmost_deck_toast",
            Self::PermissionFallbackToast => "permission_fallback_toast",
            Self::SuppressedMasterOff => "suppressed_master_off",
            Self::SuppressedCategoryOff => "suppressed_category_off",
            Self::SuppressedAnswered => "suppressed_answered",
            Self::SuppressedBoardOp => "suppressed_board_op",
            Self::SuppressedExactItem => "suppressed_exact_item",
            Self::SuppressedDuplicate => "suppressed_duplicate",
            Self::SuppressedHasPendingInbox => "suppressed_has_pending_inbox",
            Self::SuppressedZeroExit => "suppressed_zero_exit",
            Self::SuppressedAlreadyResumeFailed => "suppressed_already_resume_failed",
            Self::SuppressedEmptyRestart => "suppressed_empty_restart",
            Self::SuppressedRestartCooldown => "suppressed_restart_cooldown",
            Self::SuppressedNoChannel => "suppressed_no_channel",
        }
    }
}

/// Structured trace record for a single notification evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyTrace {
    /// Gate that made the final decision.
    pub gate: &'static str,
    /// Outcome of the gate.
    pub outcome: GateOutcome,
    /// Human-readable detail string for diagnostics.
    pub detail: String,
    /// Sound chosen for playback, if any.
    pub sound: Option<NoticeSound>,
    /// Reason sound was suppressed, if any.
    pub sound_suppressed: Option<&'static str>,
}

/// OS notification permission status (macOS `UNAuthorizationStatus` + unbundled).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionStatus {
    /// Permission status has not been queried or is unavailable on this platform.
    #[default]
    Unknown,
    /// User authorized OS notifications.
    Authorized,
    /// User denied OS notifications in System Settings.
    Denied,
    /// User has not yet been prompted for OS notification permission.
    NotDetermined,
    /// Provisional (quiet) OS notification authorization.
    Provisional,
    /// Ephemeral OS notification authorization.
    Ephemeral,
    /// Running outside a signed `.app` bundle; OS notifications are unavailable.
    Unbundled,
}

impl PermissionStatus {
    /// Whether the OS permits posting desktop notifications.
    #[must_use]
    pub const fn allows_os_delivery(self) -> bool {
        matches!(self, Self::Authorized | Self::Provisional | Self::Ephemeral)
    }

    /// Whether the UI should show a permission warning banner/toast.
    #[must_use]
    pub const fn needs_warning(self) -> bool {
        matches!(self, Self::Denied | Self::Unbundled)
    }

    /// Short human-readable label for settings UI.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Authorized => "Allowed",
            Self::Denied => "Denied",
            Self::NotDetermined => "Not requested",
            Self::Provisional => "Provisional",
            Self::Ephemeral => "Ephemeral",
            Self::Unbundled => "Unavailable (unbundled)",
        }
    }

    /// Actionable explanation for settings UI and diagnostics.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::Unknown => "Notification permission status has not been checked yet.",
            Self::Authorized => "macOS notifications are allowed for Humata.",
            Self::Denied => {
                "Notifications are disabled in System Settings. Enable them for Humata to receive desktop alerts."
            }
            Self::NotDetermined => "macOS has not asked for notification permission yet.",
            Self::Provisional => "macOS notifications are delivered quietly to Notification Center.",
            Self::Ephemeral => "macOS notifications are temporarily authorized.",
            Self::Unbundled => {
                "Running outside an app bundle; OS notifications are unavailable and in-app toasts are used instead."
            }
        }
    }
}

/// Visual kind of an in-app Deck toast.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeckToastKind {
    /// Standard notification toast shown while Humata is frontmost.
    Notice,
    /// Fallback toast shown when OS notification permission is denied or unbundled.
    PermissionWarning,
}

/// An in-app notification toast rendered on the Deck.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckToast {
    /// Stable notification identifier (matches the corresponding `DesktopNotice::id`).
    pub id: String,
    /// Toast heading.
    pub title: String,
    /// Toast body text.
    pub body: String,
    /// Navigation target when clicked.
    pub target: NotifyTarget,
    /// Auto-dismiss duration in seconds.
    pub duration_secs: u64,
    /// Whether hovering pauses the auto-dismiss countdown.
    pub pause_on_hover: bool,
    /// Visual kind of the toast.
    pub kind: DeckToastKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveDeckToast {
    toast: DeckToast,
    remaining_secs: u64,
    last_tick_secs: u64,
    hovered: bool,
}

/// Single-slot in-app notification toast queue with hover-pause and expiry.
///
/// Separate from Undo toasts: pushing a notification toast never evicts an Undo toast.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeckToastQueue {
    active: Option<ActiveDeckToast>,
}

impl DeckToastQueue {
    /// Notification toasts never evict Undo toasts.
    #[must_use]
    pub const fn can_evict_undo() -> bool {
        false
    }

    /// Replaces the current notification toast with `toast`, starting its countdown at `now_secs`.
    pub fn push(&mut self, toast: DeckToast, now_secs: u64) {
        let remaining_secs = toast.duration_secs;
        self.active = Some(ActiveDeckToast {
            toast,
            remaining_secs,
            last_tick_secs: now_secs,
            hovered: false,
        });
    }

    /// Pauses or resumes the active toast's countdown when hovered/unhovered.
    pub fn set_hovered(&mut self, hovered: bool, now_secs: u64) {
        let Some(entry) = self.active.as_mut() else {
            return;
        };
        if !entry.toast.pause_on_hover {
            entry.hovered = hovered;
            return;
        }
        if hovered && !entry.hovered {
            let elapsed = now_secs.saturating_sub(entry.last_tick_secs);
            entry.remaining_secs = entry.remaining_secs.saturating_sub(elapsed);
            entry.last_tick_secs = now_secs;
            entry.hovered = true;
        } else if !hovered && entry.hovered {
            entry.last_tick_secs = now_secs;
            entry.hovered = false;
        }
    }

    /// Expires the active toast if its countdown has elapsed at `now_secs`.
    ///
    /// Returns `true` if a toast was expired and removed.
    pub fn expire(&mut self, now_secs: u64) -> bool {
        let Some(entry) = self.active.as_ref() else {
            return false;
        };
        if entry.hovered && entry.toast.pause_on_hover {
            return false;
        }
        let elapsed = now_secs.saturating_sub(entry.last_tick_secs);
        if elapsed >= entry.remaining_secs {
            self.active = None;
            return true;
        }
        false
    }

    /// Dismisses the active toast, if any.
    pub fn dismiss(&mut self) {
        self.active = None;
    }

    /// Clears the active toast if its `id` matches. Returns `true` if cleared.
    pub fn clear_id(&mut self, id: &str) -> bool {
        if self.active.as_ref().is_some_and(|entry| entry.toast.id == id) {
            self.active = None;
            return true;
        }
        false
    }

    /// Returns the currently visible notification toast, if any.
    #[must_use]
    pub fn current(&self) -> Option<&DeckToast> {
        self.active.as_ref().map(|entry| &entry.toast)
    }
}

/// Full notification routing decision produced by [`evaluate_decision`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotifyDecision {
    /// OS desktop notification to post, if any.
    pub os_notice: Option<DesktopNotice>,
    /// In-app Deck toast to display, if any.
    pub deck_toast: Option<DeckToast>,
    /// In-app sound to play, if any.
    pub in_app_sound: Option<NoticeSound>,
    /// Structured trace of the gate decision.
    pub trace: NotifyTrace,
}

/// Builds the synthetic test notification triggered from Settings.
#[must_use]
pub fn build_test_notice(play_sound: bool) -> DesktopNotice {
    DesktopNotice {
        id: "humata.test".to_owned(),
        title: "Humata test notification".to_owned(),
        body: "Notifications are working.".to_owned(),
        target: NotifyTarget::Inbox,
        sound: play_sound.then_some(NoticeSound::Inbox),
    }
}

/// Computes the Dock badge count for blocked Agent-inbox asks.
///
/// Returns `None` when `blocked_inbox_asks == 0`, `Some(n)` otherwise.
#[must_use]
pub const fn dock_badge_count(blocked_inbox_asks: usize) -> Option<usize> {
    if blocked_inbox_asks == 0 {
        None
    } else {
        Some(blocked_inbox_asks)
    }
}

/// User configuration for desktop notifications (D889).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[allow(clippy::struct_excessive_bools)]
pub struct NotifyConfig {
    /// Master switch. Off by default.
    pub enabled: bool,
    /// Pending Agent-inbox permission requests (`Command`, `FileEdit`, `Network`,
    /// `Merge`, `Destroy`).
    pub inbox_requests: bool,
    /// Agent questions (`RequestKind::Question` and terminal-only `NeedsYou`
    /// prompts).
    pub agent_questions: bool,
    /// Failed resumes, abnormal exits, and host restart failures.
    pub failed_resumes: bool,
    /// Finished agent turns (`Turn::Ended`). Off by default per D889.
    pub finished_turns: bool,
    /// Migrate items (`RequestKind::Migrate`). Off by default per D889.
    pub migrate_items: bool,
    /// Suppress notifications while the app is frontmost and already viewing
    /// that item.
    pub quiet_when_frontmost: bool,
    /// Cooldown window in seconds for per-session and per-channel burst
    /// collapsing.
    pub cooldown_secs: u64,
    /// Play a bundled sound with each notification (NOTIFY2). Off by default.
    pub play_sound: bool,
    /// Show in-app Deck toasts when frontmost or when OS permission is unavailable.
    #[serde(default = "default_true")]
    pub in_app_toasts: bool,
}

impl Default for NotifyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            inbox_requests: true,
            agent_questions: true,
            failed_resumes: true,
            finished_turns: false,
            migrate_items: false,
            quiet_when_frontmost: true,
            cooldown_secs: 30,
            play_sound: false,
            in_app_toasts: true,
        }
    }
}

/// Where the user is looking right now, used for frontmost-on-item suppression
/// (D889).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrontmostContext {
    /// Whether the Humata 2 window is currently frontmost/active.
    pub app_active: bool,
    /// Whether the user is viewing the global Agent inbox (`Route::Inbox`).
    pub viewing_inbox: bool,
    /// Whether the user is viewing Home (`Route::Home`).
    pub viewing_home: bool,
    /// The session currently open (`Route::Session(s)`), if any.
    pub viewing_session: Option<SessionId>,
    /// Whether the open session is currently showing its Inbox panel
    /// (`Panel::Inbox`).
    pub viewing_session_inbox: bool,
    /// The channel whose Inbox tab is currently open (`Route::Channel(c)` on
    /// its `Inbox` tab), if any.
    pub viewing_channel_inbox: Option<ChannelId>,
    /// Whether the user is viewing the To-do (`Route::Todo`).
    pub viewing_todo: bool,
    /// The exact Agent-inbox request currently open in front of the user, if any.
    pub viewing_request: Option<RequestId>,
}

/// A live state transition that may produce a desktop notification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotifyEvent {
    /// A newly arrived Agent-inbox request.
    InboxRequest {
        /// The request's identifier.
        request_id: RequestId,
        /// The session asking.
        session_id: SessionId,
        /// The session's display title.
        session_title: String,
        /// The session's channel, or `None` when the session is on Desk.
        channel_id: Option<ChannelId>,
        /// The channel's name without `#`, or `None` for Desk.
        channel_name: Option<String>,
        /// What the request asks for.
        kind: RequestKind,
        /// Dotted operation id (e.g. `"board.item.move"`).
        operation: String,
        /// Whether `request.answer.is_some()`. Answered requests never notify.
        answered: bool,
        /// Preformatted notification title (e.g. from `humata2_inbox::notification`).
        title: String,
        /// Preformatted notification body.
        body: String,
    },
    /// A session transitioned into `Status::NeedsYou` with no pending
    /// Agent-inbox request (D845 terminal-only prompts).
    TerminalNeedsYou {
        /// The session needing attention.
        session_id: SessionId,
        /// The session's display title.
        session_title: String,
        /// The session's channel, or `None` for Desk.
        channel_id: Option<ChannelId>,
        /// The channel's name without `#`, or `None` for Desk.
        channel_name: Option<String>,
        /// The agent's display name (e.g. `"Claude"` or `"claude"`).
        agent_name: String,
        /// Whether the session already has a pending Agent-inbox request.
        has_pending_inbox_request: bool,
    },
    /// `session.resume_failed` transitioned from `None` to `Some(_)`.
    ResumeFailed {
        /// The session whose resume failed.
        session_id: SessionId,
        /// The session's display title.
        session_title: String,
        /// The session's channel, or `None` for Desk.
        channel_id: Option<ChannelId>,
        /// The channel's name without `#`, or `None` for Desk.
        channel_name: Option<String>,
        /// One-line explanation of why resume failed.
        reason: String,
    },
    /// A session transitioned into `Status::Exited { code }` with `code != 0`.
    AbnormalExit {
        /// The session that exited abnormally.
        session_id: SessionId,
        /// The session's display title.
        session_title: String,
        /// The session's channel, or `None` for Desk.
        channel_id: Option<ChannelId>,
        /// The channel's name without `#`, or `None` for Desk.
        channel_name: Option<String>,
        /// Process exit code (`0` never notifies).
        code: i32,
        /// Whether this session is already covered by `resume_failed`.
        already_resume_failed: bool,
    },
    /// `Event::Restart` arrived with non-empty `not_brought_back`.
    RestartFailed {
        /// The machine that restarted.
        machine_id: MachineId,
        /// The machine's display name.
        machine_name: String,
        /// The sessions that could not be brought back.
        sessions: Vec<SessionId>,
        /// Total count of items not brought back (at least `sessions.len()`).
        not_brought_back: usize,
    },
    /// A session finished its agent turn (`Turn::Ended`).
    FinishedTurn {
        /// The session whose turn ended.
        session_id: SessionId,
        /// The session's display title.
        session_title: String,
        /// The session's channel, or `None` for Desk.
        channel_id: Option<ChannelId>,
        /// The channel's name without `#`, or `None` for Desk.
        channel_name: Option<String>,
        /// The agent's display name.
        agent_name: String,
    },
    /// A Migrate item from Humata (`RequestKind::Migrate`).
    MigrateItem {
        /// The request's identifier.
        request_id: RequestId,
        /// The machine to migrate.
        machine_name: String,
        /// Notification title.
        title: String,
        /// Notification body.
        body: String,
        /// Whether `request.answer.is_some()`.
        answered: bool,
    },
    /// A To-do ask: the agent suggests a new item or asks about one. Posts
    /// under the master switch only.
    TodoAsk {
        /// The request's identifier.
        request_id: RequestId,
        /// The session asking.
        session_id: SessionId,
        /// The session's channel, or `None` for Desk.
        channel_id: Option<ChannelId>,
        /// The item it is about; None when it suggests a new item.
        item: Option<ItemId>,
        /// Whether `request.answer.is_some()`.
        answered: bool,
        /// Notification title.
        title: String,
        /// Notification body.
        body: String,
    },
}

/// Stable notification identifier for a To-do ask.
#[must_use]
pub fn todo_notice_id(request_id: &RequestId) -> String {
    format!("humata.todo.{}", request_id.as_str())
}

/// Stable notification identifier for a session.
#[must_use]
pub fn session_notice_id(session_id: &SessionId) -> String {
    format!("humata.session.{}", session_id.as_str())
}

/// Stable notification identifier for a channel burst (or Desk when `None`).
#[must_use]
pub fn channel_notice_id(channel_id: Option<&ChannelId>) -> String {
    match channel_id {
        Some(id) => format!("humata.channel.{}", id.as_str()),
        None => "humata.channel.desk".to_owned(),
    }
}

/// Stable notification identifier for a multi-session host restart summary.
#[must_use]
pub fn restart_notice_id(machine_id: &MachineId) -> String {
    format!("humata.restart.{}", machine_id.as_str())
}

/// Stable notification identifier for a Migrate item.
#[must_use]
pub fn migrate_notice_id(request_id: &RequestId) -> String {
    format!("humata.migrate.{}", request_id.as_str())
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum ChannelBucket {
    Channel(ChannelId),
    Desk,
}

impl ChannelBucket {
    fn from_option(channel_id: Option<ChannelId>) -> Self {
        match channel_id {
            Some(id) => Self::Channel(id),
            None => Self::Desk,
        }
    }

    fn notice_id(&self) -> String {
        match self {
            Self::Channel(id) => channel_notice_id(Some(id)),
            Self::Desk => channel_notice_id(None),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SessionEpisodeKind {
    TerminalNeedsYou,
    ResumeFailed,
    AbnormalExit,
    FinishedTurn,
    SingleRestartFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RequestOwner {
    Session(SessionId),
    Migrate(String),
    Todo(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SessionBurst {
    channel: Option<ChannelBucket>,
    requests: BTreeSet<RequestId>,
    episodes: BTreeSet<SessionEpisodeKind>,
    last_posted_secs: Option<u64>,
    burst_count: usize,
    has_live_notice: bool,
}

impl SessionBurst {
    fn is_empty(&self) -> bool {
        self.requests.is_empty() && self.episodes.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct ChannelBurst {
    recent_sessions: BTreeMap<SessionId, u64>,
    has_live_notice: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RestartBurst {
    last_posted_secs: u64,
    has_live_notice: bool,
}

/// Mutable deduplication and burst-collapsing state across live events.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BurstState {
    sessions: BTreeMap<SessionId, SessionBurst>,
    channels: BTreeMap<ChannelBucket, ChannelBurst>,
    request_index: BTreeMap<RequestId, RequestOwner>,
    restarts: BTreeMap<MachineId, RestartBurst>,
    last_sound_secs: Option<u64>,
}

fn resolve_sound(
    config: &NotifyConfig,
    burst: &mut BurstState,
    desired: NoticeSound,
    now_secs: u64,
) -> (Option<NoticeSound>, Option<&'static str>) {
    if !config.play_sound {
        return (None, Some("play_sound_off"));
    }
    let recent = burst
        .last_sound_secs
        .is_some_and(|last| now_secs.saturating_sub(last) < config.cooldown_secs);
    if recent {
        return (None, Some("sound_cooldown"));
    }
    burst.last_sound_secs = Some(now_secs);
    (Some(desired), None)
}

fn suppressed_decision(
    gate: &'static str,
    outcome: GateOutcome,
    detail: String,
    sound_suppressed: Option<&'static str>,
) -> NotifyDecision {
    trace_gate(gate, outcome.as_str(), &detail);
    NotifyDecision {
        os_notice: None,
        deck_toast: None,
        in_app_sound: None,
        trace: NotifyTrace {
            gate,
            outcome,
            detail,
            sound: None,
            sound_suppressed,
        },
    }
}

/// Evaluates `event` against `config`, `frontmost`, and `burst` at `now_secs`,
/// returning a [`DesktopNotice`] to post if the event should notify.
#[must_use]
pub fn evaluate(
    config: &NotifyConfig,
    frontmost: &FrontmostContext,
    burst: &mut BurstState,
    event: NotifyEvent,
    now_secs: u64,
) -> Option<DesktopNotice> {
    if !config.enabled {
        trace_gate(
            "master",
            GateOutcome::SuppressedMasterOff.as_str(),
            "enabled=false",
        );
        return None;
    }
    let is_todo = matches!(event, NotifyEvent::TodoAsk { .. });
    let mut notice = match evaluate_event_core(config, frontmost, burst, event, now_secs) {
        Ok(notice) => notice,
        Err((gate, outcome, detail)) => {
            trace_gate(gate, outcome.as_str(), &detail);
            return None;
        }
    };
    let desired = if is_todo {
        NoticeSound::Todo
    } else {
        NoticeSound::Inbox
    };
    let (sound, _) = resolve_sound(config, burst, desired, now_secs);
    notice.sound = sound;
    trace_gate(
        "delivery",
        GateOutcome::Posted.as_str(),
        &format!("id={} sound={:?}", notice.id, notice.sound),
    );
    Some(notice)
}

/// Evaluates `event` with full routing across OS notifications, in-app Deck
/// toasts, decoupled in-app sound, and structured gate tracing.
#[must_use]
pub fn evaluate_decision(
    config: &NotifyConfig,
    frontmost: &FrontmostContext,
    permission: PermissionStatus,
    burst: &mut BurstState,
    event: NotifyEvent,
    now_secs: u64,
) -> NotifyDecision {
    let frontmost_toast_route =
        frontmost.app_active && (config.quiet_when_frontmost || !config.enabled);

    if !config.enabled && !(frontmost.app_active && config.in_app_toasts) {
        return suppressed_decision(
            "master",
            GateOutcome::SuppressedMasterOff,
            "enabled=false".to_owned(),
            Some("master_off"),
        );
    }

    let is_todo = matches!(event, NotifyEvent::TodoAsk { .. });
    let mut notice = match evaluate_event_core(config, frontmost, burst, event, now_secs) {
        Ok(notice) => notice,
        Err((gate, outcome, detail)) => {
            let sound_suppressed = if outcome == GateOutcome::SuppressedExactItem {
                Some("exact_item")
            } else {
                Some(outcome.as_str())
            };
            return suppressed_decision(gate, outcome, detail, sound_suppressed);
        }
    };

    let desired = if is_todo {
        NoticeSound::Todo
    } else {
        NoticeSound::Inbox
    };

    if frontmost_toast_route {
        if !config.in_app_toasts {
            return suppressed_decision(
                "delivery",
                GateOutcome::SuppressedNoChannel,
                "frontmost=true in_app_toasts=false".to_owned(),
                Some("no_channel"),
            );
        }
        let (in_app_sound, sound_suppressed) = resolve_sound(config, burst, desired, now_secs);
        let detail = format!("id={} sound={in_app_sound:?}", notice.id);
        trace_gate(
            "delivery",
            GateOutcome::FrontmostDeckToast.as_str(),
            &detail,
        );
        return NotifyDecision {
            os_notice: None,
            deck_toast: Some(DeckToast {
                id: notice.id,
                title: notice.title,
                body: notice.body,
                target: notice.target,
                duration_secs: DECK_TOAST_DURATION_SECS,
                pause_on_hover: true,
                kind: DeckToastKind::Notice,
            }),
            in_app_sound,
            trace: NotifyTrace {
                gate: "delivery",
                outcome: GateOutcome::FrontmostDeckToast,
                detail,
                sound: in_app_sound,
                sound_suppressed,
            },
        };
    }

    if !permission.allows_os_delivery() && permission != PermissionStatus::Unknown {
        if !config.in_app_toasts {
            return suppressed_decision(
                "permission",
                GateOutcome::SuppressedNoChannel,
                format!("permission={permission:?} in_app_toasts=false"),
                Some("no_channel"),
            );
        }
        let (in_app_sound, sound_suppressed) = resolve_sound(config, burst, desired, now_secs);
        let detail = format!(
            "id={} permission={permission:?} sound={in_app_sound:?}",
            notice.id
        );
        trace_gate(
            "permission",
            GateOutcome::PermissionFallbackToast.as_str(),
            &detail,
        );
        return NotifyDecision {
            os_notice: None,
            deck_toast: Some(DeckToast {
                id: notice.id,
                title: notice.title,
                body: notice.body,
                target: notice.target,
                duration_secs: DECK_TOAST_DURATION_SECS,
                pause_on_hover: true,
                kind: DeckToastKind::PermissionWarning,
            }),
            in_app_sound,
            trace: NotifyTrace {
                gate: "permission",
                outcome: GateOutcome::PermissionFallbackToast,
                detail,
                sound: in_app_sound,
                sound_suppressed,
            },
        };
    }

    let (sound, sound_suppressed) = resolve_sound(config, burst, desired, now_secs);
    notice.sound = sound;
    let detail = format!("id={} sound={sound:?}", notice.id);
    trace_gate("delivery", GateOutcome::Posted.as_str(), &detail);
    NotifyDecision {
        os_notice: Some(notice),
        deck_toast: None,
        in_app_sound: sound,
        trace: NotifyTrace {
            gate: "delivery",
            outcome: GateOutcome::Posted,
            detail,
            sound,
            sound_suppressed,
        },
    }
}

fn evaluate_event_core(
    config: &NotifyConfig,
    frontmost: &FrontmostContext,
    burst: &mut BurstState,
    event: NotifyEvent,
    now_secs: u64,
) -> Result<DesktopNotice, (&'static str, GateOutcome, String)> {
    match event {
        NotifyEvent::InboxRequest {
            request_id,
            session_id,
            session_title,
            channel_id,
            channel_name,
            kind,
            operation,
            answered,
            title,
            body,
        } => {
            if answered {
                return Err((
                    "inbox_request",
                    GateOutcome::SuppressedAnswered,
                    format!("request={}", request_id.as_str()),
                ));
            }
            if operation.starts_with("board.") {
                return Err((
                    "inbox_request",
                    GateOutcome::SuppressedBoardOp,
                    format!("request={} op={operation}", request_id.as_str()),
                ));
            }
            let allowed = match kind {
                RequestKind::Migrate => config.migrate_items,
                RequestKind::Question => config.agent_questions,
                RequestKind::Command
                | RequestKind::FileEdit
                | RequestKind::Network
                | RequestKind::Merge
                | RequestKind::Destroy
                | RequestKind::BrowserRead
                | RequestKind::BrowserWrite => config.inbox_requests,
            };
            if !allowed {
                return Err((
                    "category",
                    GateOutcome::SuppressedCategoryOff,
                    format!("request={} kind={kind:?}", request_id.as_str()),
                ));
            }
            if is_suppressed_inbox(
                config,
                frontmost,
                &request_id,
                &session_id,
                channel_id.as_ref(),
            ) {
                return Err((
                    "frontmost",
                    GateOutcome::SuppressedExactItem,
                    format!("request={}", request_id.as_str()),
                ));
            }
            if burst.request_index.contains_key(&request_id) {
                return Err((
                    "dedup",
                    GateOutcome::SuppressedDuplicate,
                    format!("request={}", request_id.as_str()),
                ));
            }
            let target = NotifyTarget::Request {
                session: session_id.clone(),
                request: request_id.clone(),
            };
            let notice = record_session_event(
                config,
                burst,
                SessionEventMeta {
                    session_id: session_id.clone(),
                    session_title,
                    channel_id,
                    channel_name,
                    title,
                    body,
                    target,
                    dedup: SessionDedup::Request(request_id),
                },
                now_secs,
            );
            Ok(notice)
        }

        NotifyEvent::TerminalNeedsYou {
            session_id,
            session_title,
            channel_id,
            channel_name,
            agent_name,
            has_pending_inbox_request,
        } => {
            if !config.agent_questions {
                return Err((
                    "category",
                    GateOutcome::SuppressedCategoryOff,
                    format!("session={} event=TerminalNeedsYou", session_id.as_str()),
                ));
            }
            if has_pending_inbox_request {
                return Err((
                    "terminal_needs_you",
                    GateOutcome::SuppressedHasPendingInbox,
                    format!("session={}", session_id.as_str()),
                ));
            }
            if is_suppressed_session(config, frontmost, &session_id) {
                return Err((
                    "frontmost",
                    GateOutcome::SuppressedExactItem,
                    format!("session={}", session_id.as_str()),
                ));
            }
            if has_active_episode(burst, &session_id, SessionEpisodeKind::TerminalNeedsYou) {
                return Err((
                    "dedup",
                    GateOutcome::SuppressedDuplicate,
                    format!("session={} episode=TerminalNeedsYou", session_id.as_str()),
                ));
            }
            let title = format!("{agent_name} needs you in {session_title}");
            let body = "Open the session to answer.".to_owned();
            let target = NotifyTarget::Session(session_id.clone());
            Ok(record_session_event(
                config,
                burst,
                SessionEventMeta {
                    session_id,
                    session_title,
                    channel_id,
                    channel_name,
                    title,
                    body,
                    target,
                    dedup: SessionDedup::Episode(SessionEpisodeKind::TerminalNeedsYou),
                },
                now_secs,
            ))
        }

        NotifyEvent::ResumeFailed {
            session_id,
            session_title,
            channel_id,
            channel_name,
            reason,
        } => {
            if !config.failed_resumes {
                return Err((
                    "category",
                    GateOutcome::SuppressedCategoryOff,
                    format!("session={} event=ResumeFailed", session_id.as_str()),
                ));
            }
            if is_suppressed_session(config, frontmost, &session_id) {
                return Err((
                    "frontmost",
                    GateOutcome::SuppressedExactItem,
                    format!("session={}", session_id.as_str()),
                ));
            }
            if has_active_episode(burst, &session_id, SessionEpisodeKind::ResumeFailed) {
                return Err((
                    "dedup",
                    GateOutcome::SuppressedDuplicate,
                    format!("session={} episode=ResumeFailed", session_id.as_str()),
                ));
            }
            let title = format!("{session_title} couldn't resume");
            let body = if reason.trim().is_empty() {
                "Open the session to recover it.".to_owned()
            } else {
                reason
            };
            let target = NotifyTarget::Session(session_id.clone());
            Ok(record_session_event(
                config,
                burst,
                SessionEventMeta {
                    session_id,
                    session_title,
                    channel_id,
                    channel_name,
                    title,
                    body,
                    target,
                    dedup: SessionDedup::Episode(SessionEpisodeKind::ResumeFailed),
                },
                now_secs,
            ))
        }

        NotifyEvent::AbnormalExit {
            session_id,
            session_title,
            channel_id,
            channel_name,
            code,
            already_resume_failed,
        } => {
            if !config.failed_resumes {
                return Err((
                    "category",
                    GateOutcome::SuppressedCategoryOff,
                    format!("session={} event=AbnormalExit", session_id.as_str()),
                ));
            }
            if code == 0 {
                return Err((
                    "abnormal_exit",
                    GateOutcome::SuppressedZeroExit,
                    format!("session={}", session_id.as_str()),
                ));
            }
            if already_resume_failed {
                return Err((
                    "abnormal_exit",
                    GateOutcome::SuppressedAlreadyResumeFailed,
                    format!("session={}", session_id.as_str()),
                ));
            }
            if is_suppressed_session(config, frontmost, &session_id) {
                return Err((
                    "frontmost",
                    GateOutcome::SuppressedExactItem,
                    format!("session={}", session_id.as_str()),
                ));
            }
            if has_active_episode(burst, &session_id, SessionEpisodeKind::ResumeFailed)
                || has_active_episode(burst, &session_id, SessionEpisodeKind::AbnormalExit)
            {
                return Err((
                    "dedup",
                    GateOutcome::SuppressedDuplicate,
                    format!("session={} episode=AbnormalExit", session_id.as_str()),
                ));
            }
            let title = format!("{session_title} exited ({code})");
            let body = "Open the session to review or resume it.".to_owned();
            let target = NotifyTarget::Session(session_id.clone());
            Ok(record_session_event(
                config,
                burst,
                SessionEventMeta {
                    session_id,
                    session_title,
                    channel_id,
                    channel_name,
                    title,
                    body,
                    target,
                    dedup: SessionDedup::Episode(SessionEpisodeKind::AbnormalExit),
                },
                now_secs,
            ))
        }

        NotifyEvent::RestartFailed {
            machine_id,
            machine_name,
            sessions,
            not_brought_back,
        } => {
            if !config.failed_resumes {
                return Err((
                    "category",
                    GateOutcome::SuppressedCategoryOff,
                    format!("machine={} event=RestartFailed", machine_id.as_str()),
                ));
            }
            let count = not_brought_back.max(sessions.len());
            if count == 0 {
                return Err((
                    "restart_failed",
                    GateOutcome::SuppressedEmptyRestart,
                    format!("machine={}", machine_id.as_str()),
                ));
            }
            if count == 1 && sessions.len() == 1 {
                let Some(session_id) = sessions.into_iter().next() else {
                    return Err((
                        "restart_failed",
                        GateOutcome::SuppressedEmptyRestart,
                        format!("machine={}", machine_id.as_str()),
                    ));
                };
                if is_suppressed_session(config, frontmost, &session_id) {
                    return Err((
                        "frontmost",
                        GateOutcome::SuppressedExactItem,
                        format!("session={}", session_id.as_str()),
                    ));
                }
                if has_active_episode(burst, &session_id, SessionEpisodeKind::ResumeFailed)
                    || has_active_episode(
                        burst,
                        &session_id,
                        SessionEpisodeKind::SingleRestartFailed,
                    )
                {
                    return Err((
                        "dedup",
                        GateOutcome::SuppressedDuplicate,
                        format!(
                            "session={} episode=SingleRestartFailed",
                            session_id.as_str()
                        ),
                    ));
                }
                let entry = burst.sessions.entry(session_id.clone()).or_default();
                entry.episodes.insert(SessionEpisodeKind::SingleRestartFailed);
                entry.last_posted_secs = Some(now_secs);
                entry.burst_count = entry.burst_count.saturating_add(1);
                entry.has_live_notice = true;
                return Ok(DesktopNotice {
                    id: session_notice_id(&session_id),
                    title: format!("{machine_name} restarted: 1 session couldn't resume"),
                    body: "Open the session to recover it.".to_owned(),
                    target: NotifyTarget::Session(session_id),
                    sound: None,
                });
            }

            if config.quiet_when_frontmost && frontmost.app_active && frontmost.viewing_home {
                return Err((
                    "frontmost",
                    GateOutcome::SuppressedExactItem,
                    format!("machine={} viewing_home=true", machine_id.as_str()),
                ));
            }
            if let Some(prev) = burst.restarts.get(&machine_id)
                && now_secs.saturating_sub(prev.last_posted_secs) < config.cooldown_secs
            {
                return Err((
                    "restart_failed",
                    GateOutcome::SuppressedRestartCooldown,
                    format!("machine={}", machine_id.as_str()),
                ));
            }
            burst.restarts.insert(
                machine_id.clone(),
                RestartBurst {
                    last_posted_secs: now_secs,
                    has_live_notice: true,
                },
            );
            let noun = if count == 1 { "session" } else { "sessions" };
            Ok(DesktopNotice {
                id: restart_notice_id(&machine_id),
                title: format!("{machine_name} restarted: {count} {noun} couldn't resume"),
                body: "Open Home to review and recover them.".to_owned(),
                target: NotifyTarget::Home,
                sound: None,
            })
        }

        NotifyEvent::FinishedTurn {
            session_id,
            session_title,
            channel_id,
            channel_name,
            agent_name,
        } => {
            if !config.finished_turns {
                return Err((
                    "category",
                    GateOutcome::SuppressedCategoryOff,
                    format!("session={} event=FinishedTurn", session_id.as_str()),
                ));
            }
            if is_suppressed_session(config, frontmost, &session_id) {
                return Err((
                    "frontmost",
                    GateOutcome::SuppressedExactItem,
                    format!("session={}", session_id.as_str()),
                ));
            }
            if has_active_episode(burst, &session_id, SessionEpisodeKind::FinishedTurn) {
                return Err((
                    "dedup",
                    GateOutcome::SuppressedDuplicate,
                    format!("session={} episode=FinishedTurn", session_id.as_str()),
                ));
            }
            let title = format!("{agent_name} finished in {session_title}");
            let body = "Open the session to review.".to_owned();
            let target = NotifyTarget::Session(session_id.clone());
            Ok(record_session_event(
                config,
                burst,
                SessionEventMeta {
                    session_id,
                    session_title,
                    channel_id,
                    channel_name,
                    title,
                    body,
                    target,
                    dedup: SessionDedup::Episode(SessionEpisodeKind::FinishedTurn),
                },
                now_secs,
            ))
        }

        NotifyEvent::MigrateItem {
            request_id,
            machine_name,
            title,
            body,
            answered,
        } => {
            if !config.migrate_items {
                return Err((
                    "category",
                    GateOutcome::SuppressedCategoryOff,
                    format!("request={} event=MigrateItem", request_id.as_str()),
                ));
            }
            if answered {
                return Err((
                    "migrate_item",
                    GateOutcome::SuppressedAnswered,
                    format!("request={}", request_id.as_str()),
                ));
            }
            if config.quiet_when_frontmost && frontmost.app_active && frontmost.viewing_inbox {
                return Err((
                    "frontmost",
                    GateOutcome::SuppressedExactItem,
                    format!("request={} viewing_inbox=true", request_id.as_str()),
                ));
            }
            if burst.request_index.contains_key(&request_id) {
                return Err((
                    "dedup",
                    GateOutcome::SuppressedDuplicate,
                    format!("request={}", request_id.as_str()),
                ));
            }
            let id = migrate_notice_id(&request_id);
            burst
                .request_index
                .insert(request_id, RequestOwner::Migrate(id.clone()));
            let title = if title.trim().is_empty() {
                format!("Migrate available for {machine_name}")
            } else {
                title
            };
            Ok(DesktopNotice {
                id,
                title,
                body,
                target: NotifyTarget::Inbox,
                sound: None,
            })
        }

        NotifyEvent::TodoAsk {
            request_id,
            session_id: _,
            channel_id: _,
            item,
            answered,
            title,
            body,
        } => {
            if answered {
                return Err((
                    "todo_ask",
                    GateOutcome::SuppressedAnswered,
                    format!("request={}", request_id.as_str()),
                ));
            }
            if config.quiet_when_frontmost && frontmost.app_active && frontmost.viewing_todo {
                return Err((
                    "frontmost",
                    GateOutcome::SuppressedExactItem,
                    format!("request={} viewing_todo=true", request_id.as_str()),
                ));
            }
            if burst.request_index.contains_key(&request_id) {
                return Err((
                    "dedup",
                    GateOutcome::SuppressedDuplicate,
                    format!("request={}", request_id.as_str()),
                ));
            }
            let id = todo_notice_id(&request_id);
            burst
                .request_index
                .insert(request_id, RequestOwner::Todo(id.clone()));
            Ok(DesktopNotice {
                id,
                title,
                body,
                target: NotifyTarget::Todo { item },
                sound: None,
            })
        }
    }
}

/// Clears all active notification state and cooldown for `session_id` (e.g.
/// when the user navigates to the session or all its items resolve).
///
/// Returns notification identifiers that should be withdrawn.
#[must_use]
pub fn clear_session(burst: &mut BurstState, session_id: &SessionId) -> Vec<String> {
    let mut withdraw = Vec::new();
    let Some(entry) = burst.sessions.remove(session_id) else {
        return withdraw;
    };

    for request_id in &entry.requests {
        burst.request_index.remove(request_id);
    }

    if entry.has_live_notice {
        withdraw.push(session_notice_id(session_id));
    }

    if let Some(bucket) = entry.channel {
        cleanup_channel_after_session_clear(burst, &bucket, session_id, &mut withdraw);
    }

    withdraw
}

/// Clears a single resolved request and resets its session's cooldown.
///
/// If resolving `request_id` leaves the session with no remaining pending
/// items, returns the session's notification identifier (and the channel's
/// collapsed identifier if no sessions in that channel remain) to withdraw.
#[must_use]
pub fn clear_request(burst: &mut BurstState, request_id: &RequestId) -> Vec<String> {
    let mut withdraw = Vec::new();
    let Some(owner) = burst.request_index.remove(request_id) else {
        return withdraw;
    };

    match owner {
        RequestOwner::Migrate(notice_id) | RequestOwner::Todo(notice_id) => {
            withdraw.push(notice_id);
        }
        RequestOwner::Session(session_id) => {
            let mut remove_session = false;
            if let Some(entry) = burst.sessions.get_mut(&session_id) {
                entry.requests.remove(request_id);
                // Answering resets the session's cooldown window (D889).
                entry.last_posted_secs = None;
                entry.burst_count = 0;
                if entry.is_empty() {
                    remove_session = true;
                }
            }
            if remove_session && let Some(entry) = burst.sessions.remove(&session_id) {
                if entry.has_live_notice {
                    withdraw.push(session_notice_id(&session_id));
                }
                if let Some(bucket) = entry.channel {
                    cleanup_channel_after_session_clear(
                        burst,
                        &bucket,
                        &session_id,
                        &mut withdraw,
                    );
                }
            }
        }
    }

    withdraw
}

/// Clears a host restart summary notification (e.g. when the user opens Home).
///
/// Returns the restart notification identifier if one was live.
#[must_use]
pub fn clear_restart(burst: &mut BurstState, machine_id: &MachineId) -> Vec<String> {
    match burst.restarts.remove(machine_id) {
        Some(entry) if entry.has_live_notice => vec![restart_notice_id(machine_id)],
        _ => Vec::new(),
    }
}

fn cleanup_channel_after_session_clear(
    burst: &mut BurstState,
    bucket: &ChannelBucket,
    session_id: &SessionId,
    withdraw: &mut Vec<String>,
) {
    let mut remove_channel = false;
    if let Some(channel) = burst.channels.get_mut(bucket) {
        channel.recent_sessions.remove(session_id);
        let any_remaining_in_bucket = burst
            .sessions
            .values()
            .any(|s| s.channel.as_ref() == Some(bucket) && !s.is_empty());
        if !any_remaining_in_bucket {
            if channel.has_live_notice {
                withdraw.push(bucket.notice_id());
            }
            remove_channel = true;
        }
    }
    if remove_channel {
        burst.channels.remove(bucket);
    }
}

fn is_suppressed_inbox(
    config: &NotifyConfig,
    frontmost: &FrontmostContext,
    request_id: &RequestId,
    session_id: &SessionId,
    channel_id: Option<&ChannelId>,
) -> bool {
    if !config.quiet_when_frontmost || !frontmost.app_active {
        return false;
    }
    if let Some(viewing_req) = frontmost.viewing_request.as_ref() {
        return viewing_req == request_id;
    }
    if frontmost.viewing_inbox {
        return true;
    }
    if frontmost.viewing_session.as_ref() == Some(session_id) && frontmost.viewing_session_inbox {
        return true;
    }
    if let Some(channel) = channel_id
        && frontmost.viewing_channel_inbox.as_ref() == Some(channel)
    {
        return true;
    }
    false
}

fn is_suppressed_session(
    config: &NotifyConfig,
    frontmost: &FrontmostContext,
    session_id: &SessionId,
) -> bool {
    config.quiet_when_frontmost
        && frontmost.app_active
        && frontmost.viewing_session.as_ref() == Some(session_id)
}

fn has_active_episode(
    burst: &BurstState,
    session_id: &SessionId,
    kind: SessionEpisodeKind,
) -> bool {
    burst
        .sessions
        .get(session_id)
        .is_some_and(|entry| entry.episodes.contains(&kind))
}

enum SessionDedup {
    Request(RequestId),
    Episode(SessionEpisodeKind),
}

struct SessionEventMeta {
    session_id: SessionId,
    session_title: String,
    channel_id: Option<ChannelId>,
    channel_name: Option<String>,
    title: String,
    body: String,
    target: NotifyTarget,
    dedup: SessionDedup,
}

fn record_session_event(
    config: &NotifyConfig,
    burst: &mut BurstState,
    meta: SessionEventMeta,
    now_secs: u64,
) -> DesktopNotice {
    let bucket = ChannelBucket::from_option(meta.channel_id);

    // Prune expired sessions from this channel's burst window, then insert this session.
    let channel = burst.channels.entry(bucket.clone()).or_default();
    channel
        .recent_sessions
        .retain(|_, ts| now_secs.saturating_sub(*ts) < config.cooldown_secs);
    channel
        .recent_sessions
        .insert(meta.session_id.clone(), now_secs);
    let distinct_sessions = channel.recent_sessions.len();

    // Update per-session deduplication and burst state.
    let session = burst.sessions.entry(meta.session_id.clone()).or_default();
    session.channel = Some(bucket.clone());
    match meta.dedup {
        SessionDedup::Request(request_id) => {
            session.requests.insert(request_id.clone());
            burst
                .request_index
                .insert(request_id, RequestOwner::Session(meta.session_id.clone()));
        }
        SessionDedup::Episode(kind) => {
            session.episodes.insert(kind);
        }
    }

    let within_session_cooldown = session
        .last_posted_secs
        .is_some_and(|last| now_secs.saturating_sub(last) < config.cooldown_secs);

    if within_session_cooldown {
        session.burst_count = session.burst_count.saturating_add(1);
    } else {
        session.burst_count = 1;
    }
    session.last_posted_secs = Some(now_secs);

    // 2+ distinct sessions in the same channel within cooldown_secs collapse to the channel notice.
    if distinct_sessions >= 2 && config.cooldown_secs > 0 {
        channel.has_live_notice = true;
        let title = match (&bucket, meta.channel_name.as_deref()) {
            (ChannelBucket::Channel(_), Some(name)) if !name.trim().is_empty() => {
                format!("{distinct_sessions} sessions need you in #{name}")
            }
            (ChannelBucket::Channel(id), _) => {
                format!("{distinct_sessions} sessions need you in #{}", id.as_str())
            }
            (ChannelBucket::Desk, _) => {
                format!("{distinct_sessions} sessions need you on Desk")
            }
        };
        return DesktopNotice {
            id: bucket.notice_id(),
            title,
            body: "Open the Agent inbox to answer.".to_owned(),
            target: NotifyTarget::Inbox,
            sound: None,
        };
    }

    session.has_live_notice = true;
    let id = session_notice_id(&meta.session_id);

    if within_session_cooldown && session.burst_count >= 2 {
        return DesktopNotice {
            id,
            title: format!("{} items need you in {}", session.burst_count, meta.session_title),
            body: meta.body,
            target: NotifyTarget::Session(meta.session_id),
            sound: None,
        };
    }

    DesktopNotice {
        id,
        title: meta.title,
        body: meta.body,
        target: meta.target,
        sound: None,
    }
}
