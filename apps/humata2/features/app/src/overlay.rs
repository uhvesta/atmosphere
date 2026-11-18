use humata2_board::BoardAction;
use humata2_inbox::InboxAction;
use humata2_palette::PaletteAction;
use humata2_review::comments::CommentsAction;
use humata2_session::{Panel, SessionAction};
use humata2_clients::model::{SessionFile, SessionId};
use humata2_clients::{DaemonError, FileChange, SESSION_FILES_CAP};
use tca::Effect;

use crate::{App, AppAction, AppState, NewChannel, NewSession, ReportAction, Route};

/// Something over the screen. The top one takes the keyboard and Escape
/// closes it, putting focus back where it was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Overlay {
    /// The command palette.
    Palette,
    /// The New session sheet.
    NewSession(NewSession),
    /// The New channel sheet.
    NewChannel(NewChannel),
    /// REPOS (CH-21): Add repo to this session.
    SessionAddRepo(crate::session_add_repo::SessionAddRepo),
    /// REPOS (CH-6): a channel's Add a GitHub repo sheet.
    ChannelAddRepo(crate::ChannelAddRepo),
    /// Keyboard shortcuts.
    Shortcuts,
}

impl AppState {
    /// Whether something over the page holds the keyboard, so a terminal
    /// must not take focus from it: an overlay, a session's inline sheet,
    /// a Desk dialog or pop-up, the Add machine dialog, the Humata folder
    /// sheet, Review a pull request, or Submit review.
    #[must_use]
    pub fn dialog_holds_keys(&self) -> bool {
        !self.overlays.is_empty()
            || self.session.fork.is_some()
            || self.session.moving.is_some()
            || self.session.continuing.is_some()
            || self.session.archiving.is_some()
            || self.session.renaming.is_some()
            || self.desk.dialog_open()
            || self.add_machine.open
            || self.pr_dialog.open
            || self.pr_submit.step != humata2_pr_review::submit::Step::Closed
            || self.hosts.root_change.is_some()
    }
}

impl App {
    pub(super) fn escape(&self, state: &mut AppState) -> Effect<AppAction> {
        if let Some(form) = state.report.form.as_mut() {
            if form.drawing {
                form.drawing = false;
                return Effect::none();
            }
            return self.report(state, ReportAction::Close);
        }
        if state.overlay() == Some(&Overlay::Palette) && state.palette.asking.is_some() {
            return self.child(state, AppAction::Palette(PaletteAction::Escape));
        }
        if let Some(top) = state.overlays.pop() {
            if top == Overlay::Palette {
                state.palette.open = false;
            }
            return Effect::none();
        }
        // Escape leaves Submit review unposted; while it posts, it holds the
        // keys and stays until the posts answer.
        if state.pr_submit.step == humata2_pr_review::submit::Step::Posting {
            return Effect::none();
        }
        if state.pr_submit.step != humata2_pr_review::submit::Step::Closed {
            return self.child(
                state,
                AppAction::PrSubmit(humata2_pr_review::submit::SubmitAction::Close),
            );
        }
        if state.pr_dialog.open {
            return self.child(
                state,
                AppAction::PrDialog(humata2_pr_review::dialog::DialogAction::Close),
            );
        }
        if state.add_machine.open {
            return self.add_machine(state, humata2_hosts::AddMachineAction::Cancel);
        }
        if state.hosts.root_change.is_some() {
            return self.child(state, AppAction::Hosts(humata2_hosts::HostsAction::DismissRoot));
        }
        if state.desk.dialog_open() {
            return self.child(
                state,
                AppAction::Desk(humata2_desk::DeskAction::CancelDialog),
            );
        }
        if state.settings.font_menu {
            return self.child(
                state,
                AppAction::Settings(humata2_settings::SettingsAction::FontMenu),
            );
        }
        if state.navigation_overlay {
            state.navigation_overlay = false;
            return Effect::none();
        }
        if state.session.terminal_menu.is_some() {
            return self.child(state, AppAction::Session(SessionAction::TerminalMenu(None)));
        }
        if state.hosts.start_confirm.is_some() {
            return self.child(
                state,
                AppAction::Hosts(humata2_hosts::HostsAction::DismissStart),
            );
        }
        if state.machine_page.confirm.is_some() {
            return self.child(
                state,
                AppAction::MachinePage(humata2_hosts::MachinePageAction::DismissConfirm),
            );
        }
        if state.session.closing.is_some() {
            return self.child(
                state,
                AppAction::Session(SessionAction::Close(humata2_session::CloseAction::Cancel)),
            );
        }
        if state.machine_page.running.confirm_end.is_some() {
            return self.child(
                state,
                AppAction::MachinePage(humata2_hosts::MachinePageAction::Running(
                    humata2_hosts::RunningAction::CancelEnd,
                )),
            );
        }
        if state.machine_page.running.adopting.is_some() {
            return self.child(
                state,
                AppAction::MachinePage(humata2_hosts::MachinePageAction::Running(
                    humata2_hosts::RunningAction::CancelAdopt,
                )),
            );
        }
        if state.machine_page.menu_open {
            return self.child(
                state,
                AppAction::MachinePage(humata2_hosts::MachinePageAction::CloseMenu),
            );
        }
        if state.session.menu {
            return self.child(state, AppAction::Session(SessionAction::Menu(false)));
        }
        if state.session.renaming.is_some() {
            return self.child(state, AppAction::Session(SessionAction::CancelRename));
        }
        if state.session.archiving.is_some() {
            return self.child(state, AppAction::Session(SessionAction::CancelArchive));
        }
        if state.session.moving.is_some() {
            return self.child(state, AppAction::Session(SessionAction::CancelMove));
        }
        if state.session.continuing.is_some() {
            return self.child(state, AppAction::Session(SessionAction::CancelContinue));
        }
        if state.session.fork.is_some() {
            return self.child(state, AppAction::Session(SessionAction::CancelFork));
        }
        if state.inbox.confirm.is_some() {
            return self.child(state, AppAction::Inbox(InboxAction::CancelBulk));
        }
        if state.inbox.reply.is_some() {
            return self.child(state, AppAction::Inbox(InboxAction::CancelReply));
        }
        if state.board.new_title.is_some() {
            return self.child(state, AppAction::Board(BoardAction::CancelNew));
        }
        if state.board.drawer.is_some() && matches!(state.route, Route::Board(_)) {
            return self.child(state, AppAction::Board(BoardAction::Close));
        }
        if state.panel() == Some(Panel::Changes)
            && let Some(id) = state.session.open.clone()
            && state
                .comments
                .sessions
                .get(&id)
                .is_some_and(|held| held.editing.is_some())
        {
            return self.child(state, AppAction::Comments(CommentsAction::Cancel(id)));
        }
        Effect::none()
    }

    pub(super) fn open_palette(&self, state: &mut AppState) -> Effect<AppAction> {
        if state.overlay() == Some(&Overlay::Palette) {
            return Effect::none();
        }
        state.overlays.push(Overlay::Palette);
        state.palette.report = humata2_palette::ReportStatus {
            recording: state.report.recording,
            last: state.report.last.is_some(),
        };
        let in_session = matches!(state.route, Route::Session(_));
        state.palette.split =
            in_session && state.session.view().is_some_and(|view| view.groups.len() > 1);
        state.palette.context = match &state.route {
            Route::Session(id) => Some(id.clone()),
            _ => None,
        };
        state.palette.channel = match &state.route {
            Route::Channel(id) => Some(id.clone()),
            _ => None,
        };
        state.palette.marks = state.pinned.list.clone();
        let opened = self.child(
            state,
            AppAction::Palette(PaletteAction::Open { in_session }),
        );
        Effect::merge([opened, self.need_files(state)])
    }

    /// Asks the daemon for the files of every session not asked about yet,
    /// the first time the palette needs them. Later opens ask nothing.
    fn need_files(&self, state: &mut AppState) -> Effect<AppAction> {
        let wanted: Vec<SessionId> = state
            .workspace
            .sessions
            .iter()
            .filter(|session| !session.archived && !state.files_asked.contains(&session.id))
            .map(|session| session.id.clone())
            .collect();
        let effects: Vec<_> = wanted
            .into_iter()
            .map(|session| self.ask_files(state, session))
            .collect();
        Effect::merge(effects)
    }

    /// Asks the daemon for `session`'s files, unless they were asked for
    /// already.
    pub(super) fn ask_files(&self, state: &mut AppState, session: SessionId) -> Effect<AppAction> {
        if !state.files_asked.insert(session.clone()) {
            return Effect::none();
        }
        let daemon = self.clients.daemon.clone();
        Effect::future(async move {
            let result = daemon.find_files(&session).await;
            AppAction::SessionFiles { session, result }
        })
    }

    pub(super) fn session_files_found(
        &self,
        state: &mut AppState,
        session: SessionId,
        result: Result<Vec<SessionFile>, DaemonError>,
    ) -> Effect<AppAction> {
        match result {
            Ok(mut files) => {
                files.truncate(SESSION_FILES_CAP);
                std::sync::Arc::make_mut(&mut state.workspace)
                    .session_files
                    .insert(session.clone(), files);
                let shown = self.features_see_files(state);
                // A walkthrough showing on this session waited for them.
                let view = self.view_files_arrived(state, &session);
                Effect::merge([shown, view])
            }
            // Asked again the next time the palette opens or the session's
            // page does. The open session's Files tree and walkthrough wait
            // on the list, so its person is told why nothing came.
            Err(error) => {
                state.files_asked.remove(&session);
                if state.session.open.as_ref() != Some(&session) {
                    return Effect::none();
                }
                let clock = self.clients.clock.clone();
                state.toast(
                    format!("Couldn't list this session's files: {error}"),
                    &clock,
                )
            }
        }
    }

    /// A file event for a session whose list was fetched; other sessions
    /// fetch the whole list when the palette first opens.
    pub(super) fn files_changed(
        &self,
        state: &mut AppState,
        session: SessionId,
        change: FileChange,
    ) -> Effect<AppAction> {
        if !state.files_asked.contains(&session) {
            return Effect::none();
        }
        let files = std::sync::Arc::make_mut(&mut state.workspace)
            .session_files
            .entry(session)
            .or_default();
        let add = |files: &mut Vec<SessionFile>, file: SessionFile| {
            if !files.contains(&file) && files.len() < SESSION_FILES_CAP {
                files.push(file);
            }
        };
        match change {
            FileChange::Created(file) => add(files, file),
            FileChange::Deleted(file) => files.retain(|found| *found != file),
            FileChange::Renamed { from, to } => {
                files.retain(|found| *found != from);
                add(files, to);
            }
        }
        self.features_see_files(state)
    }

    /// Hands the workspace to the features that read session files: the
    /// palette's Files group, and the session page, which opens only files
    /// it knows of.
    fn features_see_files(&self, state: &mut AppState) -> Effect<AppAction> {
        let workspace = state.workspace.clone();
        let palette = self.child(state, AppAction::Palette(PaletteAction::Show(workspace.clone())));
        let session = self.child(state, AppAction::Session(SessionAction::Show(workspace)));
        Effect::merge([palette, session])
    }

    pub(super) fn show_shortcuts(&self, state: &mut AppState) -> Effect<AppAction> {
        if state.overlay() != Some(&Overlay::Shortcuts) {
            state.overlays.push(Overlay::Shortcuts);
        }
        Effect::none()
    }
}
