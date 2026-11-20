use std::sync::Arc;

use humata2_clients::model::{ChannelId, RequestKind, ResumeFailure, Session, SessionId, Status};
use humata2_clients::{DaemonError, DesktopNotice, Event, Workspace};
use humata2_notify::{FrontmostContext, NotifyEvent};
use humata2_session::{Panel, SessionAction};
use tca::{Effect, StreamEnd};

use crate::{App, AppAction, AppState, Load, Route, channel, upsert};

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
) -> (String, Option<ChannelId>, Option<String>) {
    match workspace.session(session_id) {
        Some(session) => session_channel_meta(workspace, session),
        None => (session_id.as_str().to_owned(), None, None),
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

impl App {
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
        FrontmostContext {
            app_active: state.window_active,
            viewing_inbox: state.route == Route::Inbox,
            viewing_home: state.route == Route::Home,
            viewing_session,
            viewing_session_inbox: state.panel() == Some(Panel::Inbox),
            viewing_channel_inbox,
        }
    }

    pub(super) fn post_notice(&self, notice: DesktopNotice) -> Effect<AppAction> {
        let notifier = self.clients.notifier.clone();
        Effect::run(move |_send| async move {
            notifier.post(notice);
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
        self.withdraw_notices(withdraw)
    }

    pub(super) fn apply(&self, state: &mut AppState, event: Event) -> Effect<AppAction> {
        let frontmost = Self::frontmost_context(state);
        let now_secs = self.clients.clock.now().as_secs();
        let live = state.load == Load::Ready;
        let mut snapshot = None;
        let mut left = false;
        let mut permission_alert = None;
        let mut notices: Vec<DesktopNotice> = Vec::new();
        let mut withdraw_ids: Vec<String> = Vec::new();
        match event {
            Event::Output { session, tab, line } => {
                return self.child(
                    state,
                    AppAction::Session(SessionAction::Output { session, tab, line }),
                );
            }
            Event::Session(session) => {
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
                    withdraw_ids.extend(humata2_notify::clear_session(
                        &mut state.notify_burst,
                        &session.id,
                    ));
                } else if live {
                    let (session_title, channel_id, channel_name) =
                        session_channel_meta(&state.workspace, &session);
                    if prev_resume_failed.is_none()
                        && let Some(failure) = session.resume_failed
                        && let Some(notice) = humata2_notify::evaluate(
                            &state.settings.preferences.notify,
                            &frontmost,
                            &mut state.notify_burst,
                            NotifyEvent::ResumeFailed {
                                session_id: session.id.clone(),
                                session_title: session_title.clone(),
                                channel_id: channel_id.clone(),
                                channel_name: channel_name.clone(),
                                reason: resume_failure_reason(failure),
                            },
                            now_secs,
                        )
                    {
                        notices.push(notice);
                    }
                    if prev_status != Some(Status::NeedsYou) && session.status == Status::NeedsYou {
                        let has_pending_inbox_request = state.workspace.waiting().any(|request| {
                            request.session == session.id
                                && request.kind != RequestKind::Migrate
                                && !request.operation.starts_with("board.")
                        });
                        if let Some(notice) = humata2_notify::evaluate(
                            &state.settings.preferences.notify,
                            &frontmost,
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
                        ) {
                            notices.push(notice);
                        }
                    }
                    if let Status::Exited { code } = session.status
                        && code != 0
                        && prev_status != Some(Status::Exited { code })
                        && let Some(notice) = humata2_notify::evaluate(
                            &state.settings.preferences.notify,
                            &frontmost,
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
                        )
                    {
                        notices.push(notice);
                    }
                    if session.status == Status::Working
                        && session.resume_failed.is_none()
                        && prev_status != Some(Status::Working)
                        && !state.workspace.waiting().any(|request| {
                            request.session == session.id
                                && request.kind != RequestKind::Migrate
                                && !request.operation.starts_with("board.")
                        })
                    {
                        withdraw_ids.extend(humata2_notify::clear_session(
                            &mut state.notify_burst,
                            &session.id,
                        ));
                    }
                }
                snapshot = Some(session.id.clone());
                let workspace = Arc::make_mut(&mut state.workspace);
                upsert(&mut workspace.sessions, session, |a, b| a.id == b.id);
                // A tab opened from New agent tab keeps its agent (D858).
                state.session.keep_agent_tabs(workspace);
            }
            Event::Request(request) => {
                let is_new = !state
                    .workspace
                    .requests
                    .iter()
                    .any(|saved| saved.id == request.id);
                if state.settings.preferences.permission_request_alerts
                    && request.answer.is_none()
                    && !matches!(request.kind, RequestKind::Question)
                    && is_new
                {
                    permission_alert = Some(format!("Permission requested: {}", request.title));
                }
                if request.answer.is_some() {
                    withdraw_ids.extend(humata2_notify::clear_request(
                        &mut state.notify_burst,
                        &request.id,
                    ));
                } else if live
                    && is_new
                    && request.kind != RequestKind::Migrate
                    && !request.operation.starts_with("board.")
                    && let Some(notice) = humata2_inbox::notification(&state.workspace, &request)
                {
                    let (session_title, channel_id, channel_name) =
                        session_meta(&state.workspace, &request.session);
                    if let Some(desktop_notice) = humata2_notify::evaluate(
                        &state.settings.preferences.notify,
                        &frontmost,
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
                    ) {
                        notices.push(desktop_notice);
                    }
                }
                let workspace = Arc::make_mut(&mut state.workspace);
                upsert(&mut workspace.requests, request, |a, b| a.id == b.id);
            }
            Event::RequestGone(id) => {
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
            Event::Resync => return self.load(),
        }
        let mut effects = vec![self.show(state)];
        if left {
            effects.push(self.handle(
                state,
                AppAction::Session(SessionAction::Delegate(humata2_session::Delegate::Left)),
            ));
        }
        if let Some(text) = permission_alert {
            effects.push(state.toast(text, &self.clients.clock));
        }
        for notice in notices {
            effects.push(self.post_notice(notice));
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
        let text = match end {
            StreamEnd::Finished => "The daemon stopped sending updates. Reconnecting.".to_owned(),
            StreamEnd::Failed(error) => {
                format!("Lost updates from the daemon: {error}. Reconnecting.")
            }
        };
        Effect::merge([state.toast(text, &clock), self.load(), self.follow()])
    }
}
