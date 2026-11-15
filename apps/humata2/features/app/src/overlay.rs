use humata2_board::BoardAction;
use humata2_inbox::InboxAction;
use humata2_palette::PaletteAction;
use humata2_review::ReviewAction;
use humata2_session::{Panel, SessionAction};
use tca::Effect;

use crate::{App, AppAction, AppState, NewChannel, NewSession, Route};

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
        if let Some(top) = state.overlays.pop() {
            if top == Overlay::Palette {
                state.palette.open = false;
            }
            return Effect::none();
        }
        if state.navigation_overlay {
            state.navigation_overlay = false;
            return Effect::none();
        }
        if state.session.terminal_menu.is_some() {
            return self.child(state, AppAction::Session(SessionAction::TerminalMenu(None)));
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
        if state.review.draft().is_some() && state.panel() == Some(Panel::Changes) {
            return self.child(state, AppAction::Review(ReviewAction::CancelComment));
        }
        Effect::none()
    }

    pub(super) fn open_palette(&self, state: &mut AppState) -> Effect<AppAction> {
        if state.overlay() == Some(&Overlay::Palette) {
            return Effect::none();
        }
        state.overlays.push(Overlay::Palette);
        let in_session = matches!(state.route, Route::Session(_));
        state.palette.context = match &state.route {
            Route::Session(id) => Some(id.clone()),
            _ => None,
        };
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
