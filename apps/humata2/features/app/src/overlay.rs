use humata2_board::BoardAction;
use humata2_inbox::InboxAction;
use humata2_palette::PaletteAction;
use humata2_review::comments::CommentsAction;
use humata2_session::{Panel, SessionAction};
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
    /// Keyboard shortcuts.
    Shortcuts,
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
        if state.add_machine.open {
            return self.add_machine(state, humata2_hosts::AddMachineAction::Cancel);
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
        if state.machine_page.menu_open {
            return self.child(
                state,
                AppAction::MachinePage(humata2_hosts::MachinePageAction::CloseMenu),
            );
        }
        if state.session.archiving.is_some() {
            return self.child(state, AppAction::Session(SessionAction::CancelArchive));
        }
        if state.session.moving.is_some() {
            return self.child(state, AppAction::Session(SessionAction::CancelMove));
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
        state.palette.context = match &state.route {
            Route::Session(id) => Some(id.clone()),
            _ => None,
        };
        state.palette.channel = match &state.route {
            Route::Channel(id) => Some(id.clone()),
            _ => None,
        };
        state.palette.marks = state.pinned.list.clone();
        self.child(
            state,
            AppAction::Palette(PaletteAction::Open { in_session }),
        )
    }

    pub(super) fn show_shortcuts(&self, state: &mut AppState) -> Effect<AppAction> {
        if state.overlay() != Some(&Overlay::Shortcuts) {
            state.overlays.push(Overlay::Shortcuts);
        }
        Effect::none()
    }
}
