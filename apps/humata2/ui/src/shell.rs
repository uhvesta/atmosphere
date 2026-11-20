//! The window: a rail of screens, the sidebar, the screen in the middle and
//! the overlays above them. App-wide shortcuts are handled here.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::layout::{FullScreen, PaneId, Reveal};
use humata2_app::{AppAction, Load, PaneAction, Route};
use humata2_clients::model::{ChannelId, SessionId};
use humata2_session::SessionAction;
use humata2_sidebar::SidebarAction;
use humata2_ui_machines::machine_page::select_mode::{MassDeleteAction, Surface};

use crate::bridge::{Part, Ui};
use crate::keys;
use crate::overlays::Overlays;
use crate::report_view::ReportView;
use crate::sidebar::Sidebar;
use crate::theme;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use crate::{chrome, pages, panels};

/// The window's root view.
pub struct Shell {
    ui: Rc<Ui>,
    sidebar: Entity<Sidebar>,
    sidebar_focus: FocusHandle,
    overlays: Entity<Overlays>,
    reports: Entity<ReportView>,
    focus: FocusHandle,
    /// The side whose divider is being dragged.
    dragging: Option<PaneId>,
    file_filter: Entity<InputState>,
    file_filter_session: Option<SessionId>,
    file_filter_value: String,
    _file_filter_subscription: Subscription,
    channel_filter: Entity<InputState>,
    channel_filter_id: Option<ChannelId>,
    channel_filter_value: String,
    _channel_filter_subscription: Subscription,
    todo_inputs: crate::todo::Inputs,
    applied_theme: Option<humata2_settings::Theme>,
    files_name: Entity<InputState>,
    machine_inputs: crate::machine_page::MachinePageInputs,
    providers_inputs: crate::settings::ProvidersInputs,
    _appearance: Subscription,
    _focus_lost: Subscription,
    _activation: Subscription,
    viewport: f32,
    terminals: crate::terminals::Terminals,
    documents: crate::document_tab::Documents,
    focus_target: Option<crate::terminals::FocusTarget>,
    /// The session focused by ↑/↓ while select mode is active (D875).
    select_focus: Option<SessionId>,
}

impl Shell {
    /// The text input of `session`'s terminal tab `tab`, as the platform's
    /// input method reaches it: what a test composes through, since a
    /// headless window has no input method of its own (D484). `None` while
    /// the tab has no screen.
    #[must_use]
    pub fn terminal_input(
        &self,
        session: &SessionId,
        tab: usize,
        bounds: Bounds<Pixels>,
        cx: &App,
    ) -> Option<impl InputHandler + use<>> {
        self.terminals.input_handler(session, tab, bounds, cx)
    }
}

impl Shell {
    /// The window's view on `ui`.
    pub fn new(ui: Rc<Ui>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ui.attach(Part::Main, cx.entity_id());
        let sidebar_focus = cx.focus_handle();
        let sidebar = {
            let ui = ui.clone();
            let focus = sidebar_focus.clone();
            cx.new(|cx| Sidebar::new(ui, focus, cx))
        };
        let overlays = {
            let ui = ui.clone();
            cx.new(|cx| Overlays::new(ui, window, cx))
        };
        let reports = {
            let ui = ui.clone();
            cx.new(|cx| ReportView::new(ui, window, cx))
        };
        let file_filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter files"));
        let file_filter_subscription = cx.subscribe(
            &file_filter,
            |this: &mut Self, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.ui
                        .send(AppAction::Session(SessionAction::FilterFiles(value)), cx);
                }
            },
        );
        let channel_filter =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter sessions"));
        let channel_filter_subscription = cx.subscribe(
            &channel_filter,
            |this: &mut Self, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.ui.send(
                        AppAction::Channel(humata2_app::channel::Action::Filter(
                            input.read(cx).value().to_string(),
                        )),
                        cx,
                    );
                }
            },
        );
        let todo_inputs = {
            let ui = ui.clone();
            let send: crate::todo::Send =
                Rc::new(move |action, cx| ui.send(AppAction::Todo(action), cx));
            crate::todo::Inputs::new(&send, window, cx)
        };
        let files_name = cx.new(|cx| InputState::new(window, cx).placeholder("Name"));
        let machine_inputs = crate::machine_page::MachinePageInputs::new(
            &ui,
            AppAction::MachinePage,
            window,
            cx,
        );
        let providers_inputs = crate::settings::ProvidersInputs::new(&ui, window, cx);
        let appearance_ui = ui.clone();
        let appearance = window.observe_window_appearance(move |window, cx| {
            if appearance_ui
                .read(|s| s.settings.preferences.theme == humata2_settings::Theme::System)
            {
                gpui_kit::component::theme::Theme::sync_system_appearance(Some(window), cx);
            }
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // When the focused element leaves the tree (a sidebar folded away, a
        // sheet or panel closed), GPUI dispatches keys from the window's root,
        // which sits above the shell: its actions (⌘L, ⌘0, Escape) would stop
        // answering. Hand the keyboard to the nearest focusable ancestor that
        // is still drawn, or to the shell itself.
        let focus_lost = cx.on_focus_lost(window, |this, window, cx| {
            let target = window
                .focus_lost_restore_target(cx)
                .unwrap_or_else(|| this.focus.clone());
            window.focus(&target, cx);
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            this.send(AppAction::WindowActive(window.is_window_active()), cx);
        });
        let terminals = crate::terminals::Terminals::new(&ui, cx);
        let documents = crate::document_tab::Documents::new(&ui, window, cx);
        Self {
            terminals,
            documents,
            focus_target: None,
            select_focus: None,
            ui,
            sidebar,
            sidebar_focus,
            overlays,
            reports,
            focus,
            dragging: None,
            file_filter,
            file_filter_session: None,
            file_filter_value: String::new(),
            _file_filter_subscription: file_filter_subscription,
            channel_filter,
            channel_filter_id: None,
            channel_filter_value: String::new(),
            _channel_filter_subscription: channel_filter_subscription,
            todo_inputs,
            applied_theme: None,
            files_name,
            machine_inputs,
            providers_inputs,
            _appearance: appearance,
            _focus_lost: focus_lost,
            _activation: activation,
            viewport: 0.,
        }
    }

    fn send(&self, action: AppAction, cx: &mut App) {
        self.ui.send(action, cx);
    }

    fn input_focused(&self, window: &Window, cx: &App) -> bool {
        self.file_filter.read(cx).focus_handle(cx).is_focused(window)
            || self.channel_filter.read(cx).focus_handle(cx).is_focused(window)
            || self.files_name.read(cx).focus_handle(cx).is_focused(window)
    }

    /// MASSDELETE (D875): ⌘A selects all visible on the current surface, ↑/↓
    /// moves focus, Space ticks/unticks the focused session, Return or ⌘⌫
    /// opens the delete confirmation sheet.
    fn handle_select_key(
        &mut self,
        event: &KeyDownEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.input_focused(window, cx) {
            return false;
        }
        let Some(key) = match_select_key(&event.keystroke) else {
            return false;
        };
        let Some((visible, fallback_focus, has_selection)) = self.ui.read(|state| {
            let surface = state.mass_delete.surface.clone()?;
            if state.mass_delete.sheet.is_some()
                || state.dialog_holds_keys()
                || state.report.form.is_some()
            {
                return None;
            }
            let visible = visible_sessions(state, &surface);
            let sidebar_cursor = match (&surface, &state.sidebar.cursor) {
                (Surface::Sidebar, Some(humata2_sidebar::RowKey::Session(id))) => Some(id.clone()),
                _ => None,
            };
            let fallback = sidebar_cursor.or_else(|| state.mass_delete.anchor.clone());
            Some((visible, fallback, state.mass_delete.selected_count() > 0))
        }) else {
            return false;
        };
        match key {
            SelectKey::SelectAll => {
                self.send(
                    AppAction::MassDelete(MassDeleteAction::SelectAllVisible(visible)),
                    cx,
                );
                true
            }
            SelectKey::MoveUp | SelectKey::MoveDown => {
                let current = self
                    .select_focus
                    .as_ref()
                    .filter(|id| visible.contains(id))
                    .or_else(|| fallback_focus.as_ref().filter(|id| visible.contains(id)));
                if let Some(next) = step_focus(&visible, current, key == SelectKey::MoveDown) {
                    self.select_focus = Some(next);
                    cx.notify();
                }
                true
            }
            SelectKey::ToggleFocused => {
                let target = self
                    .select_focus
                    .clone()
                    .filter(|id| visible.contains(id))
                    .or_else(|| fallback_focus.filter(|id| visible.contains(id)))
                    .or_else(|| visible.first().cloned());
                if let Some(id) = target {
                    self.select_focus = Some(id.clone());
                    self.send(AppAction::MassDelete(MassDeleteAction::Toggle(id)), cx);
                }
                true
            }
            SelectKey::OpenSheet => {
                if has_selection {
                    self.send(AppAction::MassDelete(MassDeleteAction::OpenPicked), cx);
                }
                true
            }
        }
    }
}

fn top_bar(
    ui: &Rc<Ui>,
    title: String,
    can_go_back: bool,
    sidebar_hidden: bool,
    look: Look,
) -> impl IntoElement {
    row()
        .id("top-bar")
        .h(px(theme::TOP_BAR))
        .flex_shrink_0()
        .px_3()
        .gap_2()
        .border_b_1()
        .border_color(rgb(look.line))
        .when(sidebar_hidden, |bar| {
            bar.child(
                widgets::ghost_in(look, ui, "show-sidebar", "", AppAction::ToggleNavigationOverlay)
                    .icon(IconName::PanelLeft)
                    .accessibility_label("Show sidebar")
                    .tooltip("Show sidebar  ⌘0"),
            )
        })
        // A quiet ghost Back, as the session header draws it (DS-58).
        .child({
            let back = ui.clone();
            Button::new("back")
                .ghost()
                .small()
                .w(px(24.))
                .h(px(24.))
                .p_0()
                .flex_shrink_0()
                .icon(chrome::icon(IconName::ChevronLeft, look.secondary, 14.))
                .accessibility_label("Back")
                .tooltip("Back  ⌘[")
                .disabled(!can_go_back)
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    back.send(AppAction::Back, cx);
                })
        })
        .child(
            div()
                .id("screen-title")
                .role(Role::Heading)
                .aria_label(title.clone())
                .flex_1()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.text))
                .child(title)
                .test_support(),
        )
        .child(
            Button::new("search")
                .icon(IconName::Search)
                .label("Search")
                .ghost()
                .small()
                .accessibility_label("Search")
                .tooltip("Command palette  ⌘K")
                .on_click({
                    let ui = ui.clone();
                    move |_, _, cx| ui.send(AppAction::OpenPalette, cx)
                }),
        )
        .when(
            ui.read(|state| matches!(state.route, Route::Session(_))),
            |bar| {
                bar.child(
                    widgets::ghost_in(look,
                        ui,
                        "toggle-side-panel",
                        "",
                        AppAction::Pane(PaneAction::Toggle(PaneId::Panel)),
                    )
                    .icon(IconName::PanelRight)
                    .accessibility_label("Toggle side panel")
                    .tooltip("Toggle side panel  ⌥⌘0"),
                )
            },
        )
}

impl Shell {
    /// The strip between `pane` and the main area: drag it to resize the
    /// pane, past its narrowest to hide it; double-click to reset it.
    fn divider(&self, pane: PaneId, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::of(cx);
        let id = match pane {
            PaneId::Sidebar => "sidebar-divider",
            PaneId::Panel => "panel-divider",
        };
        div()
            .id(id)
            .role(Role::Splitter)
            .aria_label(format!("Resize the {}", pane.name()))
            .w(px(theme::DIVIDER))
            .ml(px(-(theme::DIVIDER - 1.) / 2.))
            .mr(px(-(theme::DIVIDER - 1.) / 2.))
            .h_full()
            .flex_shrink_0()
            .cursor_col_resize()
            .group(id)
            .flex()
            .items_center()
            .justify_center()
            // The grip shows under the pointer, as frames 37 and 38 draw it.
            .child(
                div()
                    .w(px(5.))
                    .h(px(36.))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(rgb(look.secondary))
                    .opacity(0.)
                    .group_hover(id, |grip| grip.opacity(1.)),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        this.dragging = None;
                        this.send(AppAction::Pane(PaneAction::Reset(pane)), cx);
                    } else {
                        this.dragging = Some(pane);
                    }
                }),
            )
            .test_support()
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.select_focus.is_some()
            && self.ui.read(|state| state.mass_delete.surface.is_none())
        {
            self.select_focus = None;
        }
        let theme = self.ui.read(|s| s.settings.preferences.theme);
        if self.applied_theme != Some(theme) {
            self.applied_theme = Some(theme);
            use gpui_kit::component::theme::{Theme, ThemeMode};
            match theme {
                humata2_settings::Theme::System => Theme::sync_system_appearance(Some(window), cx),
                humata2_settings::Theme::Light => Theme::change(ThemeMode::Light, Some(window), cx),
                humata2_settings::Theme::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
            }
        }
        let look = Look::of(cx);
        let lost_focus = self
            .ui
            .read(|state| self.terminals.sync(&self.ui, state, look, window, cx));
        if lost_focus {
            // The Inbox's terminal went with its item: the list takes the
            // keys back (D781).
            window.focus(&self.focus, cx);
        }
        // A Stop and Resume makes the tab a new view, which nothing has
        // focused: it takes the keys again, as a session just opened does,
        // when the old view had them or nothing else has them now.
        if let Some(had_focus) = self.terminals.take_replaced()
            && (had_focus || window.focused(cx).is_none() || self.focus.is_focused(window))
        {
            self.focus_target = None;
        }
        self.documents.sync(&self.ui, window, cx);
        let target = self.ui.read(|state| {
            (!state.dialog_holds_keys()).then(|| crate::terminals::focus_target(state))
        });
        if let Some(target) = target
            && self.focus_target.as_ref() != Some(&target)
        {
            let wants_terminal = matches!(&target, (Route::Session(_), Some(humata2_session::Tab::Terminal(_)), side) if *side != Some(FullScreen::Panel));
            let focused = match &target {
                (Route::Session(id), Some(humata2_session::Tab::Terminal(tab)), side)
                    if *side != Some(FullScreen::Panel) =>
                {
                    self.terminals.focus(id, *tab, window, cx)
                }
                _ => false,
            };
            if !focused {
                window.focus(&self.focus, cx);
            }
            // The open reply can arrive before its session snapshot creates the pane.
            self.focus_target = if wants_terminal && !focused {
                None
            } else {
                Some(target)
            };
        }
        let viewport = f32::from(window.viewport_size().width);
        if self.viewport != viewport {
            self.viewport = viewport;
            if self.ui.read(|state| state.navigation_overlay) {
                self.send(AppAction::CloseNavigationOverlay, cx);
            }
        }
        let (selected_panel, columns, full_screen, in_session, navigation_overlay, strip_width) =
            self.ui.read(|state| {
                let in_session = matches!(state.route, Route::Session(_));
                (
                    state.panel(),
                    state
                        .layout
                        .columns(viewport, in_session, state.panel().is_some()),
                    state.full_screen(),
                    in_session,
                    state.navigation_overlay,
                    if state.layout.panel.collapsed {
                        14.
                    } else {
                        64.
                    },
                )
            });
        let full = full_screen == Some(FullScreen::Panel);
        let terminal_full = full_screen == Some(FullScreen::Terminal);
        let panel = selected_panel.filter(|_| full || !columns.panel_hidden);
        let panel_width = columns.panel;
        let collapsed = columns.sidebar_hidden;
        // How ⌘0 and ⌘L bring a hidden sidebar back (NV-6).
        let reveal = self.ui.read(|state| {
            state
                .layout
                .reveal_sidebar(viewport, in_session, state.panel().is_some())
        });
        let main_width = if terminal_full {
            viewport
        } else {
            viewport
                - columns.sidebar
                - if collapsed { 14. } else { 1. }
                - panel_width
                - if panel.is_some() { 1. } else { 0. }
                - if in_session { strip_width } else { 0. }
        };
        let (load, can_go_back, title, restore_error) = self.ui.read(|state| {
            (
                state.load.clone(),
                !state.trail.is_empty(),
                pages::title(state),
                state.restore_error.clone(),
            )
        });
        let (session, filter) = self.ui.read(|state| {
            (
                state.session.open.clone(),
                state
                    .session
                    .view()
                    .map(|view| view.file_filter.clone())
                    .unwrap_or_default(),
            )
        });
        // Input events are delivered after the render that follows editing. Sync
        // only a new stored value/session, so that render cannot erase the edit.
        if self.file_filter_session != session || self.file_filter_value != filter {
            self.file_filter_session = session;
            self.file_filter_value = filter.clone();
            if self.file_filter.read(cx).value().as_ref() != filter {
                self.file_filter
                    .update(cx, |input, cx| input.set_value(filter, window, cx));
            }
        }
        let (channel, query) = self.ui.read(|s| match &s.route {
            Route::Channel(id) => (
                Some(id.clone()),
                s.channel_pages
                    .get(id)
                    .map_or_else(String::new, |v| v.query.clone()),
            ),
            _ => (None, String::new()),
        });
        if self.channel_filter_id != channel || self.channel_filter_value != query {
            self.channel_filter_id = channel;
            self.channel_filter_value = query.clone();
            if self.channel_filter.read(cx).value().as_ref() != query {
                self.channel_filter
                    .update(cx, |input, cx| input.set_value(query, window, cx));
            }
        }
        self.ui
            .read(|state| self.todo_inputs.sync(&state.todo, window, cx));
        let ui = self.ui.clone();
        let body = match load {
            Load::Loading => col()
                .flex_1()
                .items_center()
                .justify_center()
                .child(widgets::dim_in(look, "Connecting to this Mac…"))
                .into_any_element(),
            Load::Failed(message) => col()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_3()
                .child(
                    div()
                        .id("load-error")
                        .role(Role::Alert)
                        .aria_label(message.clone())
                        .child(message)
                        .test_support(),
                )
                .child(widgets::primary_in(look,
                    &ui,
                    "retry",
                    "Try again",
                    AppAction::Retry,
                ))
                .into_any_element(),
            Load::Ready => self.ui.read(|state| {
                pages::page(
                    &ui,
                    state,
                    self.channel_filter.clone(),
                    &self.todo_inputs,
                    self.files_name.clone(),
                    &self.terminals,
                    &self.documents,
                    &self.machine_inputs,
                    &self.providers_inputs,
                    window,
                    cx,
                )
            }),
        };
        let main = col()
            .id("main")
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(look.content))
            .child(if in_session {
                self.ui
                    .read(|state| pages::session_header(&ui, state, collapsed, main_width, look))
                    .into_any_element()
            } else {
                self.ui.read(|state| {
                    if matches!(state.route, Route::Settings) {
                        crate::settings::toolbar(look).into_any_element()
                    } else if matches!(state.route, Route::Machine(_)) {
                        div().into_any_element()
                    } else if matches!(state.route, Route::Hosts) {
                        crate::hosts::toolbar(&ui, look).into_any_element()
                    } else if matches!(state.route, Route::Desk) {
                        crate::desk::toolbar(&ui, state, collapsed, look).into_any_element()
                    } else if let Route::Channel(id) = &state.route
                        && let Some(channel) = state.workspace.channel(id)
                    {
                        crate::channel::header(&ui, state, channel, look).into_any_element()
                    } else if matches!(state.route, Route::Home | Route::Inbox) {
                        crate::lists::toolbar(&ui, state, title, collapsed, look).into_any_element()
                    } else {
                        top_bar(&ui, title, can_go_back, collapsed, look).into_any_element()
                    }
                })
            })
            .children(restore_error.map(|error| {
                div().p_3().child(widgets::error_banner_in(
                    look,
                    &ui,
                    "restore-error",
                    error,
                    AppAction::DismissRestoreError,
                ))
            }))
            .child(div().flex_1().min_h_0().flex().flex_col().child(body));
        let inbox_keys = self
            .ui
            .read(|state| state.route == Route::Inbox && !state.dialog_holds_keys());
        div()
            .id("shell")
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .bg(rgb(look.content))
            .text_color(rgb(look.text))
            .font_family(".SystemUIFont")
            .text_size(px(13.))
            .track_focus(&self.focus)
            // Escape in a terminal reaches its program (keys.rs unbinds
            // Dismiss there), unless something over the page would close.
            // While select mode is active, its keys (⌘A, Space, ↑/↓, Return,
            // ⌘⌫) are captured here before children act on them.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let bare = !event.keystroke.modifiers.modified();
                let escape = event.keystroke.key == "escape" && bare;
                if escape && this.ui.read(escape_closes_something) {
                    cx.stop_propagation();
                    this.send(AppAction::Escape, cx);
                    return;
                }
                if this.handle_select_key(event, window, cx) {
                    cx.stop_propagation();
                }
            }))
            // Tab from the Inbox's list moves into its open item's
            // terminal; nothing focuses it by itself, and once it has focus
            // Tab reaches its program, whose context unbinds it (D781).
            .when(inbox_keys, |shell| shell.key_context(keys::INBOX))
            .on_action(cx.listener(|this, _: &keys::FocusInboxTerminal, window, cx| {
                if !this.terminals.focus_inbox(window, cx) {
                    window.focus_next(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &keys::OpenPalette, _, cx| {
                this.send(AppAction::OpenPalette, cx)
            }))
            .on_action(cx.listener(|this, _: &keys::NewSession, _, cx| {
                this.send(AppAction::StartNewSession, cx)
            }))
            .on_action(cx.listener(|this, _: &keys::NewChannel, _, cx| {
                this.send(AppAction::StartNewChannel, cx)
            }))
            .on_action(cx.listener(|this, _: &keys::Back, _, cx| this.send(AppAction::Back, cx)))
            .on_action(
                cx.listener(|this, _: &keys::Dismiss, _, cx| this.send(AppAction::Escape, cx)),
            )
            .on_action(cx.listener(|this, _: &keys::ShowShortcuts, _, cx| {
                this.send(AppAction::ShowShortcuts, cx)
            }))
            .on_action(cx.listener(|this, _: &keys::ExpandAll, _, cx| {
                this.send(AppAction::Sidebar(SidebarAction::ExpandAll), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::CollapseAll, _, cx| {
                this.send(AppAction::Sidebar(SidebarAction::CollapseAll), cx);
            }))
            .on_action(cx.listener(move |this, _: &keys::ToggleSidebar, _, cx| {
                // Folded with room to spare, the sidebar docks again; only a
                // window too narrow for it shows it over the page.
                let action = if reveal == Some(Reveal::Overlay) {
                    AppAction::ToggleNavigationOverlay
                } else {
                    AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar))
                };
                this.send(action, cx);
            }))
            .on_action(cx.listener(|this, _: &keys::GoHome, _, cx| {
                this.send(AppAction::Switch(Route::Home), cx)
            }))
            .on_action(cx.listener(|this, _: &keys::GoInbox, _, cx| {
                this.send(AppAction::Switch(Route::Inbox), cx)
            }))
            .on_action(cx.listener(|this, _: &keys::GoBoard, _, cx| {
                this.send(AppAction::Switch(Route::Todo), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::OpenSettings, _, cx| {
                this.send(AppAction::Switch(Route::Settings), cx);
            }))
            .on_action(cx.listener(move |this, _: &keys::FocusSidebar, window, cx| {
                // A hidden sidebar is shown first, so focus lands on rows
                // people can see.
                match reveal {
                    Some(Reveal::Dock) => {
                        this.send(AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)), cx);
                    }
                    Some(Reveal::Overlay) if !navigation_overlay => {
                        this.send(AppAction::ToggleNavigationOverlay, cx);
                    }
                    _ => {}
                }
                window.focus(&this.sidebar_focus, cx);
            }))
            .on_action(cx.listener(|this, _: &keys::GoHosts, _, cx| {
                this.send(AppAction::Switch(Route::Hosts), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::GoFiles, _, cx| {
                this.send(AppAction::Switch(Route::Files), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::TerminalFullScreen, _, cx| {
                if this.ui.read(|state| matches!(state.route, Route::Session(_))) {
                    this.send(AppAction::ToggleFullScreen(FullScreen::Terminal), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &keys::PanelFullScreen, _, cx| {
                if this.ui.read(|state| matches!(state.route, Route::Session(_))) {
                    let run = humata2_palette::PaletteAction::Delegate(
                        humata2_palette::Delegate::Run(humata2_palette::Command::PanelFullScreen),
                    );
                    this.send(AppAction::Palette(run), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &keys::ForkSession, _, cx| {
                // The palette's Fork, which a Desk session doesn't have.
                if this.ui.read(|state| matches!(state.route, Route::Session(_))) {
                    let run = humata2_palette::PaletteAction::Delegate(
                        humata2_palette::Delegate::Run(humata2_palette::Command::Fork),
                    );
                    this.send(AppAction::Palette(run), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &keys::NewTerminalTab, _, cx| {
                if this.ui.read(|state| matches!(state.route, Route::Session(_))) {
                    this.send(AppAction::Session(SessionAction::NewTerminalTab), cx);
                }
            }))
            .on_action(cx.listener(|this, _: &keys::TogglePanel, _, cx| {
                this.send(AppAction::Pane(PaneAction::Toggle(PaneId::Panel)), cx);
            }))
            // D804: step a stack's layers; the bindings only fire in the
            // Diff panel while a stack shows.
            .on_action(cx.listener(|this, _: &keys::NextLayer, _, cx| {
                this.send(AppAction::Review(humata2_review::ReviewAction::NextLayer), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::PrevLayer, _, cx| {
                this.send(AppAction::Review(humata2_review::ReviewAction::PrevLayer), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::NextFile, _, cx| {
                // Runs only while the open session's panel shows the Diff.
                let run = humata2_palette::PaletteAction::Delegate(
                    humata2_palette::Delegate::Run(humata2_palette::Command::NextFile),
                );
                this.send(AppAction::Palette(run), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::PrevFile, _, cx| {
                // Runs only while the open session's panel shows the Diff.
                let run = humata2_palette::PaletteAction::Delegate(
                    humata2_palette::Delegate::Run(humata2_palette::Command::PrevFile),
                );
                this.send(AppAction::Palette(run), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::NextChange, _, cx| {
                // Runs only while the open session's panel shows the Diff.
                let run = humata2_palette::PaletteAction::Delegate(
                    humata2_palette::Delegate::Run(humata2_palette::Command::NextChange),
                );
                this.send(AppAction::Palette(run), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::PrevChange, _, cx| {
                // Runs only while the open session's panel shows the Diff.
                let run = humata2_palette::PaletteAction::Delegate(
                    humata2_palette::Delegate::Run(humata2_palette::Command::PrevChange),
                );
                this.send(AppAction::Palette(run), cx);
            }))
            // DS-40: ⌥Z, bound only while the diff has focus, flips the same
            // preference as Settings › Long lines.
            .on_action(cx.listener(|this, _: &keys::ToggleDiffWrap, _, cx| {
                this.send(
                    AppAction::Settings(humata2_settings::SettingsAction::ToggleWrap),
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &keys::MoveSession, _, cx| {
                // ⌥⌘M asks for a machine only when a session is open.
                if this.ui.read(|state| matches!(state.route, Route::Session(_))) {
                    this.send(AppAction::OpenPalette, cx);
                    let begin = humata2_palette::PaletteAction::Begin(
                        humata2_palette::Command::MoveSession,
                    );
                    this.send(AppAction::Palette(begin), cx);
                } else {
                    // With nothing to move, the app says so (a toast).
                    let run = humata2_palette::PaletteAction::Delegate(
                        humata2_palette::Delegate::Run(humata2_palette::Command::MoveSession),
                    );
                    this.send(AppAction::Palette(run), cx);
                }
            }))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let Some(pane) = this.dragging else { return };
                if event.pressed_button != Some(MouseButton::Left) {
                    this.dragging = None;
                    return;
                }
                let x = f32::from(event.position.x);
                let width = match pane {
                    PaneId::Sidebar => x,
                    PaneId::Panel => viewport - if in_session { 64. } else { 0. } - x,
                };
                this.send(AppAction::Pane(PaneAction::Drag(pane, width)), cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.dragging = None),
            )
            .when(!full, |shell| {
                shell
                    .when(!terminal_full && !collapsed, |shell| {
                        shell
                            .child(self.sidebar.clone())
                            .child(self.divider(PaneId::Sidebar, cx))
                    })
                    .when(!terminal_full && collapsed, |shell| {
                        shell.child(
                            div()
                                .id("sidebar-strip")
                                .role(Role::Button)
                                .aria_label("Show sidebar")
                                .w(px(14.))
                                .h_full()
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(rgb(look.sidebar))
                                .border_r_1()
                                .border_color(rgb(look.line))
                                .child(chrome::icon(IconName::ChevronRight, look.secondary, 12.))
                                .on_click({
                                    let ui = ui.clone();
                                    move |_, _, cx| ui.send(AppAction::ToggleNavigationOverlay, cx)
                                })
                                .test_support(),
                        )
                    })
                    .child(main)
                    .when_some(panel.filter(|_| !terminal_full), |shell, panel| {
                        shell
                            .child(self.divider(PaneId::Panel, cx))
                            .child(self.ui.read(|state| {
                                panels::panel(
                                    &ui,
                                    state,
                                    panel,
                                    panel_width,
                                    false,
                                    self.file_filter.clone(),
                                    window,
                                    &self.documents,
                                    &self.todo_inputs,
                                    cx,
                                )
                            }))
                    })
                    .when(in_session && !terminal_full, |shell| {
                        if strip_width == 14. {
                            shell.child(
                                div()
                                    .id("panel-restore-strip")
                                    .role(Role::Button)
                                    .aria_label("Show side panel")
                                    .w(px(14.))
                                    .h_full()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .bg(rgb(look.sidebar))
                                    .border_l_1()
                                    .border_color(rgb(look.line))
                                    .child(chrome::icon(IconName::ChevronLeft, look.secondary, 12.))
                                    .on_click({
                                        let ui = ui.clone();
                                        move |_, _, cx| {
                                            ui.send(
                                                AppAction::Pane(PaneAction::Toggle(PaneId::Panel)),
                                                cx,
                                            )
                                        }
                                    })
                                    .test_support(),
                            )
                        } else {
                            shell.child(panels::strip(&ui, selected_panel, cx))
                        }
                    })
            })
            .when(full, |shell| {
                shell.children(panel.map(|panel| {
                    self.ui.read(|state| {
                        panels::panel(
                            &ui,
                            state,
                            panel,
                            viewport,
                            true,
                            self.file_filter.clone(),
                            window,
                            &self.documents,
                            &self.todo_inputs,
                            cx,
                        )
                    })
                }))
            })
            .when(
                navigation_overlay && collapsed && !terminal_full && !full,
                |shell| {
                    shell.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .child(
                                div()
                                    .id("navigation-dismiss")
                                    .absolute()
                                    .inset_0()
                                    .bg(rgba(0x00000040))
                                    .on_click({
                                        let ui = ui.clone();
                                        move |_, _, cx| {
                                            ui.send(AppAction::CloseNavigationOverlay, cx)
                                        }
                                    }),
                            )
                            .child(self.sidebar.clone()),
                    )
                },
            )
            .children(
                self.ui
                    .read(|state| crate::channel::delete_modal(&ui, state, look)),
            )
            .child(self.overlays.clone())
            .child(self.reports.clone())
            .map(|root| crate::report_view::actions(root, &self.ui))
    }
}

/// Whether Escape has something over the page to close, so it is the app's
/// even while a terminal holds the keyboard.
fn escape_closes_something(state: &humata2_app::AppState) -> bool {
    state.dialog_holds_keys()
        || state.mass_delete.selecting()
        || state.report.form.is_some()
        || state.navigation_overlay
        || state.settings.font_menu
        || state.settings.desk_rows_menu
        || state.settings.repos.adding_repo
        || state.channel_menu
        || state.session.terminal_menu.is_some()
        || state.session.menu
        || state.hosts.start_confirm.is_some()
        || state.machine_page.confirm.is_some()
        || state.machine_page.menu_open
}

/// Select-mode keyboard actions (MASSDELETE, D875).
#[derive(Debug, PartialEq, Eq)]
enum SelectKey {
    SelectAll,
    MoveUp,
    MoveDown,
    ToggleFocused,
    OpenSheet,
}

fn match_select_key(keystroke: &Keystroke) -> Option<SelectKey> {
    let m = &keystroke.modifiers;
    // macOS sets the function flag on arrow keys, so ignore it here.
    let unmodified = !m.platform && !m.control && !m.alt && !m.shift;
    let cmd_only = m.platform && !m.control && !m.alt && !m.shift;
    match keystroke.key.as_str() {
        "a" if cmd_only => Some(SelectKey::SelectAll),
        "backspace" if cmd_only => Some(SelectKey::OpenSheet),
        "enter" if unmodified => Some(SelectKey::OpenSheet),
        "space" if unmodified => Some(SelectKey::ToggleFocused),
        "up" if unmodified => Some(SelectKey::MoveUp),
        "down" if unmodified => Some(SelectKey::MoveDown),
        _ => None,
    }
}

fn step_focus(
    visible: &[SessionId],
    current: Option<&SessionId>,
    down: bool,
) -> Option<SessionId> {
    if visible.is_empty() {
        return None;
    }
    let next = match current.and_then(|id| visible.iter().position(|x| x == id)) {
        Some(index) if down => (index + 1).min(visible.len() - 1),
        Some(index) => index.saturating_sub(1),
        None if down => 0,
        None => visible.len() - 1,
    };
    visible.get(next).cloned()
}

fn visible_sessions(state: &humata2_app::AppState, surface: &Surface) -> Vec<SessionId> {
    match surface {
        Surface::Sidebar => {
            let channel_start = state
                .sidebar
                .rows
                .iter()
                .position(|r| matches!(r.kind, humata2_sidebar::RowKind::Section("Channels")))
                .unwrap_or(0);
            state
                .sidebar
                .rows
                .iter()
                .skip(channel_start + 1)
                .filter_map(|r| match &r.key {
                    Some(humata2_sidebar::RowKey::Session(id)) => Some(id.clone()),
                    _ => None,
                })
                .collect()
        }
        Surface::Home => {
            let needs = humata2_app::needs_you(&state.workspace);
            let listed: Vec<&SessionId> = needs
                .rows
                .iter()
                .filter_map(|row| match row {
                    humata2_app::NeedsRow::Session { session, .. } => Some(&session.id),
                    humata2_app::NeedsRow::Request(_) => None,
                })
                .collect();
            let all = state
                .workspace
                .sessions
                .iter()
                .filter(|s| !s.archived && !listed.contains(&&s.id));
            let working = all
                .clone()
                .filter(|s| s.status == humata2_clients::model::Status::Working);
            let recent = all.filter(|s| {
                matches!(
                    s.status,
                    humata2_clients::model::Status::Idle
                        | humata2_clients::model::Status::ProbablyIdle
                ) && !s.activity.is_empty()
            });
            let (stopped, ended): (Vec<_>, Vec<_>) = needs
                .rows
                .iter()
                .filter(|row| {
                    !matches!(
                        row,
                        humata2_app::NeedsRow::Request(r) if r.operation.starts_with("board.")
                    )
                })
                .partition(|row| match row {
                    humata2_app::NeedsRow::Session { session, request } => {
                        request.is_some()
                            || session.status == humata2_clients::model::Status::NeedsYou
                    }
                    humata2_app::NeedsRow::Request(_) => true,
                });
            stopped
                .into_iter()
                .chain(ended)
                .filter_map(|row| match row {
                    humata2_app::NeedsRow::Session { session, .. } => Some(session.id.clone()),
                    humata2_app::NeedsRow::Request(_) => None,
                })
                .chain(working.map(|s| s.id.clone()))
                .chain(recent.map(|s| s.id.clone()))
                .collect()
        }
        Surface::Machine(machine) => {
            if state.machine_page.machine.as_ref() == Some(machine)
                && let Some(detail) = &state.machine_page.detail
            {
                detail
                    .sessions
                    .iter()
                    .filter(|id| state.workspace.session(id).is_some())
                    .cloned()
                    .collect()
            } else {
                state
                    .workspace
                    .sessions
                    .iter()
                    .filter(|s| !s.archived && &s.machine == machine)
                    .map(|s| s.id.clone())
                    .collect()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(name: &str, platform: bool) -> Keystroke {
        Keystroke {
            modifiers: Modifiers {
                platform,
                ..Modifiers::default()
            },
            key: name.to_owned(),
            key_char: None,
        }
    }

    #[::core::prelude::v1::test]
    fn select_mode_key_matching() {
        assert_eq!(match_select_key(&key("a", true)), Some(SelectKey::SelectAll));
        assert_eq!(match_select_key(&key("a", false)), None);
        assert_eq!(
            match_select_key(&key("backspace", true)),
            Some(SelectKey::OpenSheet)
        );
        assert_eq!(match_select_key(&key("backspace", false)), None);
        assert_eq!(
            match_select_key(&key("enter", false)),
            Some(SelectKey::OpenSheet)
        );
        assert_eq!(
            match_select_key(&key("space", false)),
            Some(SelectKey::ToggleFocused)
        );
        assert_eq!(match_select_key(&key("up", false)), Some(SelectKey::MoveUp));
        assert_eq!(
            match_select_key(&key("down", false)),
            Some(SelectKey::MoveDown)
        );
        // macOS sets the function flag on arrow keys; arrows still match.
        let mut arrow = key("down", false);
        arrow.modifiers.function = true;
        assert_eq!(match_select_key(&arrow), Some(SelectKey::MoveDown));
    }

    #[::core::prelude::v1::test]
    fn step_focus_clamps_to_visible_bounds() {
        let s1 = SessionId("s1".to_owned());
        let s2 = SessionId("s2".to_owned());
        let s3 = SessionId("s3".to_owned());
        let visible = vec![s1.clone(), s2.clone(), s3.clone()];

        assert_eq!(step_focus(&visible, None, true), Some(s1.clone()));
        assert_eq!(step_focus(&visible, None, false), Some(s3.clone()));
        assert_eq!(step_focus(&visible, Some(&s1), true), Some(s2.clone()));
        assert_eq!(step_focus(&visible, Some(&s3), true), Some(s3.clone()));
        assert_eq!(step_focus(&visible, Some(&s1), false), Some(s1.clone()));
        assert_eq!(step_focus(&visible, Some(&s2), false), Some(s1));
        assert_eq!(step_focus(&[], None, true), None);
    }
}
