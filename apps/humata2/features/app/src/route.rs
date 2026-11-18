use humata2_board::BoardAction;
use humata2_clients::model::{ChannelId, Scope, SessionId};
use humata2_files::FilesAction;
use humata2_hosts::HostsAction;
use humata2_inbox::InboxAction;
use humata2_palette::PaletteAction;
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::SessionAction;
use humata2_sidebar::{RowKey, SidebarAction};
use humata2_views::ViewsAction;
use serde::{Deserialize, Serialize};
use tca::Effect;

use crate::{App, AppAction, AppState, TRAIL, channel};

/// A screen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Route {
    /// What needs the person, across channels (HM-1–5).
    #[default]
    Home,
    /// The Inbox.
    Inbox,
    /// A board.
    Board(Scope),
    /// A channel's page.
    Channel(ChannelId),
    /// A session's page.
    Session(SessionId),
    /// Hosts.
    Hosts,
    /// Files and transfers.
    Files,
    /// Prepared views.
    Views,
    /// The Desk: sessions outside any channel (DK-5).
    Desk,
    /// Settings.
    Settings,
    /// A machine's page (HO-4).
    Machine(humata2_clients::model::MachineId),
}

impl App {
    /// Hands the new workspace to every feature.
    pub(super) fn show(&self, state: &mut AppState) -> Effect<AppAction> {
        let workspace = &state.workspace;
        Effect::merge([
            self.child(
                state,
                AppAction::Sidebar(SidebarAction::Show(workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Session(SessionAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Review(ReviewAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Inbox(InboxAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Board(BoardAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Hosts(HostsAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Files(FilesAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Palette(PaletteAction::Show(state.workspace.clone())),
            ),
            self.child(
                state,
                AppAction::Desk(humata2_desk::DeskAction::Show(state.workspace.clone())),
            ),
        ])
    }

    pub(super) fn navigate(&self, state: &mut AppState, route: Route) -> Effect<AppAction> {
        if state.route != route {
            let previous = std::mem::replace(&mut state.route, route);
            state.trail.push(previous);
            if state.trail.len() > TRAIL {
                state.trail.remove(0);
            }
        }
        let arrived = self.arrive(state);
        self.desk_fresh(state, arrived)
    }

    pub(super) fn switch(&self, state: &mut AppState, route: Route) -> Effect<AppAction> {
        state.route = route;
        state.navigation_overlay = false;
        state.trail.clear();
        let arrived = self.arrive(state);
        self.desk_fresh(state, arrived)
    }

    /// Opening the Desk page reads the Desk again: the daemon sends no Desk
    /// events, and its scratch folders change under it (an agent writes a
    /// document). Not in `arrive`, which a load itself runs.
    fn desk_fresh(&self, state: &AppState, arrived: Effect<AppAction>) -> Effect<AppAction> {
        if state.route == Route::Desk {
            Effect::merge([arrived, self.load()])
        } else {
            arrived
        }
    }

    /// Tells the features what the route now shows.
    pub(super) fn arrive(&self, state: &mut AppState) -> Effect<AppAction> {
        // The sidebar's keyboard cursor walks the Desk rows only while they show.
        state.sidebar.desk_shown = state.desk_in_sidebar();
        // NV-37, SE-13: the side panel's collapsed state belongs to each
        // session. `session.open` is still the session being left here.
        if let Some(left) = &state.session.open {
            state
                .panel_collapsed
                .insert(left.clone(), state.layout.panel.collapsed);
        }
        if let Route::Session(id) = &state.route {
            state.layout.panel.collapsed = state.panel_collapsed.get(id).copied().unwrap_or(false);
        }
        let current = match &state.route {
            Route::Session(id) => Some(RowKey::Session(id.clone())),
            Route::Channel(id) => Some(RowKey::Channel(id.clone())),
            _ => None,
        };
        let mut effects = vec![self.child(
            state,
            AppAction::Sidebar(SidebarAction::SetCurrent(current)),
        )];
        match state.route.clone() {
            Route::Session(id) => {
                effects.push(self.child(state, AppAction::Views(ViewsAction::Return(id.clone()))));
                effects.push(self.follow_review(state, &id));
                effects
                    .push(self.child(state, AppAction::Session(SessionAction::Open(id.clone()))));
                effects.push(
                    self.child(state, AppAction::Comments(CommentsAction::Open(id.clone()))),
                );
                effects.push(self.child(state, AppAction::Review(ReviewAction::Open(id.clone()))));
                // The page's Files tree and the files it opens come from the
                // session's file list, fetched the first time it is shown.
                effects.push(self.ask_files(state, id));
            }
            Route::Channel(id) => {
                state.session.open = None;
                match state.channel_pages.get(&id).map(|v| v.tab) {
                    Some(channel::Tab::Inbox) => effects.push(self.child(
                        state,
                        AppAction::Inbox(InboxAction::SetFilter(humata2_inbox::Filter::Channel(
                            id,
                        ))),
                    )),
                    Some(channel::Tab::Board) => effects.push(self.child(
                        state,
                        AppAction::Board(BoardAction::SetScope(Scope::Channel(id))),
                    )),
                    _ => {}
                }
            }
            Route::Inbox => {
                state.session.open = None;
                effects.push(self.child(
                    state,
                    AppAction::Inbox(InboxAction::SetFilter(humata2_inbox::Filter::All)),
                ));
            }
            Route::Board(scope) => {
                effects.push(self.child(state, AppAction::Board(BoardAction::SetScope(scope))));
            }
            Route::Hosts => {
                state.session.open = None;
                effects.push(self.child(state, AppAction::Hosts(humata2_hosts::HostsAction::Opened)));
            }
            Route::Machine(id) => {
                state.session.open = None;
                effects.push(self.machine_page(
                    state,
                    humata2_hosts::MachinePageAction::Show(id),
                ));
            }
            Route::Files => {
                state.session.open = None;
                effects.push(self.child(state, AppAction::Files(FilesAction::Opened)));
            }
            _ => state.session.open = None,
        }
        // CH-16, CH-17 (D392): the daemon keeps the session shown read, and
        // only that one. Nothing comes back: a failure shows nowhere, and the
        // next arrival tells it again.
        let viewed = match &state.route {
            Route::Session(id) => Some(id.clone()),
            _ => None,
        };
        let daemon = self.clients.daemon.clone();
        effects.push(Effect::run(move |_send| async move {
            let _ = humata2_clients::Daemon::view(&*daemon, viewed.as_ref()).await;
        }));
        Effect::merge(effects)
    }

    pub(super) fn go_back(&self, state: &mut AppState) -> Effect<AppAction> {
        match state.trail.pop() {
            Some(route) => {
                state.route = route;
                self.arrive(state)
            }
            None => Effect::none(),
        }
    }

    /// Leaves `gone`, a session that was archived, for `to`. The session
    /// leaves the trail too, and so does a `to` at its end, so Back never
    /// reopens it or lands where the page already is (frame 32).
    pub(super) fn leave(&self, state: &mut AppState, gone: &Route, to: Route) -> Effect<AppAction> {
        state.trail.retain(|route| route != gone);
        while state.trail.last() == Some(&to) {
            state.trail.pop();
        }
        state.route = to;
        let arrived = self.arrive(state);
        self.desk_fresh(state, arrived)
    }
}
