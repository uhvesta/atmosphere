use humata2_clients::model::{MachineId, Scope, SessionId};
use humata2_find::PinnedAction;
use humata2_palette::Command;
use humata2_settings::SettingsAction;
use humata2_session::{Panel, SessionAction};
use humata2_sidebar::SidebarAction;
use tca::{Effect, Reducer};

use crate::layout::{FullScreen, PaneId};
use crate::{App, AppAction, AppState, ReportAction, ReportRequest, Route};

impl App {
    /// Sends a review step when the open session's panel shows the Diff.
    fn step_review(
        &self,
        state: &mut AppState,
        step: humata2_review::ReviewAction,
    ) -> Effect<AppAction> {
        if matches!(state.panel(), Some(Panel::Changes)) {
            self.child(state, AppAction::Review(step))
        } else {
            Effect::none()
        }
    }

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
            Command::RenameSession => {
                self.child(state, AppAction::Session(SessionAction::StartRename))
            }
            Command::StopAgent => self.child(state, AppAction::Session(SessionAction::StopAgent)),
            Command::ContinueInChannel => {
                self.child(state, AppAction::Session(SessionAction::StartContinue))
            }
            Command::Channel(id) => self.navigate(state, Route::Channel(id)),
            Command::Session(id) => self.navigate(state, Route::Session(id)),
            // D784: a terminal found with Search opens in its tab, shown
            // again if it was hidden.
            Command::Terminal { session, tab } => self.child(
                state,
                AppAction::Session(SessionAction::Close(humata2_session::CloseAction::Then(
                    humata2_session::CloseThen::Open { session, tab },
                ))),
            ),
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
            Command::NewTerminalTab => {
                self.reduce(state, AppAction::Session(SessionAction::NewTerminalTab))
            }
            Command::TerminalFullScreen => {
                self.reduce(state, AppAction::ToggleFullScreen(FullScreen::Terminal))
            }
            Command::PanelFullScreen => {
                // A session with no panel open gets the Diff first.
                let open = if matches!(state.route, Route::Session(_)) && state.panel().is_none() {
                    self.reduce(
                        state,
                        AppAction::Session(SessionAction::SetPanel(Some(Panel::Changes))),
                    )
                } else {
                    Effect::none()
                };
                let widen = self.reduce(state, AppAction::Session(SessionAction::ToggleWiden));
                Effect::merge([open, widen])
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
            Command::JoinTabGroups => {
                self.child(state, AppAction::Session(SessionAction::CloseGroup(1)))
            }
            Command::NextLayer => {
                self.child(state, AppAction::Review(humata2_review::ReviewAction::NextLayer))
            }
            Command::PrevLayer => {
                self.child(state, AppAction::Review(humata2_review::ReviewAction::PrevLayer))
            }
            Command::WholeStack => self.child(
                state,
                AppAction::Review(humata2_review::ReviewAction::PickLayer(None)),
            ),
            // The Diff panel's own steps; with another panel showing, a key
            // or a palette pick leaves the review where it is.
            Command::NextFile => self.step_review(state, humata2_review::ReviewAction::NextFile),
            Command::PrevFile => self.step_review(state, humata2_review::ReviewAction::PrevFile),
            Command::NextChange => {
                self.step_review(state, humata2_review::ReviewAction::NextChange)
            }
            Command::PrevChange => {
                self.step_review(state, humata2_review::ReviewAction::PrevChange)
            }
            Command::Back => self.go_back(state),
            Command::ShowShortcuts => self.show_shortcuts(state),
            Command::Views => self.navigate(state, Route::Views),
            Command::Todo => self.navigate(state, Route::Todo),
            Command::Machine(id) => self.navigate(state, Route::Machine(id)),
            Command::ChannelBoard(id) => self.navigate(state, Route::Board(Scope::Channel(id))),
            // The palette asks for the machine itself and sends MoveTo, so
            // this is ⌥⌘M with no session open.
            Command::MoveSession => {
                let clock = self.clients.clock.clone();
                state.toast(
                    "Nothing to move: open a session, then press ⌥⌘M to move it to another machine."
                        .to_owned(),
                    &clock,
                )
            }
            Command::MoveTo { session, machine } => self.move_session(state, session, machine),
            // REPOS (RL-5, KP-4): the channel's Add a GitHub repo sheet, prefilled.
            // The sheet is the app's own, not a child feature's.
            Command::AddRepoToChannel { channel, repo } => {
                let open =
                    self.change_channel_add_repo(state, crate::ChannelAddRepoAction::Open(channel));
                let fill =
                    self.change_channel_add_repo(state, crate::ChannelAddRepoAction::Query(repo));
                open.and(fill)
            }
            Command::ReviewPull(pull) => self.review_pull(state, &pull),
            Command::SettingsSection(section) => {
                let go = self.navigate(state, Route::Settings);
                go.and(self.child(state, AppAction::Settings(SettingsAction::Section(section))))
            }
            Command::SettingsRow(row) => {
                let go = self.navigate(state, Route::Settings);
                go.and(self.child(state, AppAction::Settings(SettingsAction::Reveal(row))))
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
            Command::Pin { thing, on } => self.child(
                state,
                AppAction::Pinned(PinnedAction::Toggle {
                    thing: thing.to_thing(),
                    pin: on,
                }),
            ),
            Command::Vip { thing, on } => self.child(
                state,
                AppAction::Pinned(PinnedAction::Vip {
                    thing: thing.to_thing(),
                    on,
                }),
            ),
        }
    }

    fn move_session(&self, state: &mut AppState, session: SessionId, machine: MachineId) -> Effect<AppAction> {
        let desk = state.workspace.session(&session).is_some_and(humata2_clients::desk::is_desk);
        if desk {
            return self.child(state, AppAction::Desk(humata2_desk::DeskAction::Move(session, machine)));
        }
        // A channel session moves through its own Move dialog (frames 63/64): the
        // palette already picked the machine, so open the session, pick it and confirm.
        let go = self.navigate(state, Route::Session(session));
        let start = self.child(state, AppAction::Session(SessionAction::StartMove));
        let pick = self.child(state, AppAction::Session(SessionAction::MovePick(machine)));
        let confirm = self.child(state, AppAction::Session(SessionAction::ConfirmMove));
        Effect::merge([go, start, pick, confirm])
    }
}

/// Takes a report command back out of the palette's recent picks: the
/// recording rewinds it out, so a replay could never put it back (D159).
fn forget(state: &mut AppState, command: &Command) {
    state.palette.recent.retain(|recent| recent != command);
}
