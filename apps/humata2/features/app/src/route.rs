use humata2_board::BoardAction;
use humata2_clients::model::{ChannelId, Scope, SessionId};
use humata2_files::FilesAction;
use humata2_hosts::HostsAction;
use humata2_inbox::InboxAction;
use humata2_palette::PaletteAction;
use humata2_review::ReviewAction;
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
                AppAction::Views(ViewsAction::Show(state.workspace.clone())),
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
        self.arrive(state)
    }

    pub(super) fn switch(&self, state: &mut AppState, route: Route) -> Effect<AppAction> {
        state.route = route;
        state.navigation_overlay = false;
        state.trail.clear();
        self.arrive(state)
    }

    /// Tells the features what the route now shows.
    pub(super) fn arrive(&self, state: &mut AppState) -> Effect<AppAction> {
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
                effects
                    .push(self.child(state, AppAction::Session(SessionAction::Open(id.clone()))));
                effects.push(self.child(state, AppAction::Review(ReviewAction::Open(id))));
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
            Route::Files => {
                state.session.open = None;
                effects.push(self.child(state, AppAction::Files(FilesAction::Opened)));
            }
            _ => state.session.open = None,
        }
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
}
