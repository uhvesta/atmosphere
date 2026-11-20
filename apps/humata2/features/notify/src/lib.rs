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

use humata2_clients::model::{ChannelId, MachineId, RequestId, RequestKind, SessionId};
use humata2_clients::{DesktopNotice, NotifyTarget};
use serde::{Deserialize, Serialize};

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
        return None;
    }

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
            if answered || operation.starts_with("board.") {
                return None;
            }
            let allowed = match kind {
                RequestKind::Migrate => config.migrate_items,
                RequestKind::Question => config.agent_questions,
                RequestKind::Command
                | RequestKind::FileEdit
                | RequestKind::Network
                | RequestKind::Merge
                | RequestKind::Destroy => config.inbox_requests,
            };
            if !allowed {
                return None;
            }
            if is_suppressed_inbox(config, frontmost, &session_id, channel_id.as_ref()) {
                return None;
            }
            if burst.request_index.contains_key(&request_id) {
                return None;
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
            Some(notice)
        }

        NotifyEvent::TerminalNeedsYou {
            session_id,
            session_title,
            channel_id,
            channel_name,
            agent_name,
            has_pending_inbox_request,
        } => {
            if !config.agent_questions || has_pending_inbox_request {
                return None;
            }
            if is_suppressed_session(config, frontmost, &session_id) {
                return None;
            }
            if has_active_episode(burst, &session_id, SessionEpisodeKind::TerminalNeedsYou) {
                return None;
            }
            let title = format!("{agent_name} needs you in {session_title}");
            let body = "Open the session to answer.".to_owned();
            let target = NotifyTarget::Session(session_id.clone());
            Some(record_session_event(
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
                return None;
            }
            if is_suppressed_session(config, frontmost, &session_id) {
                return None;
            }
            if has_active_episode(burst, &session_id, SessionEpisodeKind::ResumeFailed) {
                return None;
            }
            let title = format!("{session_title} couldn't resume");
            let body = if reason.trim().is_empty() {
                "Open the session to recover it.".to_owned()
            } else {
                reason
            };
            let target = NotifyTarget::Session(session_id.clone());
            Some(record_session_event(
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
            if !config.failed_resumes || code == 0 || already_resume_failed {
                return None;
            }
            if is_suppressed_session(config, frontmost, &session_id) {
                return None;
            }
            if has_active_episode(burst, &session_id, SessionEpisodeKind::ResumeFailed)
                || has_active_episode(burst, &session_id, SessionEpisodeKind::AbnormalExit)
            {
                return None;
            }
            let title = format!("{session_title} exited ({code})");
            let body = "Open the session to review or resume it.".to_owned();
            let target = NotifyTarget::Session(session_id.clone());
            Some(record_session_event(
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
                return None;
            }
            let count = not_brought_back.max(sessions.len());
            if count == 0 {
                return None;
            }
            if count == 1 && sessions.len() == 1 {
                let session_id = sessions.into_iter().next()?;
                if is_suppressed_session(config, frontmost, &session_id) {
                    return None;
                }
                if has_active_episode(burst, &session_id, SessionEpisodeKind::ResumeFailed)
                    || has_active_episode(
                        burst,
                        &session_id,
                        SessionEpisodeKind::SingleRestartFailed,
                    )
                {
                    return None;
                }
                let entry = burst.sessions.entry(session_id.clone()).or_default();
                entry.episodes.insert(SessionEpisodeKind::SingleRestartFailed);
                entry.last_posted_secs = Some(now_secs);
                entry.burst_count = entry.burst_count.saturating_add(1);
                entry.has_live_notice = true;
                return Some(DesktopNotice {
                    id: session_notice_id(&session_id),
                    title: format!("{machine_name} restarted: 1 session couldn't resume"),
                    body: "Open the session to recover it.".to_owned(),
                    target: NotifyTarget::Session(session_id),
                });
            }

            if config.quiet_when_frontmost && frontmost.app_active && frontmost.viewing_home {
                return None;
            }
            if let Some(prev) = burst.restarts.get(&machine_id)
                && now_secs.saturating_sub(prev.last_posted_secs) < config.cooldown_secs
            {
                return None;
            }
            burst.restarts.insert(
                machine_id.clone(),
                RestartBurst {
                    last_posted_secs: now_secs,
                    has_live_notice: true,
                },
            );
            let noun = if count == 1 { "session" } else { "sessions" };
            Some(DesktopNotice {
                id: restart_notice_id(&machine_id),
                title: format!("{machine_name} restarted: {count} {noun} couldn't resume"),
                body: "Open Home to review and recover them.".to_owned(),
                target: NotifyTarget::Home,
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
                return None;
            }
            if is_suppressed_session(config, frontmost, &session_id) {
                return None;
            }
            if has_active_episode(burst, &session_id, SessionEpisodeKind::FinishedTurn) {
                return None;
            }
            let title = format!("{agent_name} finished in {session_title}");
            let body = "Open the session to review.".to_owned();
            let target = NotifyTarget::Session(session_id.clone());
            Some(record_session_event(
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
            if !config.migrate_items || answered {
                return None;
            }
            if config.quiet_when_frontmost && frontmost.app_active && frontmost.viewing_inbox {
                return None;
            }
            if burst.request_index.contains_key(&request_id) {
                return None;
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
            Some(DesktopNotice {
                id,
                title,
                body,
                target: NotifyTarget::Inbox,
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
        RequestOwner::Migrate(notice_id) => {
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
    session_id: &SessionId,
    channel_id: Option<&ChannelId>,
) -> bool {
    if !config.quiet_when_frontmost || !frontmost.app_active {
        return false;
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
        };
    }

    DesktopNotice {
        id,
        title: meta.title,
        body: meta.body,
        target: meta.target,
    }
}
