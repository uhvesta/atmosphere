use humata2_clients::model::Scope;
use humata2_palette::Command;
use humata2_session::SessionAction;
use humata2_sidebar::SidebarAction;
use tca::{Effect, Reducer};

use crate::layout::{FullScreen, PaneId};
use crate::{App, AppAction, AppState, ReportAction, ReportRequest, Route};

impl App {
    pub(super) fn run(&self, state: &mut AppState, command: Command) -> Effect<AppAction> {
        match command {
            Command::Home => self.navigate(state, Route::Home),
            Command::Inbox => self.navigate(state, Route::Inbox),
            Command::Board => self.navigate(state, Route::Board(Scope::Global)),
            Command::Hosts => self.navigate(state, Route::Hosts),
            Command::Files => self.navigate(state, Route::Files),
            Command::Desk => self.navigate(state, Route::Desk),
            Command::NewDeskSession => self.reduce(state, AppAction::StartNewDeskSession),
            Command::Settings => self.navigate(state, Route::Settings),
            Command::NewSession => self.reduce(state, AppAction::StartNewSession),
            Command::NewChannel => self.reduce(state, AppAction::StartNewChannel),
            Command::ExpandAll => self.child(state, AppAction::Sidebar(SidebarAction::ExpandAll)),
            Command::CollapseAll => {
                self.child(state, AppAction::Sidebar(SidebarAction::CollapseAll))
            }
            Command::ToggleSidebar => {
                state.layout.toggle(PaneId::Sidebar);
                Effect::none()
            }
            Command::Fork => {
                self.fork_unless_desk(state, humata2_clients::model::ForkKind::Worktree)
            }
            Command::Archive => self.archive_or_close(state),
            Command::Channel(id) => self.navigate(state, Route::Channel(id)),
            Command::Session(id) => self.navigate(state, Route::Session(id)),
            Command::Pull(number) => {
                let found = state
                    .workspace
                    .pulls
                    .iter()
                    .find(|pull| pull.number == number);
                match found.and_then(|pull| pull.session.clone()) {
                    Some(session) => self.navigate(state, Route::Session(session)),
                    None => {
                        let clock = self.clients.clock.clone();
                        state.toast(
                            format!(
                                "No session here opened pull request #{number}. Open it on GitHub, or start a session to review it."
                            ),
                            &clock,
                        )
                    }
                }
            }
            Command::TogglePanel => {
                state.layout.toggle(PaneId::Panel);
                Effect::none()
            }
            Command::TerminalFullScreen => {
                self.reduce(state, AppAction::ToggleFullScreen(FullScreen::Terminal))
            }
            Command::File {
                session,
                repo,
                path,
            } => {
                let go = self.navigate(state, Route::Session(session));
                let select = self.child(
                    state,
                    AppAction::Session(SessionAction::SelectFile { repo, path }),
                );
                let open = self.child(state, AppAction::Session(SessionAction::OpenFile));
                Effect::merge([go, select, open])
            }
            Command::Record => {
                forget(state, &Command::Record);
                self.report(state, ReportAction::Request(ReportRequest::Toggle))
            }
            Command::Snapshot => {
                forget(state, &Command::Snapshot);
                self.report(state, ReportAction::Request(ReportRequest::Snapshot))
            }
            Command::ReportBug => {
                forget(state, &Command::ReportBug);
                self.report(state, ReportAction::Request(ReportRequest::ReportBug))
            }
            Command::OpenLastReport => {
                forget(state, &Command::OpenLastReport);
                self.report(state, ReportAction::OpenLast)
            }
        }
    }
}

/// Takes a report command back out of the palette's recent picks: the
/// recording rewinds it out, so a replay could never put it back (D159).
fn forget(state: &mut AppState, command: &Command) {
    state.palette.recent.retain(|recent| recent != command);
}
