//! The window: a rail of screens, the sidebar, the screen in the middle and
//! the overlays above them. App-wide shortcuts are handled here.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::layout::PaneId;
use humata2_app::{AppAction, Load, PaneAction, Route};
use humata2_clients::model::Scope;
use humata2_sidebar::SidebarAction;

use crate::bridge::{Part, Ui};
use crate::keys;
use crate::overlays::Overlays;
use crate::pages;
use crate::sidebar::Sidebar;
use crate::theme::{self, color};
use crate::widgets::{self, col, row};

/// The window's root view.
pub struct Shell {
    ui: Rc<Ui>,
    sidebar: Entity<Sidebar>,
    sidebar_focus: FocusHandle,
    overlays: Entity<Overlays>,
    focus: FocusHandle,
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
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            ui,
            sidebar,
            sidebar_focus,
            overlays,
            focus,
        }
    }

    fn send(&self, action: AppAction, cx: &mut App) {
        self.ui.send(action, cx);
    }
}

/// One screen's button on the rail.
struct Screen {
    id: &'static str,
    label: &'static str,
    icon: IconName,
    route: Route,
    shortcut: &'static str,
}

fn screens() -> [Screen; 6] {
    [
        Screen {
            id: "rail-home",
            label: "Home",
            icon: IconName::House,
            route: Route::Home,
            shortcut: "⌘1",
        },
        Screen {
            id: "rail-inbox",
            label: "Inbox",
            icon: IconName::Inbox,
            route: Route::Inbox,
            shortcut: "⌘2",
        },
        Screen {
            id: "rail-board",
            label: "Board",
            icon: IconName::LayoutDashboard,
            route: Route::Board(Scope::Global),
            shortcut: "⌘3",
        },
        Screen {
            id: "rail-hosts",
            label: "Hosts",
            icon: IconName::Server,
            route: Route::Hosts,
            shortcut: "",
        },
        Screen {
            id: "rail-files",
            label: "Files",
            icon: IconName::Files,
            route: Route::Files,
            shortcut: "",
        },
        Screen {
            id: "rail-settings",
            label: "Settings",
            icon: IconName::Settings,
            route: Route::Settings,
            shortcut: "",
        },
    ]
}

const fn same_screen(a: &Route, b: &Route) -> bool {
    matches!(
        (a, b),
        (Route::Home, Route::Home)
            | (Route::Inbox, Route::Inbox)
            | (Route::Board(_), Route::Board(_))
            | (Route::Hosts, Route::Hosts)
            | (Route::Files, Route::Files)
            | (Route::Settings, Route::Settings)
    )
}

fn rail(ui: &Rc<Ui>, route: &Route, waiting: usize) -> impl IntoElement {
    col()
        .id("rail")
        .role(Role::Toolbar)
        .aria_label("Screens")
        .w(px(theme::RAIL_WIDTH))
        .h_full()
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .pt_3()
        .bg(color(theme::RAIL))
        .border_r_1()
        .border_color(color(theme::LINE))
        .children(screens().into_iter().map(|screen| {
            let on = same_screen(route, &screen.route);
            let tooltip = if screen.shortcut.is_empty() {
                screen.label.to_owned()
            } else {
                format!("{}  {}", screen.label, screen.shortcut)
            };
            let badge = (screen.label == "Inbox" && waiting > 0).then(|| {
                div()
                    .absolute()
                    .top(px(2.))
                    .right(px(2.))
                    .min_w(px(16.))
                    .h(px(16.))
                    .px_1()
                    .rounded_full()
                    .bg(color(theme::ORANGE))
                    .text_size(px(10.))
                    .text_color(color(theme::RAIL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(waiting.to_string())
            });
            div().relative().child(
                widgets::ghost(ui, screen.id, "", AppAction::Navigate(screen.route))
                    .icon(screen.icon)
                    .accessibility_label(screen.label)
                    .tooltip(tooltip)
                    .toggled(on)
                    .when(on, |button| button.primary()),
            )
            .children(badge)
        }))
        .test_support()
}

fn top_bar(ui: &Rc<Ui>, title: String, can_go_back: bool, sidebar_hidden: bool) -> impl IntoElement {
    row()
        .id("top-bar")
        .h(px(theme::TOP_BAR))
        .flex_shrink_0()
        .px_3()
        .gap_2()
        .border_b_1()
        .border_color(color(theme::LINE))
        .when(sidebar_hidden, |bar| {
            bar.child(
                widgets::ghost(ui, "show-sidebar", "", AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)))
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
                .text_color(color(theme::TEXT))
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
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (route, load, can_go_back, collapsed, title, waiting, restore_error) = self.ui.read(|state| {
            (
                state.route.clone(),
                state.load.clone(),
                !state.trail.is_empty(),
                state.layout.sidebar.collapsed,
                pages::title(state),
                state.workspace.requests.iter().filter(|request| request.answer.is_none()).count(),
                state.restore_error.clone(),
            )
        });
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
                .child(div().id("load-error").role(Role::Alert).aria_label(message.clone()).child(message).test_support())
                .child(widgets::primary(&ui, "retry", "Try again", AppAction::Retry))
                .into_any_element(),
            Load::Ready => self.ui.read(|state| pages::page(&ui, state, window, cx)),
        };
        let main = col()
            .id("main")
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(color(theme::BG))
            .child(top_bar(&ui, title, can_go_back, collapsed))
            .children(restore_error.map(|error| {
                div().p_3().child(widgets::error_banner(&ui, "restore-error", error, AppAction::DismissRestoreError))
            }))
            .child(div().flex_1().min_h_0().flex().flex_col().child(body));
        div()
            .id("shell")
            .relative()
            .size_full()
            .flex()
            .flex_row()
            .bg(color(theme::BG))
            .text_color(color(theme::TEXT))
            .text_size(px(13.))
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &keys::OpenPalette, _, cx| this.send(AppAction::OpenPalette, cx)))
            .on_action(cx.listener(|this, _: &keys::NewSession, _, cx| this.send(AppAction::StartNewSession, cx)))
            .on_action(cx.listener(|this, _: &keys::NewChannel, _, cx| this.send(AppAction::StartNewChannel, cx)))
            .on_action(cx.listener(|this, _: &keys::Back, _, cx| this.send(AppAction::Back, cx)))
            .on_action(cx.listener(|this, _: &keys::Dismiss, _, cx| this.send(AppAction::Escape, cx)))
            .on_action(cx.listener(|this, _: &keys::ShowShortcuts, _, cx| this.send(AppAction::ShowShortcuts, cx)))
            .on_action(cx.listener(|this, _: &keys::ExpandAll, _, cx| {
                this.send(AppAction::Sidebar(SidebarAction::ExpandAll), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::CollapseAll, _, cx| {
                this.send(AppAction::Sidebar(SidebarAction::CollapseAll), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::ToggleSidebar, _, cx| {
                this.send(AppAction::Pane(PaneAction::Toggle(PaneId::Sidebar)), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::GoHome, _, cx| this.send(AppAction::Navigate(Route::Home), cx)))
            .on_action(cx.listener(|this, _: &keys::GoInbox, _, cx| this.send(AppAction::Navigate(Route::Inbox), cx)))
            .on_action(cx.listener(|this, _: &keys::GoBoard, _, cx| {
                this.send(AppAction::Navigate(Route::Board(Scope::Global)), cx);
            }))
            .on_action(cx.listener(|this, _: &keys::FocusSidebar, window, cx| {
                window.focus(&this.sidebar_focus, cx);
            }))
            .child(rail(&ui, &route, waiting))
            .when(!collapsed, |shell| shell.child(self.sidebar.clone()))
            .child(main)
            .child(self.overlays.clone())
    }
}
