use std::sync::Arc;
use std::time::Duration;

use humata2_clients::model::{ChannelId, RequestKind, ResumeFailure, Session, SessionId, Status};
use humata2_clients::{DaemonError, DesktopNotice, Event, NoticeSound, Workspace};
use humata2_notify::{
    DeckToastQueue, FrontmostContext, NotifyDecision, NotifyEvent, dock_badge_count, trace_gate,
};
use humata2_session::{Panel, SessionAction};
use tca::{Effect, StreamEnd};

use crate::{App, AppAction, AppState, Load, Route, Toast, channel, upsert};

fn session_channel_meta(
    workspace: &Workspace,
    session: &Session,
) -> (String, Option<ChannelId>, Option<String>) {
    let title = if session.title.is_empty() {
        session.id.as_str().to_owned()
    } else {
        session.title.clone()
    };
    if humata2_clients::desk::is_desk(session) {
        (title, None, None)
    } else {
        let name = workspace
            .channel(&session.channel)
            .map(|c| c.name.clone())
            .or_else(|| Some(session.channel.as_str().to_owned()));
        (title, Some(session.channel.clone()), name)
    }
}

fn session_meta(
    workspace: &Workspace,
    session_id: &SessionId,
    channel_id: &ChannelId,
) -> (String, Option<ChannelId>, Option<String>) {
    match workspace.session(session_id) {
        Some(session) => session_channel_meta(workspace, session),
        None if *channel_id == humata2_clients::desk::channel() => {
            (session_id.as_str().to_owned(), None, None)
        }
        None => {
            let name = workspace
                .channel(channel_id)
                .map(|c| c.name.clone())
                .or_else(|| Some(channel_id.as_str().to_owned()));
            (session_id.as_str().to_owned(), Some(channel_id.clone()), name)
        }
    }
}

fn resume_failure_reason(failure: ResumeFailure) -> String {
    match failure {
        ResumeFailure::ConversationMissing => "Its conversation file is missing.".to_owned(),
        ResumeFailure::OtherConversation => {
            "The agent started a different conversation.".to_owned()
        }
        ResumeFailure::AgentExited(code) => {
            format!("The agent exited while resuming ({code}).")
        }
        ResumeFailure::AgentNotInstalled => "The agent isn't installed on this host.".to_owned(),
        ResumeFailure::FolderMissing => "The session's folder is missing.".to_owned(),
        ResumeFailure::ConversationUnknown => {
            "The agent's conversation couldn't be identified.".to_owned()
        }
    }
}

fn blocked_inbox_asks(workspace: &Workspace) -> usize {
    workspace
        .waiting()
        .filter(|r| {
            r.kind != RequestKind::Migrate
                && !r.is_to_do_ask()
                && !r.operation.starts_with("board.")
        })
        .count()
}

impl App {
    /// Prunes heavy cached per-session state when a session is archived or removed.
    pub(super) fn evict_session(state: &mut AppState, id: &SessionId) {
        state.session.evict_session(id);
        state.review.evict_session(id);
        let workspace = Arc::make_mut(&mut state.workspace);
        workspace.session_files.remove(id);
        workspace.diffs.retain(|diff| &diff.session != id);
    }

    pub(super) fn frontmost_context(state: &AppState) -> FrontmostContext {
        let viewing_session = match &state.route {
            Route::Session(id) => Some(id.clone()),
            _ => None,
        };
        let viewing_channel_inbox = match &state.route {
            Route::Channel(id)
                if state
                    .channel_pages
                    .get(id)
                    .is_some_and(|v| v.tab == channel::Tab::Inbox) =>
            {
                Some(id.clone())
            }
            _ => None,
        };
        let viewing_inbox = state.route == Route::Inbox;
        let viewing_session_inbox = state.panel() == Some(Panel::Inbox);
        // When the full Inbox list (global, channel, or session panel) is open,
        // leave `viewing_request` as `None` so list-level suppression applies
        // to every incoming request visible in that list; callers focused on a
        // single request detail can set `viewing_request = Some(id)`.
        let viewing_request = None;
        FrontmostContext {
            app_active: state.window_active,
            viewing_inbox,
            viewing_home: state.route == Route::Home,
            viewing_session,
            viewing_session_inbox,
            viewing_channel_inbox,
            viewing_todo: state.route == Route::Todo,
            viewing_request,
        }
    }

    pub(super) fn post_notice(&self, notice: DesktopNotice) -> Effect<AppAction> {
        trace_gate(
            "app_post_notice",
            "dispatch",
            &format!("id={} sound={:?}", notice.id, notice.sound),
        );
        let notifier = self.clients.notifier.clone();
        Effect::run(move |_send| async move {
            notifier.post(notice);
        })
    }

    fn play_sound(&self, sound: NoticeSound) -> Effect<AppAction> {
        let notifier = self.clients.notifier.clone();
        Effect::run(move |_send| async move {
            notifier.play_sound(sound);
        })
    }

    fn set_dock_badge(&self, count: Option<usize>) -> Effect<AppAction> {
        let notifier = self.clients.notifier.clone();
        Effect::run(move |_send| async move {
            notifier.set_dock_badge(count);
        })
    }

    pub(super) fn withdraw_notices(&self, ids: Vec<String>) -> Effect<AppAction> {
        if ids.is_empty() {
            return Effect::none();
        }
        let notifier = self.clients.notifier.clone();
        Effect::run(move |_send| async move {
            notifier.withdraw(&ids);
        })
    }

    pub(super) fn clear_visible_notices(&self, state: &mut AppState) -> Effect<AppAction> {
        if !state.window_active {
            trace_gate(
                "clear_visible_notices",
                "skip",
                &format!("window_active=false route={:?}", state.route),
            );
            return Effect::none();
        }
        let mut withdraw = Vec::new();
        match &state.route {
            Route::Session(id) => {
                withdraw.extend(humata2_notify::clear_session(&mut state.notify_burst, id));
            }
            Route::Inbox => {
                for request in state.workspace.waiting() {
                    withdraw.extend(humata2_notify::clear_request(
                        &mut state.notify_burst,
                        &request.id,
                    ));
                }
            }
            Route::Channel(channel_id)
                if state
                    .channel_pages
                    .get(channel_id)
                    .is_some_and(|v| v.tab == channel::Tab::Inbox) =>
            {
                for request in state.workspace.waiting() {
                    if state
                        .workspace
                        .session(&request.session)
                        .is_some_and(|s| &s.channel == channel_id)
                    {
                        withdraw.extend(humata2_notify::clear_request(
                            &mut state.notify_burst,
                            &request.id,
                        ));
                    }
                }
            }
            Route::Home => {
                for machine in &state.workspace.machines {
                    withdraw.extend(humata2_notify::clear_restart(
                        &mut state.notify_burst,
                        &machine.id,
                    ));
                }
                for restart in &state.workspace.restarts {
                    withdraw.extend(humata2_notify::clear_restart(
                        &mut state.notify_burst,
                        &restart.machine,
                    ));
                }
            }
            _ => {}
        }
        if !withdraw.is_empty() {
            trace_gate(
                "clear_visible_notices",
                "withdraw",
                &format!(
                    "window_active=true route={:?} count={}",
                    state.route,
                    withdraw.len()
                ),
            );
        }
        self.withdraw_notices(withdraw)
    }

    pub(super) fn apply(&self, state: &mut AppState, event: Event) -> Effect<AppAction> {
        let frontmost = Self::frontmost_context(state);
        let now_secs = self.clients.clock.now().as_secs();
        let live = state.load == Load::Ready;
        let mut snapshot = None;
        let mut left = false;
        let mut permission_alert = None;
        let mut decisions: Vec<NotifyDecision> = Vec::new();
        let mut withdraw_ids: Vec<String> = Vec::new();
        let mut update_badge = false;
        match event {
            Event::Output { session, tab, line } => {
                return self.child(
                    state,
                    AppAction::Session(SessionAction::Output { session, tab, line }),
                );
            }
            Event::Session(session) => {
                update_badge = true;
                // Archived from another window: leave its page here, and drop
                // it from the Back trail without moving off other screens.
                let gone = Route::Session(session.id.clone());
                left = session.archived && state.route == gone;
                if session.archived && !left {
                    state.trail.retain(|route| route != &gone);
                }
                let (prev_status, prev_resume_failed) = state
                    .workspace
                    .session(&session.id)
                    .map_or((None, None), |prev| (Some(prev.status), prev.resume_failed));
                if session.archived {
                    if let Some(record) = state.workspace.desk.channel_sessions.get(&session.id)
                        && record.target.is_none()
                    {
                        state.desk.channel_status.insert(
                            record.channel.clone(),
                            humata2_desk::ChannelDeskStatus::Closed,
                        );
                    }
                    let cleared = humata2_notify::clear_session(
                        &mut state.notify_burst,
                        &session.id,
                    );
                    if !cleared.is_empty() {
                        trace_gate(
                            "event_session",
                            "clear_session",
                            &format!(
                                "session={} reason=archived count={}",
                                session.id.as_str(),
                                cleared.len()
                            ),
                        );
                    }
                    withdraw_ids.extend(cleared);
                    Self::evict_session(state, &session.id);
                } else if live {
                    let (session_title, channel_id, channel_name) =
                        session_channel_meta(&state.workspace, &session);
                    if prev_resume_failed.is_none()
                        && let Some(failure) = session.resume_failed
                    {
                        let decision = humata2_notify::evaluate_decision(
                            &state.settings.preferences.notify,
                            &frontmost,
                            state.settings.notification_permission,
                            &mut state.notify_burst,
                            NotifyEvent::ResumeFailed {
                                session_id: session.id.clone(),
                                session_title: session_title.clone(),
                                channel_id: channel_id.clone(),
                                channel_name: channel_name.clone(),
                                reason: resume_failure_reason(failure),
                            },
                            now_secs,
                        );
                        trace_gate(
                            "event_session_resume_failed",
                            if decision.os_notice.is_some() || decision.deck_toast.is_some() {
                                "notify"
                            } else {
                                "suppressed"
                            },
                            &format!("session={}", session.id.as_str()),
                        );
                        decisions.push(decision);
                    }
                    if prev_status != Some(Status::NeedsYou) && session.status == Status::NeedsYou {
                        let has_pending_inbox_request = state.workspace.waiting().any(|request| {
                            request.session == session.id
                                && request.kind != RequestKind::Migrate
                                && !request.is_to_do_ask()
                                && !request.operation.starts_with("board.")
                        });
                        let has_pending_to_do_ask = state.workspace.requests.iter().any(|request| {
                            request.session == session.id
                                && request.answer.is_none()
                                && request.is_to_do_ask()
                        });
                        if has_pending_to_do_ask {
                            trace_gate(
                                "event_session_needs_you",
                                "suppressed",
                                &format!("session={} reason=pending_todo_ask", session.id.as_str()),
                            );
                        } else {
                            let decision = humata2_notify::evaluate_decision(
                                &state.settings.preferences.notify,
                                &frontmost,
                                state.settings.notification_permission,
                                &mut state.notify_burst,
                                NotifyEvent::TerminalNeedsYou {
                                    session_id: session.id.clone(),
                                    session_title: session_title.clone(),
                                    channel_id: channel_id.clone(),
                                    channel_name: channel_name.clone(),
                                    agent_name: session.agent.name().to_owned(),
                                    has_pending_inbox_request,
                                },
                                now_secs,
                            );
                            trace_gate(
                                "event_session_needs_you",
                                if decision.os_notice.is_some() || decision.deck_toast.is_some() {
                                    "notify"
                                } else {
                                    "suppressed"
                                },
                                &format!(
                                    "session={} has_pending_inbox={has_pending_inbox_request}",
                                    session.id.as_str()
                                ),
                            );
                            decisions.push(decision);
                        }
                    }
                    if let Status::Exited { code } = session.status
                        && code != 0
                        && prev_status != Some(Status::Exited { code })
                    {
                        let decision = humata2_notify::evaluate_decision(
                            &state.settings.preferences.notify,
                            &frontmost,
                            state.settings.notification_permission,
                            &mut state.notify_burst,
                            NotifyEvent::AbnormalExit {
                                session_id: session.id.clone(),
                                session_title,
                                channel_id,
                                channel_name,
                                code,
                                already_resume_failed: session.resume_failed.is_some(),
                            },
                            now_secs,
                        );
                        trace_gate(
                            "event_session_abnormal_exit",
                            if decision.os_notice.is_some() || decision.deck_toast.is_some() {
                                "notify"
                            } else {
                                "suppressed"
                            },
                            &format!("session={} code={code}", session.id.as_str()),
                        );
                        decisions.push(decision);
                    }
                    if session.status == Status::Working
                        && session.resume_failed.is_none()
                        && prev_status != Some(Status::Working)
                        && !state.workspace.waiting().any(|request| {
                            request.session == session.id
                                && request.kind != RequestKind::Migrate
                                && !request.is_to_do_ask()
                                && !request.operation.starts_with("board.")
                        })
                    {
                        let cleared = humata2_notify::clear_session(
                            &mut state.notify_burst,
                            &session.id,
                        );
                        if !cleared.is_empty() {
                            trace_gate(
                                "event_session",
                                "clear_session",
                                &format!(
                                    "session={} reason=working count={}",
                                    session.id.as_str(),
                                    cleared.len()
                                ),
                            );
                        }
                        withdraw_ids.extend(cleared);
                    }
                }
                snapshot = Some(session.id.clone());
                let workspace = Arc::make_mut(&mut state.workspace);
                upsert(&mut workspace.sessions, session, |a, b| a.id == b.id);
                // A tab opened from New agent tab keeps its agent (D858).
                state.session.keep_agent_tabs(workspace);
            }
            Event::Request(request) => {
                update_badge = true;
                let is_new = !state
                    .workspace
                    .requests
                    .iter()
                    .any(|saved| saved.id == request.id);
                let answered = request.answer.is_some();
                let is_todo_ask = request.is_to_do_ask();
                let is_board_op = request.operation.starts_with("board.");
                let is_migrate = request.kind == RequestKind::Migrate;
                trace_gate(
                    "event_request",
                    "enter",
                    &format!(
                        "id={} live={live} is_new={is_new} answered={answered} todo={is_todo_ask} board={is_board_op} migrate={is_migrate}",
                        request.id.as_str()
                    ),
                );
                if state.settings.preferences.permission_request_alerts
                    && !answered
                    && !matches!(request.kind, RequestKind::Question)
                    && !is_todo_ask
                    && is_new
                {
                    permission_alert = Some(format!("Permission requested: {}", request.title));
                }
                if answered {
                    withdraw_ids.extend(humata2_notify::clear_request(
                        &mut state.notify_burst,
                        &request.id,
                    ));
                } else if live
                    && is_new
                    && let Some(ask) = &request.to_do
                {
                    if let Some(notice) =
                        humata2_inbox::to_do_notification(&state.workspace, &request)
                    {
                        let channel_id = Some(request.channel.clone());
                        let item = match ask {
                            humata2_clients::model::ToDoAsk::Item(id) => Some(id.clone()),
                            humata2_clients::model::ToDoAsk::NewItem => None,
                        };
                        let decision = humata2_notify::evaluate_decision(
                            &state.settings.preferences.notify,
                            &frontmost,
                            state.settings.notification_permission,
                            &mut state.notify_burst,
                            NotifyEvent::TodoAsk {
                                request_id: request.id.clone(),
                                session_id: request.session.clone(),
                                channel_id,
                                item,
                                answered: false,
                                title: notice.title,
                                body: notice.body,
                            },
                            now_secs,
                        );
                        trace_gate(
                            "event_request_todo",
                            if decision.os_notice.is_some() || decision.deck_toast.is_some() {
                                "notify"
                            } else {
                                "suppressed"
                            },
                            &format!("id={}", request.id.as_str()),
                        );
                        decisions.push(decision);
                    } else {
                        trace_gate(
                            "event_request_todo",
                            "no_inbox_notice",
                            &format!("id={}", request.id.as_str()),
                        );
                    }
                } else if live && is_new && !is_migrate && !is_todo_ask && !is_board_op {
                    if let Some(notice) = humata2_inbox::notification(&state.workspace, &request) {
                        let (session_title, channel_id, channel_name) =
                            session_meta(&state.workspace, &request.session, &request.channel);
                        let decision = humata2_notify::evaluate_decision(
                            &state.settings.preferences.notify,
                            &frontmost,
                            state.settings.notification_permission,
                            &mut state.notify_burst,
                            NotifyEvent::InboxRequest {
                                request_id: request.id.clone(),
                                session_id: request.session.clone(),
                                session_title,
                                channel_id,
                                channel_name,
                                kind: request.kind.clone(),
                                operation: request.operation.clone(),
                                answered: false,
                                title: notice.title,
                                body: notice.body,
                            },
                            now_secs,
                        );
                        trace_gate(
                            "event_request_inbox",
                            if decision.os_notice.is_some() || decision.deck_toast.is_some() {
                                "notify"
                            } else {
                                "suppressed"
                            },
                            &format!("id={}", request.id.as_str()),
                        );
                        decisions.push(decision);
                    } else {
                        trace_gate(
                            "event_request_inbox",
                            "no_inbox_notice",
                            &format!("id={}", request.id.as_str()),
                        );
                    }
                }
                let workspace = Arc::make_mut(&mut state.workspace);
                upsert(&mut workspace.requests, request, |a, b| a.id == b.id);
            }
            Event::RequestGone(id) => {
                update_badge = true;
                Arc::make_mut(&mut state.workspace)
                    .requests
                    .retain(|request| request.id != id);
                withdraw_ids.extend(humata2_notify::clear_request(&mut state.notify_burst, &id));
            }
            Event::Rules { channel, rules } => {
                Arc::make_mut(&mut state.workspace)
                    .rules
                    .insert(channel, rules);
            }
            Event::Machine(machine) => {
                upsert(
                    &mut Arc::make_mut(&mut state.workspace).machines,
                    machine,
                    |a, b| a.id == b.id,
                );
            }
            Event::Transfer(transfer) => {
                upsert(
                    &mut Arc::make_mut(&mut state.workspace).transfers,
                    transfer,
                    |a, b| a.id == b.id,
                );
            }
            Event::Item { item, details } => {
                let workspace = Arc::make_mut(&mut state.workspace);
                workspace.item_details.insert(item.id.clone(), details);
                upsert(&mut workspace.items, item, |a, b| a.id == b.id);
            }
            Event::ItemGone(id) => {
                let workspace = Arc::make_mut(&mut state.workspace);
                workspace.items.retain(|item| item.id != id);
                workspace.item_details.remove(&id);
            }
            Event::Files { session, change } => return self.files_changed(state, session, change),
            // GAP (D438): an open search isn't run again when the index
            // catches up; the next keystroke searches the new entries.
            Event::HistoryIndexed(_) => return Effect::none(),
            // DESKWATCH: the Desk follows what the daemon tells, with no reload.
            Event::Desk(change) => Arc::make_mut(&mut state.workspace).desk.apply(change),
            // MACHINESCRIPTS (D686): the open machine page reads its runs again, and so does
            // Settings > Machines while it is the page shown. Nothing else reads.
            Event::MachineScriptRun(machine) => {
                let mut effects = Vec::new();
                if state.machine_page.machine.as_ref() == Some(&machine) {
                    effects.push(self.machine_page(
                        state,
                        humata2_hosts::MachinePageAction::ScriptRunChanged(machine),
                    ));
                }
                if state.route == crate::Route::Settings
                    && state.settings.section == humata2_settings::Section::Machines
                {
                    effects.push(self.child(
                        state,
                        AppAction::Settings(humata2_settings::SettingsAction::Section(
                            humata2_settings::Section::Machines,
                        )),
                    ));
                }
                return Effect::merge(effects);
            }
            // D908: the open Sync or Browser section reads its live state again.
            Event::SettingsChanged => {
                if state.route == crate::Route::Settings
                    && state.settings.section == humata2_settings::Section::Sync
                {
                    return self.child(
                        state,
                        AppAction::Settings(humata2_settings::SettingsAction::Sync(
                            humata2_settings::SyncAction::Load,
                        )),
                    );
                }
                if state.route == crate::Route::Settings
                    && state.settings.section == humata2_settings::Section::Browser
                {
                    return self.child(
                        state,
                        AppAction::Settings(humata2_settings::SettingsAction::Browser(
                            humata2_settings::BrowserAction::Load,
                        )),
                    );
                }
                if state.route == crate::Route::Settings
                    && matches!(
                        state.settings.section,
                        humata2_settings::Section::General
                            | humata2_settings::Section::Agents
                            | humata2_settings::Section::Layout
                            | humata2_settings::Section::Notifications
                    )
                {
                    return self.child(
                        state,
                        AppAction::Settings(humata2_settings::SettingsAction::General(
                            humata2_settings::GeneralAction::Load,
                        )),
                    );
                }
                return Effect::none();
            }
            Event::PendingCliLogin(login) => {
                upsert(&mut state.pending_cli_logins, login, |a, b| a.id == b.id);
                return Effect::none();
            }
            Event::PendingCliLoginGone(id) => {
                state.pending_cli_logins.retain(|login| login.id != id);
                state.answering_cli_logins.remove(&id);
                return Effect::none();
            }
            Event::Restart(restart) => return self.heard_restart(state, restart),
            Event::ChannelSetup(setup) => return self.heard_setup(state, setup),
            Event::Operation(update) => {
                self.new_session_operation(state, update.clone());
                self.add_repo_operation(state, update.clone());
                let pr_dialog = self.child(
                    state,
                    AppAction::PrDialog(humata2_pr_review::dialog::DialogAction::OperationMoved(
                        update.clone(),
                    )),
                );
                let session = self.child(
                    state,
                    AppAction::Session(SessionAction::OperationMoved(update)),
                );
                return Effect::merge([pr_dialog, session]);
            }
            Event::Resync => {
                // SMB-032: after a daemon restart/resync, restart the Pinned
                // watch (and Todo watch when To-do is open) so live feeds
                // resume instead of staying silently dead.
                let mut effects = vec![
                    self.load(),
                    self.child(state, AppAction::Pinned(humata2_find::PinnedAction::Start)),
                ];
                if state.route == crate::Route::Todo {
                    effects.push(self.child(
                        state,
                        AppAction::Todo(humata2_todo_feature::TodoAction::Watch),
                    ));
                }
                return Effect::merge(effects);
            }
        }
        let mut effects = vec![self.show(state)];
        if left {
            effects.push(self.handle(
                state,
                AppAction::Session(SessionAction::Delegate(humata2_session::Delegate::Left)),
            ));
        }
        let has_permission_alert = permission_alert.is_some();
        if let Some(text) = permission_alert {
            effects.push(state.toast(text, &self.clients.clock));
        }
        let mut deck_queue = DeckToastQueue::default();
        for decision in decisions {
            if let Some(notice) = decision.os_notice {
                effects.push(self.post_notice(notice));
            } else if let Some(sound) = decision.in_app_sound {
                effects.push(self.play_sound(sound));
            }
            if let Some(deck_toast) = decision.deck_toast {
                deck_queue.push(deck_toast, now_secs);
            }
        }
        if live
            && !has_permission_alert
            && let Some(deck_toast) = deck_queue.current()
        {
            debug_assert!(!DeckToastQueue::can_evict_undo());
            let mut plain_ids = Vec::new();
            state.toasts.retain(|toast| {
                if toast.action.is_none() {
                    plain_ids.push(toast.id);
                    false
                } else {
                    true
                }
            });
            for id in plain_ids {
                effects.push(Effect::cancel(format!("toast-{id}")));
            }
            state.next_toast += 1;
            let id = state.next_toast;
            state.toasts.push(Toast {
                id,
                text: deck_toast.title.clone(),
                action: None,
                working: false,
            });
            let sleep = self
                .clients
                .clock
                .sleep(Duration::from_secs(deck_toast.duration_secs));
            effects.push(
                Effect::future(async move {
                    sleep.await;
                    AppAction::ToastExpired(id)
                })
                .cancellable(format!("toast-{id}"), true),
            );
        }
        if update_badge {
            effects.push(self.set_dock_badge(dock_badge_count(blocked_inbox_asks(
                &state.workspace,
            ))));
        }
        if !withdraw_ids.is_empty() {
            effects.push(self.withdraw_notices(withdraw_ids));
        }
        // A session event can be the one that marks the open page a review
        // after it was arrived at: follow it now.
        if let Some(id) = snapshot.as_ref().filter(|id| state.route == crate::Route::Session((*id).clone())) {
            effects.push(self.follow_review(state, id));
        }
        if let Some(id) = snapshot {
            effects.push(self.child(state, AppAction::Session(SessionAction::Snapshot(id))));
        }
        Effect::merge(effects)
    }

    pub(super) fn events_ended(
        &self,
        state: &mut AppState,
        end: StreamEnd<DaemonError>,
    ) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        let attempt = state.reconnect_attempts;
        state.reconnect_attempts = attempt.saturating_add(1);
        // SMB-003: toast once per outage (not on every failed retry) and back
        // off exponentially on repeated failures instead of spinning in a
        // tight reconnect loop.
        let mut effects = Vec::new();
        if attempt == 0 {
            let text = match end {
                StreamEnd::Finished => "The daemon stopped sending updates. Reconnecting.".to_owned(),
                StreamEnd::Failed(error) => {
                    format!("Lost updates from the daemon: {error}. Reconnecting.")
                }
            };
            effects.push(state.toast(text, &clock));
            effects.push(self.load());
            effects.push(self.follow());
        } else {
            let shift = (attempt - 1).min(6);
            let delay_ms = (250u64 << shift).min(10_000);
            effects.push(
                Effect::future(async move {
                    clock.sleep(std::time::Duration::from_millis(delay_ms)).await;
                    AppAction::ReconnectEvents
                })
                .cancellable("events-reconnect", true),
            );
        }
        Effect::merge(effects)
    }
}
