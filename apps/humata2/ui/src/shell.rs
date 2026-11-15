//! The window: a rail of screens, the sidebar, the screen in the middle and
//! the overlays above them. App-wide shortcuts are handled here.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, Load, PaneAction, Route};
use humata2_clients::model::{Scope, SessionId};
use humata2_session::SessionAction;
use humata2_sidebar::SidebarAction;

use crate::bridge::{Part, Ui};
use crate::keys;
use crate::overlays::Overlays;
use crate::sidebar::Sidebar;
use crate::theme;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use crate::{pages, panels};

/// The window's root view.
pub struct Shell {
    ui: Rc<Ui>,
    sidebar: Entity<Sidebar>,
    sidebar_focus: FocusHandle,
    overlays: Entity<Overlays>,
    focus: FocusHandle,
    /// The side whose divider is being dragged.
    dragging: Option<PaneId>,
    file_filter: Entity<InputState>,
    file_filter_session: Option<SessionId>,
    file_filter_value: String,
    _file_filter_subscription: Subscription,
    _appearance: Subscription,
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
        let appearance = window.observe_window_appearance(|window, cx| {
            gpui_kit::component::theme::Theme::sync_system_appearance(Some(window), cx)
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            ui,
            sidebar,
            sidebar_focus,
            overlays,
            focus,
            dragging: None,
            file_filter,
            file_filter_session: None,
            file_filter_value: String::new(),
            _file_filter_subscription: file_filter_subscription,
            _appearance: appearance,
        }
    }

    fn send(&self, action: AppAction, cx: &mut App) {
        self.ui.send(action, cx);
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
                widgets::ghost(
                    ui,
                    "show-sidebar",
                    "",
                    AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)),
                )
                .icon(IconName::PanelLeft)
                .accessibility_label("Show sidebar")
                .tooltip("Show sidebar  ⌘0"),
            )
        })
        .child(
            widgets::ghost(ui, "back", "", AppAction::Back)
                .icon(IconName::ChevronLeft)
                .accessibility_label("Back")
                .tooltip("Back  ⌘[")
                .disabled(!can_go_back),
        )
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
                    widgets::ghost(
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
            .hover(move |divider| divider.bg(rgb(look.line)))
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
        let look = Look::of(cx);
        let viewport = f32::from(window.viewport_size().width);
        let (panel, panel_width, full, in_session) = self.ui.read(|state| {
            let panel = state.panel().filter(|_| !state.layout.panel.collapsed);
            let in_session = matches!(state.route, Route::Session(_));
            let strip = if in_session { 64. } else { 0. };
            let (_, width) = state
                .layout
                .widths(viewport - theme::RAIL_WIDTH - strip, panel.is_some());
            let full = panel.is_some() && state.session.view().is_some_and(|view| view.widened);
            (panel, width, full, in_session)
        });
        let (load, can_go_back, collapsed, title, restore_error) = self.ui.read(|state| {
            (
                state.load.clone(),
                !state.trail.is_empty(),
                state.layout.sidebar.collapsed,
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
        let ui = self.ui.clone();
        let body = match load {
            Load::Loading => col()
                .flex_1()
                .items_center()
                .justify_center()
                .child(widgets::dim("Connecting to this Mac…"))
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
                .child(widgets::primary(
                    &ui,
                    "retry",
                    "Try again",
                    AppAction::Retry,
                ))
                .into_any_element(),
            Load::Ready => self.ui.read(|state| pages::page(&ui, state, window, cx)),
        };
        let main = col()
            .id("main")
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(rgb(look.content))
            .child(if in_session {
                self.ui
                    .read(|state| pages::session_header(&ui, state, collapsed, look))
                    .into_any_element()
            } else {
                top_bar(&ui, title, can_go_back, collapsed, look).into_any_element()
            })
            .children(restore_error.map(|error| {
                div().p_3().child(widgets::error_banner(
                    &ui,
                    "restore-error",
                    error,
                    AppAction::DismissRestoreError,
                ))
            }))
            .child(div().flex_1().min_h_0().flex().flex_col().child(body));
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
            .on_action(cx.listener(|this, _: &keys::ToggleSidebar, _, cx| {
                this.send(AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::GoHome, _, cx| {
                this.send(AppAction::Switch(Route::Home), cx)
            }))
            .on_action(cx.listener(|this, _: &keys::GoInbox, _, cx| {
                this.send(AppAction::Switch(Route::Inbox), cx)
            }))
            .on_action(cx.listener(|this, _: &keys::GoBoard, _, cx| {
                this.send(AppAction::Switch(Route::Board(Scope::Global)), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::FocusSidebar, window, cx| {
                window.focus(&this.sidebar_focus, cx);
            }))
            .on_action(cx.listener(|this, _: &keys::TogglePanel, _, cx| {
                this.send(AppAction::Pane(PaneAction::Toggle(PaneId::Panel)), cx);
            }))
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                let Some(pane) = this.dragging else { return };
                if event.pressed_button != Some(MouseButton::Left) {
                    this.dragging = None;
                    return;
                }
                let x = f32::from(event.position.x);
                let width = match pane {
                    PaneId::Sidebar => x - theme::RAIL_WIDTH,
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
                    .when(!collapsed, |shell| {
                        shell
                            .child(self.sidebar.clone())
                            .child(self.divider(PaneId::Sidebar, cx))
                    })
                    .child(main)
                    .when_some(panel, |shell, panel| {
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
                                    cx,
                                )
                            }))
                    })
                    .when(in_session, |shell| {
                        shell.child(panels::strip(&ui, panel, cx))
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
                            cx,
                        )
                    })
                }))
            })
            .child(self.overlays.clone())
    }
}
