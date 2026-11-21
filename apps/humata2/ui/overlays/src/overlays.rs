//! What opens over the screen: the command palette (NV-6–10), the New
//! session (SE-1) and New channel (CH-1) sheets, the shortcuts, and toasts.
//!
//! The top overlay takes the keyboard: it is a focus trap, so Tab stays in
//! it, and when it closes focus goes back where it was. Clicking the scrim
//! or pressing Escape closes it.

use std::rc::Rc;
use std::time::Duration;

use humata2_ui_core::action_menu::{ENTRANCE_MS, ease_out_quint};
use humata2_ui_core::theme::Look;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, NewChannelAction, NewSessionAction, Overlay};
use humata2_hosts::HostsAction;
use humata2_palette::PaletteAction;
use humata2_session::SessionAction;

use humata2_ui_core::bridge::{Part, Ui};
use humata2_ui_core::theme;
use humata2_ui_core::widgets::col;

#[path = "overlay_add_machine.rs"]
mod add_machine;
#[path = "overlay_desk.rs"]
mod desk;
// MASSDELETE (D875): the delete sheet for many sessions.
#[path = "overlay_delete_sessions.rs"]
mod delete_sessions;
#[path = "overlay_archive.rs"]
mod archive;
#[path = "overlay_close.rs"]
mod close;
#[path = "overlay_continue.rs"]
mod continue_dialog;
// REPOS (CH-6): a channel's Add a GitHub repo sheet.
#[path = "overlay_channel_add_repo.rs"]
mod channel_add_repo;
#[path = "overlay_fork.rs"]
mod fork;
#[path = "overlay_move.rs"]
mod move_dialog;
#[path = "overlay_new_channel.rs"]
mod new_channel;
#[path = "overlay_new_session.rs"]
mod new_session;
#[path = "overlay_palette.rs"]
mod palette;
// PR (frames 67, 69): Review a pull request and Submit review.
#[path = "overlay_pr_review.rs"]
mod pr_review;
#[path = "overlay_rename.rs"]
mod rename;
#[path = "overlay_root.rs"]
mod root;
#[path = "overlay_running.rs"]
mod running;
#[path = "overlay_session_add_repo.rs"]
mod session_add_repo;
#[path = "overlay_shortcuts.rs"]
mod shortcuts;
#[path = "overlay_toasts.rs"]
mod toasts;

/// Which overlay shows, to tell when another opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    AddMachine,
    Archive,
    Close,
    Continue,
    DeleteSessions,
    Desk,
    Fork,
    Move,
    MachineStop,
    MachineStart,
    Palette,
    NewSession,
    NewChannel,
    ChannelAddRepo,
    PrReview,
    SubmitReview,
    Rename,
    RootChange,
    RunningEnd,
    RunningAdopt,
    SessionAddRepo,
    Shortcuts,
}

const fn kind(overlay: &Overlay) -> Kind {
    match overlay {
        Overlay::Palette => Kind::Palette,
        Overlay::NewSession(_) => Kind::NewSession,
        Overlay::NewChannel(_) => Kind::NewChannel,
        Overlay::ChannelAddRepo(_) => Kind::ChannelAddRepo,
        Overlay::SessionAddRepo(_) => Kind::SessionAddRepo,
        Overlay::Shortcuts => Kind::Shortcuts,
    }
}

/// The overlays' view.
pub struct Overlays {
    ui: Rc<Ui>,
    query: Entity<InputState>,
    palette_scroll: ScrollHandle,
    palette_cursor: Option<usize>,
    palette_query: String,
    title: Entity<InputState>,
    fork_branch: Entity<InputState>,
    first_message: Entity<TextareaState>,
    first_message_placeholder: String,
    name: Entity<InputState>,
    repos: Entity<InputState>,
    // REPOS (RL-8): the + Org field.
    owner: Entity<InputState>,
    // REPOS (RL-8): whether + Org was open when last drawn, to focus it as it opens.
    owner_open: std::cell::Cell<bool>,
    // REPOS (CH-21): the Add repo to this session sheet's query field.
    session_repo: Entity<InputState>,
    // REPOS (CH-6): the channel's Add a GitHub repo field.
    channel_add_repo: channel_add_repo::Inputs,
    rename_title: Entity<InputState>,
    root_text: Entity<InputState>,
    // PR: the Review a pull request and Submit review dialogs' fields.
    pr: pr_review::Inputs,
    sheet: FocusHandle,
    desk: desk::Inputs,
    delete_sessions: delete_sessions::Inputs,
    add_machine: add_machine::AddMachineDialog,
    shown: Option<Kind>,
    restore: Option<FocusHandle>,
    restore_target: Option<humata2_ui_core::bridge::FocusTarget>,
    _subscriptions: Vec<Subscription>,
}

impl Overlays {
    /// The overlays on `ui`.
    pub fn new(ui: Rc<Ui>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ui.attach(Part::Overlay, cx.entity_id());
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Type a command, a channel or a session")
        });
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("What should it do?"));
        let fork_branch =
            cx.new(|cx| InputState::new(window, cx).placeholder("uhvesta/ledger-schema-2"));
        let first_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Optional. Sent to the agent on start.")
                .rows(3)
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("web"));
        let repos = cx.new(|cx| InputState::new(window, cx).placeholder("Search repos"));
        let owner = cx.new(|cx| InputState::new(window, cx).placeholder("Org or user"));
        let rename_title = cx.new(|cx| InputState::new(window, cx).placeholder("Session title"));
        let root_text = cx.new(|cx| InputState::new(window, cx).placeholder("~/Humata"));
        let mut subscriptions = Vec::new();
        for (input, change, enter) in [
            (
                &query,
                (|text: String| AppAction::Palette(PaletteAction::Query(text)))
                    as fn(String) -> AppAction,
                AppAction::Palette(PaletteAction::Run),
            ),
            (
                &title,
                |text| AppAction::NewSession(NewSessionAction::Title(text)),
                AppAction::NewSession(NewSessionAction::Create),
            ),
            (
                &fork_branch,
                |text| AppAction::Session(SessionAction::ForkBranch(text)),
                AppAction::Session(SessionAction::ConfirmFork),
            ),
            (
                &name,
                |text| AppAction::NewChannel(NewChannelAction::Name(text)),
                AppAction::NewChannel(NewChannelAction::Create),
            ),
            (
                &repos,
                |text| AppAction::NewChannel(NewChannelAction::Query(text)),
                AppAction::NewChannel(NewChannelAction::Create),
            ),
            (
                &rename_title,
                |text| AppAction::Session(SessionAction::RenameTitle(text)),
                AppAction::Session(SessionAction::ConfirmRename),
            ),
            (
                &root_text,
                |text| AppAction::Hosts(HostsAction::RootText(text)),
                AppAction::Hosts(HostsAction::PlanRoot),
            ),
        ] {
            subscriptions.push(cx.subscribe(
                input,
                move |this: &mut Self, input, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.ui.send(change(text), cx);
                    }
                    InputEvent::PressEnter { .. } => this.ui.send(enter.clone(), cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ));
        }
        subscriptions.push(cx.subscribe(
            &first_message,
            |this: &mut Self, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.ui.send(
                        AppAction::NewSession(NewSessionAction::FirstMessage(
                            input.read(cx).value().to_string(),
                        )),
                        cx,
                    );
                }
                InputEvent::PressEnter { secondary: true, .. } => {
                    this.ui
                        .send(AppAction::NewSession(NewSessionAction::Create), cx);
                }
                _ => {}
            },
        ));
        // REPOS (RL-8): the + Org field belongs to the picker on screen.
        subscriptions.push(cx.subscribe(
            &owner,
            |this: &mut Self, input, event: &InputEvent, cx| {
                let Some(picker) = this.ui.read(humata2_app::RepoPicker::on_screen) else {
                    return;
                };
                let action = match event {
                    InputEvent::Change => humata2_app::RepoSearchAction::OwnerTextChanged(
                        input.read(cx).value().to_string(),
                    ),
                    InputEvent::PressEnter { .. } => humata2_app::RepoSearchAction::OwnerEntered,
                    InputEvent::Focus | InputEvent::Blur => return,
                };
                this.ui.send(AppAction::RepoSearch(picker, action), cx);
            },
        ));
        let channel_add_repo = channel_add_repo::Inputs::new(window, cx, &mut subscriptions);
        let session_repo = session_add_repo::query_input(window, cx, &mut subscriptions);
        let desk = desk::Inputs::new(window, cx, &mut subscriptions);
        let delete_sessions = delete_sessions::Inputs::new(window, cx, &mut subscriptions);
        let pr = pr_review::Inputs::new(window, cx, &mut subscriptions);
        let add_machine =
            add_machine::AddMachineDialog::new(ui.clone(), AppAction::AddMachine, window, cx);
        Self {
            ui,
            query,
            palette_scroll: ScrollHandle::new(),
            palette_cursor: None,
            palette_query: String::new(),
            title,
            fork_branch,
            first_message,
            first_message_placeholder: "Optional. Sent to the agent on start.".into(),
            name,
            repos,
            owner,
            owner_open: std::cell::Cell::new(false),
            channel_add_repo,
            rename_title,
            root_text,
            pr,
            session_repo,
            sheet: cx.focus_handle(),
            desk,
            delete_sessions,
            add_machine,
            shown: None,
            restore: None,
            restore_target: None,
            _subscriptions: subscriptions,
        }
    }

    /// Moves focus into the overlay that just opened, or back where it was
    /// before the last one closed.
    fn follow_focus(&mut self, top: Option<Kind>, window: &mut Window, cx: &mut Context<Self>) {
        if top == self.shown {
            return;
        }
        if self.shown.is_none() {
            self.restore = window.focused(cx);
            self.restore_target = Some(self.ui.read(humata2_ui_core::bridge::focus_target));
        }
        self.shown = top;
        self.owner_open.set(false);
        if top != Some(Kind::Palette) {
            self.palette_cursor = None;
            self.palette_query.clear();
        }
        match top {
            Some(Kind::Palette) => self.query.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::NewSession) => self.title.update(cx, |input, cx| input.focus(window, cx)),
            // Return forks from the dialog itself (D873).
            Some(Kind::Fork | Kind::Move | Kind::Continue | Kind::Archive | Kind::Close | Kind::RunningEnd | Kind::RunningAdopt) => {
                window.focus(&self.sheet, cx);
            }
            Some(Kind::Rename) => self
                .rename_title
                .update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::NewChannel) => self.name.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::RootChange) => self.root_text.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::ChannelAddRepo) => self.channel_add_repo_focus(window, cx),
            Some(Kind::SessionAddRepo) => self
                .session_repo
                .update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::Shortcuts) => window.focus(&self.sheet, cx),
            Some(kind @ (Kind::PrReview | Kind::SubmitReview)) => self.pr_focus(kind, window, cx),
            Some(Kind::Desk) => self.desk_focus(window, cx),
            Some(Kind::DeleteSessions) => self.delete_sessions_focus(window, cx),
            Some(Kind::AddMachine) => self.add_machine.focus(window, cx),
            // The buttons take focus as they are clicked or tabbed to.
            Some(Kind::MachineStop | Kind::MachineStart) => {}
            None => {
                let same_target = self.restore_target.take().as_ref()
                    == Some(&self.ui.read(humata2_ui_core::bridge::focus_target));
                if let Some(focus) = self.restore.take()
                    && same_target
                {
                    window.focus(&focus, cx);
                }
            }
        }
    }
}

impl Overlays {
    /// Shows what is being typed after **+ Org** (`None`: it is closed), and
    /// moves focus into the field the moment it opens (RL-8).
    fn sync_owner(&self, adding: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        sync(&self.owner, adding.unwrap_or_default(), window, cx);
        let was_open = self.owner_open.replace(adding.is_some());
        if adding.is_some() && !was_open {
            self.owner.update(cx, |input, cx| input.focus(window, cx));
        }
    }
}

fn sync(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    let last = window.use_keyed_state(
        ("overlay-sync", input.entity_id()),
        cx,
        |_, _| String::new(),
    );
    if last.read(cx).as_str() == value {
        return;
    }
    last.update(cx, |synced, _| {
        synced.clear();
        synced.push_str(value);
    });
    if input.read(cx).value().as_ref() != value {
        let value = value.to_owned();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

/// The Stop or Start sessions confirm of a machine, from the machine page or
/// the Hosts page: drawn over the whole window like the other dialogs.
enum MachineConfirm {
    Stop { name: String, sessions: usize },
    Start { name: String, sessions: usize, at: Option<String>, from_hosts: bool },
}

impl MachineConfirm {
    const fn kind(&self) -> Kind {
        match self {
            Self::Stop { .. } => Kind::MachineStop,
            Self::Start { .. } => Kind::MachineStart,
        }
    }

    fn card(&self, ui: &Rc<Ui>, look: Look) -> impl IntoElement {
        use humata2_hosts::HostsAction as H;
        use humata2_hosts::page::MachinePageAction as P;
        match self {
            Self::Stop { name, sessions } => humata2_ui_machines::machine_page::stop_confirm(
                ui,
                name,
                *sessions,
                AppAction::MachinePage(P::DismissConfirm),
                AppAction::MachinePage(P::ConfirmSessions),
                look,
            )
            .test_support(),
            Self::Start { name, sessions, at, from_hosts } => {
                let (cancel, go) = if *from_hosts {
                    (AppAction::Hosts(H::DismissStart), AppAction::Hosts(H::ConfirmStart))
                } else {
                    (
                        AppAction::MachinePage(P::DismissConfirm),
                        AppAction::MachinePage(P::ConfirmSessions),
                    )
                };
                humata2_ui_machines::machine_page::start_confirm(ui, name, *sessions, at.as_deref(), cancel, go, look)
                    .test_support()
            }
        }
    }
}

/// The machine confirm that is open, if any: the Hosts page's Start first
/// (it sits over the list), then the machine page's Stop or Start.
fn machine_confirm(state: &humata2_app::AppState) -> Option<MachineConfirm> {
    use humata2_hosts::page::Confirm;
    if let Some(id) = state.hosts.start_confirm.as_ref() {
        let machine = state.workspace.machine(id)?;
        let sessions = state
            .workspace
            .sessions
            .iter()
            .filter(|s| &s.machine == id && !s.archived)
            .count();
        return Some(MachineConfirm::Start {
            name: machine.name.clone(),
            sessions,
            at: machine.stop.as_ref().map(|stop| stop.at.clone()),
            from_hosts: true,
        });
    }
    let page = &state.machine_page;
    let confirm = page.confirm.as_ref()?;
    let id = page.machine.as_ref()?;
    let name = state
        .workspace
        .machine(id)
        .map_or_else(|| id.to_string(), |machine| machine.name.clone());
    let sessions = page.detail.as_ref().map_or(0, |detail| detail.sessions.len());
    Some(match confirm {
        Confirm::Stop => MachineConfirm::Stop { name, sessions },
        Confirm::Start => MachineConfirm::Start {
            name,
            sessions,
            at: page.stop().map(|stop| stop.at.clone()),
            from_hosts: false,
        },
    })
}

fn sheet(id: &'static str, label: &'static str, look: Look) -> Stateful<Div> {
    col()
        .id(id)
        .role(Role::Dialog)
        .aria_label(label)
        .w(px(theme::SHEET))
        .max_h(relative(0.8))
        .overflow_y_scroll()
        .gap_3()
        .p(px(22.))
        .rounded(px(12.))
        .bg(rgb(look.card))
        .text_color(rgb(look.text))
        .border(px(0.5))
        .border_color(look.hairline())
        .shadow(theme::shadow_sheet(look))
}

impl Render for Overlays {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::of(cx);
        let (top, palette, toasts) = self.ui.read(|state| {
            (
                state.overlay().cloned(),
                state.palette.clone(),
                state.toasts.clone(),
            )
        });
        let (fork, moving, archiving, forking, fork_error, parent) = self.ui.read(|state| {
            (
                state.session.fork.clone(),
                state.session.moving.clone(),
                state.session.archiving.clone(),
                state.session.forking,
                state.session.error.clone(),
                state
                    .session
                    .open
                    .as_ref()
                    .and_then(|id| state.workspace.session(id))
                    .cloned(),
            )
        });
        let renaming = self.ui.read(|state| state.session.renaming.clone());
        let closing = self.ui.read(|state| state.session.closing.clone());
        let continuing = self.ui.read(|state| state.session.continuing.clone());
        let machine_confirm = self.ui.read(machine_confirm);
        let add_machine = self
            .ui
            .read(|state| state.add_machine.open.then(|| state.add_machine.clone()));
        let root_change = self.ui.read(root::RootView::read);
        let running = self.ui.read(running::RunningView::read);
        let delete_view = self.ui.read(delete_sessions::View::read);
        let shown = top
            .as_ref()
            .map(kind)
            .or_else(|| closing.as_ref().map(|_| Kind::Close))
            .or_else(|| archiving.as_ref().map(|_| Kind::Archive))
            .or_else(|| renaming.as_ref().map(|_| Kind::Rename))
            .or_else(|| moving.as_ref().map(|_| Kind::Move))
            .or_else(|| continuing.as_ref().map(|_| Kind::Continue))
            .or_else(|| fork.as_ref().map(|_| Kind::Fork))
            .or_else(|| self.desk_open().then_some(Kind::Desk))
            .or_else(|| add_machine.is_some().then_some(Kind::AddMachine))
            .or_else(|| machine_confirm.as_ref().map(MachineConfirm::kind))
            .or_else(|| {
                running
                    .as_ref()
                    .map(|view| if view.is_end() { Kind::RunningEnd } else { Kind::RunningAdopt })
            })
            .or_else(|| root_change.is_some().then_some(Kind::RootChange))
            .or_else(|| delete_view.is_some().then_some(Kind::DeleteSessions))
            .or_else(|| self.pr_kind());
        self.follow_focus(shown, window, cx);
        let ui = self.ui.clone();
        let content = top.map(|top| match top {
            Overlay::Palette => self.palette(&palette, look, &ui, window, cx),
            Overlay::NewSession(sheet_state) => {
                self.new_session(&sheet_state, look, &ui, window, cx)
            }
            Overlay::NewChannel(sheet_state) => {
                self.new_channel(&sheet_state, look, &ui, window, cx)
            }
            Overlay::ChannelAddRepo(sheet_state) => {
                self.channel_add_repo(&sheet_state, look, &ui, window, cx)
            }
            Overlay::SessionAddRepo(sheet_state) => {
                self.session_add_repo(&sheet_state, look, &ui, window, cx)
            }
            Overlay::Shortcuts => self.shortcuts(look, &ui, cx),
        });
        let content = content.or_else(|| {
            closing
                .as_ref()
                .map(|confirm| self.close_dialog(confirm, look, &ui))
        });
        let content = content.or_else(|| {
            archiving
                .as_ref()
                .map(|confirm| self.archive_dialog(confirm, fork_error.clone(), look, &ui))
        });
        let content = content.or_else(|| {
            renaming
                .as_ref()
                .map(|draft| self.rename_dialog(draft, fork_error.clone(), look, &ui, window, cx))
        });
        let content = content.or_else(|| {
            moving
                .clone()
                .map(|draft| self.move_dialog(draft, fork_error.clone(), parent.clone(), look, &ui))
        });
        let content = content.or_else(|| {
            continuing.clone().map(|draft| {
                self.continue_dialog(draft, fork_error.clone(), parent.clone(), look, &ui)
            })
        });
        let content = content.or_else(|| {
            fork.map(|fork| self.fork(fork, forking, fork_error, parent, look, &ui, window, cx))
        });
        let content = match content {
            Some(content) => Some(content),
            None => self.desk(look, &ui, window, cx),
        };
        let content = content.or_else(|| {
            add_machine
                .as_ref()
                .map(|dialog| self.add_machine.draw(dialog, look, window, cx))
        });
        let content = content.or_else(|| {
            machine_confirm.as_ref().map(|confirm| confirm.card(&ui, look).into_any_element())
        });
        let content = content.or_else(|| running.as_ref().map(|view| self.running_dialog(view, look, &ui)));
        let content = content.or_else(|| {
            root_change
                .as_ref()
                .map(|view| self.root_dialog(view, look, &ui, window, cx))
        });
        let content = content.or_else(|| {
            delete_view
                .as_ref()
                .map(|view| self.delete_sessions_dialog(view, look, &ui, window, cx))
        });
        let content = content.or_else(|| self.pr_overlay(look, &ui, window, cx));
        let layer = div().absolute().top_0().left_0().size_full();
        let layer = match content {
            Some(content) => {
                let ui = ui.clone();
                // The close dialog's scrim cancels it; Escape alone doesn't reach it.
                let outside = match shown {
                    Some(Kind::Close) => {
                        AppAction::Session(SessionAction::Close(humata2_session::CloseAction::Cancel))
                    }
                    Some(Kind::DeleteSessions) => {
                        AppAction::MassDelete(humata2_mass_delete::MassDeleteAction::Cancel)
                    }
                    Some(Kind::RunningEnd) => {
                        AppAction::MachinePage(humata2_hosts::MachinePageAction::Running(
                            humata2_hosts::RunningAction::CancelEnd,
                        ))
                    }
                    Some(Kind::RunningAdopt) => {
                        AppAction::MachinePage(humata2_hosts::MachinePageAction::Running(
                            humata2_hosts::RunningAction::CancelAdopt,
                        ))
                    }
                    _ => AppAction::Escape,
                };
                let shown_key = shown.map_or(0usize, |k| k as usize);
                let animate = crate::chrome::native_traffic_lights();
                let scrim = div()
                    .id("scrim")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(if look.content == 0xffffff {
                        rgba(0x0000_0030)
                    } else {
                        rgba(0x0000_0080)
                    })
                    .occlude()
                    .on_click(move |_, _, cx| ui.send(outside.clone(), cx));
                let scrim = if animate {
                    scrim
                        .with_animation(
                            ("overlay-scrim-in", shown_key),
                            Animation::new(Duration::from_millis(ENTRANCE_MS))
                                .with_easing(ease_out_quint),
                            |el, delta| el.opacity(delta),
                        )
                        .into_any_element()
                } else {
                    scrim.into_any_element()
                };
                // Clicks inside the sheet stay there.
                let holder = div()
                    .id("sheet-holder")
                    .max_w(relative(0.94))
                    .occlude()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(content);
                let holder = if animate {
                    holder
                        .with_animation(
                            ("overlay-sheet-in", shown_key),
                            Animation::new(Duration::from_millis(ENTRANCE_MS + 10))
                                .with_easing(ease_out_quint),
                            |el, delta| el.opacity(delta).mt(px(-6. * (1. - delta))),
                        )
                        .into_any_element()
                } else {
                    holder.into_any_element()
                };
                layer
                    .child(scrim)
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full()
                            .flex()
                            .justify_center()
                            .pt(px(match shown {
                                Some(Kind::Palette | Kind::Move | Kind::Continue | Kind::Rename | Kind::RootChange) => 110.,
                                Some(Kind::Archive | Kind::Close | Kind::RunningEnd | Kind::RunningAdopt | Kind::MachineStop) => 120.,
                                Some(Kind::MachineStart) => 180.,
                                Some(Kind::Fork | Kind::SubmitReview | Kind::DeleteSessions) => 24.,
                                Some(Kind::NewSession | Kind::Desk) => 70.,
                                Some(Kind::ChannelAddRepo) => 60.,
                                Some(Kind::AddMachine) => 60.,
                                Some(Kind::NewChannel) => 50.,
                                _ => 96.,
                            }))
                            .child(holder),
                    )
            }
            None => layer,
        };
        layer.child(toasts::stack(toasts, look, &ui))
    }
}
