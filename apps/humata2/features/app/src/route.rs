use humata2_clients::model::{ChannelId, ItemId, SessionId};
use humata2_palette::PullRef;
use humata2_files::FilesAction;
use humata2_hosts::HostsAction;
use humata2_inbox::InboxAction;
use humata2_palette::PaletteAction;
use humata2_review::ReviewAction;
use humata2_review::comments::CommentsAction;
use humata2_session::SessionAction;
use humata2_todo_feature::TodoAction;
use humata2_sidebar::{RowKey, SidebarAction};
use humata2_views::ViewsAction;
use serde::{Deserialize, Serialize};
use tca::Effect;

use crate::{App, AppAction, AppState, TRAIL, channel};

/// A screen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SavedRoute")]
pub enum Route {
    /// What needs the person, across channels (HM-1–5).
    #[default]
    Home,
    /// The Inbox.
    Inbox,
    /// Dedicated Reviews inbox (local stacks + GitHub PRs).
    Reviews,
    /// The To-do list.
    Todo,
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

/// A route as an earlier build saved it. The board page is gone (D874):
/// a saved board opens the To-do list, so the rest of what was saved
/// still restores.
#[derive(Deserialize)]
enum SavedRoute {
    Home,
    Inbox,
    Reviews,
    Board(serde::de::IgnoredAny),
    Todo,
    Channel(ChannelId),
    Session(SessionId),
    Hosts,
    Files,
    Views,
    Desk,
    Settings,
    Machine(humata2_clients::model::MachineId),
}

impl From<SavedRoute> for Route {
    fn from(saved: SavedRoute) -> Self {
        match saved {
            SavedRoute::Home => Self::Home,
            SavedRoute::Inbox => Self::Inbox,
            SavedRoute::Reviews => Self::Reviews,
            SavedRoute::Board(_) | SavedRoute::Todo => Self::Todo,
            SavedRoute::Channel(id) => Self::Channel(id),
            SavedRoute::Session(id) => Self::Session(id),
            SavedRoute::Hosts => Self::Hosts,
            SavedRoute::Files => Self::Files,
            SavedRoute::Views => Self::Views,
            SavedRoute::Desk => Self::Desk,
            SavedRoute::Settings => Self::Settings,
            SavedRoute::Machine(id) => Self::Machine(id),
        }
    }
}

impl App {
    /// Hands the new workspace to every feature.
    pub(super) fn show(&self, state: &mut AppState) -> Effect<AppAction> {
        state.sync_todo_channels();
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
            self.child(
                state,
                AppAction::MassDelete(humata2_mass_delete::MassDeleteAction::Show(
                    mass_delete_listing(&state.workspace),
                )),
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
            state.forward_trail.clear();
        }
        let arrived = self.arrive(state);
        self.desk_fresh(state, arrived)
    }

    pub(super) fn switch(&self, state: &mut AppState, route: Route) -> Effect<AppAction> {
        state.route = route;
        state.navigation_overlay = false;
        state.trail.clear();
        state.forward_trail.clear();
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

    /// A channel's To-do tab is the To-do list narrowed to that channel
    /// (D874). The list loads once and is followed, as on the page.
    pub(super) fn channel_todo(&self, state: &mut AppState, id: ChannelId) -> Vec<Effect<AppAction>> {
        let filter = humata2_todo_feature::ViewFilter {
            channel: Some(humata2_todo_feature::ChannelPick::Channel(id.as_str().to_owned())),
            ..Default::default()
        };
        vec![
            self.child(state, AppAction::Todo(TodoAction::SetFilter(filter))),
            self.child(state, AppAction::Todo(TodoAction::Load)),
            self.child(state, AppAction::Todo(TodoAction::Watch)),
        ]
    }

    /// Tells the features what the route now shows.
    pub(super) fn arrive(&self, state: &mut AppState) -> Effect<AppAction> {
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
        let surface = match &state.route {
            Route::Machine(id) => humata2_mass_delete::Surface::Machine(id.clone()),
            Route::Home => humata2_mass_delete::Surface::Home,
            _ => humata2_mass_delete::Surface::Sidebar,
        };
        let mut effects = vec![
            self.child(state, AppAction::Sidebar(SidebarAction::SetCurrent(current))),
            self.child(
                state,
                AppAction::MassDelete(humata2_mass_delete::MassDeleteAction::SurfaceChanged(
                    surface,
                )),
            ),
        ];
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
                // The session's To-do panel reads the list; it is fetched the
                // first time a session shows it.
                if !state.todo.loaded {
                    effects.push(self.child(state, AppAction::Todo(TodoAction::Load)));
                    effects.push(self.child(state, AppAction::Todo(TodoAction::Watch)));
                }
            }
            Route::Channel(id) => {
                state.session.open = None;
                match state.channel_pages.get(&id).map(|v| v.tab).unwrap_or_default() {
                    channel::Tab::Desk => {
                        effects.push(self.ensure_channel_desk(state, &id, false));
                    }
                    channel::Tab::Inbox => effects.push(self.child(
                        state,
                        AppAction::Inbox(InboxAction::SetFilter(humata2_inbox::Filter::Channel(
                            id,
                        ))),
                    )),
                    channel::Tab::Board => effects.extend(self.channel_todo(state, id)),
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
            Route::Reviews => {
                state.session.open = None;
                if !state.todo.loaded {
                    effects.push(self.child(state, AppAction::Todo(TodoAction::Load)));
                    effects.push(self.child(state, AppAction::Todo(TodoAction::Watch)));
                }
            }
            Route::Todo => {
                state.session.open = None;
                effects.push(self.child(state, AppAction::Todo(TodoAction::Load)));
                // Follows changes made elsewhere; a second entry replaces the watch.
                effects.push(self.child(state, AppAction::Todo(TodoAction::Watch)));
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

    /// Do next: the New session sheet, filled from the item `do_next` names
    /// (frame b4-3). An old sheet from a skipped item is replaced.
    pub(super) fn fill_do_next(&self, state: &mut AppState) -> Effect<AppAction> {
        state
            .overlays
            .retain(|overlay| !matches!(overlay, crate::Overlay::NewSession(_)));
        let Some(entry) = state
            .todo
            .do_next
            .as_ref()
            .and_then(|key| state.todo.items.iter().find(|e| &e.todo.key == key))
            .cloned()
        else {
            return Effect::none();
        };
        let effect = self.start_new_session(state);
        let item = entry.todo;
        let channel = item
            .channel
            .as_ref()
            .and_then(|name| {
                state
                    .workspace
                    .channels
                    .iter()
                    .find(|c| c.id.as_str() == name.as_str() || &c.name == name)
            })
            .map(|c| c.id.clone());
        let machine = channel.as_ref().and_then(|id| {
            state.workspace.channel_machines(id).find(|m| m.online).map(|m| m.id.clone())
        });
        let first_message = if item.description.trim().is_empty() {
            item.title.clone()
        } else {
            format!("{}\n\n{}", item.title, item.description.trim())
        };
        if let Some(crate::Overlay::NewSession(sheet)) = state.overlays.last_mut() {
            sheet.title = item.title.clone();
            sheet.first_message = first_message;
            sheet.item = Some(ItemId::new(item.key.as_str()));
            if let Some(channel) = channel {
                sheet.channel = Some(channel);
                if machine.is_some() {
                    sheet.machine = machine;
                }
            }
        }
        effect
    }

    /// Start the review of a pull request review item: the Review a pull
    /// request dialog opens on its first pull request, the rest under "Also
    /// review" (frame b7-1).
    pub(super) fn start_todo_review(&self, state: &mut AppState, key: &str) -> Effect<AppAction> {
        let prs = state
            .todo
            .items
            .iter()
            .find(|e| e.todo.key == key)
            .and_then(|e| e.todo.review.as_ref())
            .map(|r| r.repo_prs.clone())
            .unwrap_or_default();
        let mut prs = prs.into_iter();
        let Some(first) = prs.next() else {
            return Effect::none();
        };
        let open = match parse_pull(&first) {
            Some(pull) => self.review_pull(state, &pull),
            None => Effect::none(),
        };
        let also: Vec<_> = prs
            .map(|pr| self.child(state, AppAction::PrDialog(humata2_pr_review::dialog::DialogAction::AddAlso(pr))))
            .collect();
        open.and(Effect::merge(also))
    }

    pub(super) fn go_back(&self, state: &mut AppState) -> Effect<AppAction> {
        match state.trail.pop() {
            Some(route) => {
                let current = std::mem::replace(&mut state.route, route);
                state.forward_trail.push(current);
                if state.forward_trail.len() > TRAIL {
                    state.forward_trail.remove(0);
                }
                self.arrive(state)
            }
            None => Effect::none(),
        }
    }

    pub(super) fn go_forward(&self, state: &mut AppState) -> Effect<AppAction> {
        match state.forward_trail.pop() {
            Some(route) => {
                let current = std::mem::replace(&mut state.route, route);
                state.trail.push(current);
                if state.trail.len() > TRAIL {
                    state.trail.remove(0);
                }
                self.arrive(state)
            }
            None => Effect::none(),
        }
    }

    /// Refreshes the active route or open session side panel (⌘R).
    pub(super) fn refresh_active(&self, state: &mut AppState) -> Effect<AppAction> {
        match state.route.clone() {
            Route::Session(id) => match state.panel() {
                Some(humata2_session::Panel::Changes) => Effect::merge([
                    self.child(state, AppAction::Review(ReviewAction::Open(id.clone()))),
                    self.child(state, AppAction::Comments(CommentsAction::Open(id))),
                ]),
                Some(humata2_session::Panel::Files) => self.refresh_files(state, id),
                Some(humata2_session::Panel::Board) => {
                    self.child(state, AppAction::Todo(TodoAction::Load))
                }
                _ => self.load(),
            },
            Route::Hosts => Effect::merge([
                self.load(),
                self.child(state, AppAction::Hosts(HostsAction::Opened)),
            ]),
            Route::Machine(_) => self.machine_page(state, humata2_hosts::MachinePageAction::Reload),
            Route::Files => self.child(state, AppAction::Files(FilesAction::Opened)),
            Route::Todo => self.child(state, AppAction::Todo(TodoAction::Load)),
            Route::Channel(id) => {
                if state
                    .channel_pages
                    .get(&id)
                    .map(|view| view.tab)
                    .unwrap_or_default()
                    == channel::Tab::Board
                {
                    Effect::merge(self.channel_todo(state, id))
                } else {
                    self.load()
                }
            }
            _ => self.load(),
        }
    }

    /// Leaves `gone`, a session that was archived, for `to`. The session
    /// leaves the trail too, and so does a `to` at its end, so Back never
    /// reopens it or lands where the page already is (frame 32).
    pub(super) fn leave(&self, state: &mut AppState, gone: &Route, to: Route) -> Effect<AppAction> {
        state.trail.retain(|route| route != gone);
        state.forward_trail.retain(|route| route != gone);
        while state.trail.last() == Some(&to) {
            state.trail.pop();
        }
        while state.forward_trail.last() == Some(&to) {
            state.forward_trail.pop();
        }
        state.route = to;
        let arrived = self.arrive(state);
        self.desk_fresh(state, arrived)
    }
}

/// "acme/api#483" as a pull request reference.
fn parse_pull(text: &str) -> Option<PullRef> {
    let (repo, number) = text.split_once('#')?;
    let (owner, name) = repo.split_once('/')?;
    Some(PullRef {
        owner: owner.to_owned(),
        repo: name.to_owned(),
        number: number.parse().ok()?,
    })
}

fn mass_delete_listing(workspace: &humata2_clients::Workspace) -> humata2_mass_delete::Listing {
    let rows = workspace
        .sessions
        .iter()
        .map(|s| humata2_mass_delete::Row {
            id: s.id.clone(),
            channel: s.channel.clone(),
            machine: s.machine.clone(),
            context_fork_of: (s.fork == Some(humata2_clients::model::ForkKind::Conversation))
                .then(|| s.parent.clone())
                .flatten(),
            archived: s.archived,
            live: !s.archived,
            desk: humata2_clients::desk::is_desk(s),
        })
        .collect();
    let offline = workspace
        .machines
        .iter()
        .filter(|m| !m.online)
        .map(|m| m.id.clone())
        .collect();
    let machine_names = workspace
        .machines
        .iter()
        .map(|m| (m.id.clone(), m.name.clone()))
        .collect();
    humata2_mass_delete::Listing {
        rows,
        offline,
        machine_names,
    }
}
