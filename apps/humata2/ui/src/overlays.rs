//! What opens over the screen: the command palette (NV-6–10), the New
//! session (SE-1) and New channel (CH-1) sheets, the shortcuts, and toasts.
//!
//! The top overlay takes the keyboard: it is a focus trap, so Tab stays in
//! it, and when it closes focus goes back where it was. Clicking the scrim
//! or pressing Escape closes it.

use std::rc::Rc;

use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, NewChannelAction, NewSessionAction, Overlay};
use humata2_clients::model::Agent;
use humata2_palette::PaletteAction;

use crate::bridge::{Part, Ui};
use crate::keys::SHORTCUTS;
use crate::theme::{self, color};
use crate::widgets::{self, col, row};

/// Which overlay shows, to tell when another opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Palette,
    NewSession,
    NewChannel,
    Shortcuts,
}

const fn kind(overlay: &Overlay) -> Kind {
    match overlay {
        Overlay::Palette => Kind::Palette,
        Overlay::NewSession(_) => Kind::NewSession,
        Overlay::NewChannel(_) => Kind::NewChannel,
        Overlay::Shortcuts => Kind::Shortcuts,
    }
}

/// The overlays' view.
pub struct Overlays {
    ui: Rc<Ui>,
    query: Entity<InputState>,
    title: Entity<InputState>,
    name: Entity<InputState>,
    repos: Entity<InputState>,
    sheet: FocusHandle,
    shown: Option<Kind>,
    restore: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl Overlays {
    /// The overlays on `ui`.
    pub fn new(ui: Rc<Ui>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ui.attach(Part::Overlay, cx.entity_id());
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Type a command, a channel or a session"));
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("What should it do?"));
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("web"));
        let repos = cx.new(|cx| InputState::new(window, cx).placeholder("org/web, org/design"));
        let mut subscriptions = Vec::new();
        for (input, change, enter) in [
            (
                &query,
                (|text: String| AppAction::Palette(PaletteAction::Query(text))) as fn(String) -> AppAction,
                AppAction::Palette(PaletteAction::Run),
            ),
            (
                &title,
                |text| AppAction::NewSession(NewSessionAction::Title(text)),
                AppAction::NewSession(NewSessionAction::Create),
            ),
            (
                &name,
                |text| AppAction::NewChannel(NewChannelAction::Name(text)),
                AppAction::NewChannel(NewChannelAction::Create),
            ),
            (
                &repos,
                |text| AppAction::NewChannel(NewChannelAction::Repos(text)),
                AppAction::NewChannel(NewChannelAction::Create),
            ),
        ] {
            subscriptions.push(cx.subscribe(input, move |this: &mut Self, input, event: &InputEvent, cx| {
                match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.ui.send(change(text), cx);
                    }
                    InputEvent::PressEnter { .. } => this.ui.send(enter.clone(), cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                }
            }));
        }
        Self {
            ui,
            query,
            title,
            name,
            repos,
            sheet: cx.focus_handle(),
            shown: None,
            restore: None,
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
        }
        self.shown = top;
        match top {
            Some(Kind::Palette) => self.query.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::NewSession) => self.title.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::NewChannel) => self.name.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::Shortcuts) => window.focus(&self.sheet, cx),
            None => {
                if let Some(focus) = self.restore.take() {
                    window.focus(&focus, cx);
                }
            }
        }
    }
}

fn sync(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        let value = value.to_owned();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

fn sheet(id: &'static str, label: &'static str) -> Stateful<Div> {
    col()
        .id(id)
        .role(Role::Dialog)
        .aria_label(label)
        .w(px(theme::SHEET))
        .max_h(relative(0.8))
        .gap_3()
        .p_4()
        .rounded(px(10.))
        .bg(color(theme::PANEL))
        .border_1()
        .border_color(color(theme::LINE))
        .shadow_lg()
}

fn field(label: &'static str, input: impl IntoElement) -> Div {
    col().gap_1().child(widgets::dim(label).text_size(px(12.))).child(input)
}

impl Render for Overlays {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (top, palette, toasts) = self.ui.read(|state| {
            (state.overlay().cloned(), state.palette.clone(), state.toasts.clone())
        });
        let shown = top.as_ref().map(kind);
        self.follow_focus(shown, window, cx);
        let ui = self.ui.clone();
        let content = top.map(|top| match top {
            Overlay::Palette => {
                sync(&self.query, &palette.query, window, cx);
                let entries = palette.entries.iter().enumerate().map(|(index, entry)| {
                    let selected = index == palette.cursor;
                    let ui = ui.clone();
                    widgets::list_item(("palette-entry", index), Role::ListBoxOption, entry.title.clone(), selected)
                        .h(px(30.))
                        .child(div().flex_1().child(entry.title.clone()))
                        .child(widgets::faint(entry.hint.clone()))
                        .on_click(move |_, _, cx| ui.send(AppAction::Palette(PaletteAction::Pick(index)), cx))
                        .test_support()
                });
                sheet("palette", "Command palette")
                    .test_support()
                    .child(Input::new(&self.query).aria_label("Search commands"))
                    .child(
                        col()
                            .id("palette-entries")
                            .role(Role::ListBox)
                            .aria_label("Commands")
                            .overflow_y_scroll()
                            .max_h(px(380.))
                            .children(entries)
                            .when(palette.entries.is_empty(), |list| {
                                list.child(widgets::empty("Nothing matches.", "Try fewer letters, or a channel's name."))
                            }),
                    )
                    .child(widgets::faint("↑ ↓ to move · Enter to run · Esc to close"))
                    .focus_trap("palette-trap", &self.query.focus_handle(cx))
                    .into_any_element()
            }
            Overlay::NewSession(sheet_state) => {
                sync(&self.title, &sheet_state.title, window, cx);
                let (channels, machines) = self.ui.read(|state| {
                    (
                        state
                            .workspace
                            .channels
                            .iter()
                            .map(|channel| (channel.id.clone(), channel.name.clone()))
                            .collect::<Vec<_>>(),
                        state
                            .workspace
                            .machines
                            .iter()
                            .map(|machine| (machine.id.clone(), machine.name.clone(), machine.online))
                            .collect::<Vec<_>>(),
                    )
                });
                let channel_choices = row().flex_wrap().gap_1().children(channels.into_iter().enumerate().map(|(index, (id, name))| {
                    widgets::choice(
                        &ui,
                        ("new-session-channel", index),
                        format!("#{name}"),
                        sheet_state.channel.as_ref() == Some(&id),
                        AppAction::NewSession(NewSessionAction::Channel(id)),
                    )
                }));
                let agent_choices = row().flex_wrap().gap_1().children(Agent::ALL.into_iter().enumerate().map(|(index, agent)| {
                    widgets::choice(
                        &ui,
                        ("new-session-agent", index),
                        agent.name(),
                        sheet_state.agent == agent,
                        AppAction::NewSession(NewSessionAction::Agent(agent)),
                    )
                }));
                let machine_choices = row().flex_wrap().gap_1().children(machines.into_iter().enumerate().map(|(index, (id, name, online))| {
                    widgets::choice(
                        &ui,
                        ("new-session-machine", index),
                        if online { name } else { format!("{name} (offline)") },
                        sheet_state.machine.as_ref() == Some(&id),
                        AppAction::NewSession(NewSessionAction::Machine(id)),
                    )
                    .disabled(!online)
                }));
                sheet("new-session", "New session")
                    .test_support()
                    .child(widgets::title("New session"))
                    .child(field("Channel", channel_choices))
                    .child(field("Agent", agent_choices))
                    .child(field("Host", machine_choices))
                    .child(field("Task", Input::new(&self.title).aria_label("Task")))
                    .children(sheet_state.error.clone().map(|error| {
                        widgets::error_banner(&ui, "new-session-error", error, AppAction::Escape)
                    }))
                    .child(
                        row()
                            .justify_end()
                            .gap_2()
                            .child(widgets::ghost(&ui, "new-session-cancel", "Cancel", AppAction::Escape))
                            .child(
                                widgets::primary(
                                    &ui,
                                    "new-session-create",
                                    if sheet_state.creating { "Starting…" } else { "Start session" },
                                    AppAction::NewSession(NewSessionAction::Create),
                                )
                                .disabled(sheet_state.creating),
                            ),
                    )
                    .focus_trap("new-session-trap", &self.title.focus_handle(cx))
                    .into_any_element()
            }
            Overlay::NewChannel(sheet_state) => {
                sync(&self.name, &sheet_state.name, window, cx);
                sync(&self.repos, &sheet_state.repos, window, cx);
                sheet("new-channel", "New channel")
                    .test_support()
                    .child(widgets::title("New channel"))
                    .child(field("Name", Input::new(&self.name).aria_label("Channel name")))
                    .child(field(
                        "Repositories, separated by commas",
                        Input::new(&self.repos).aria_label("Repositories"),
                    ))
                    .children(sheet_state.error.clone().map(|error| {
                        widgets::error_banner(&ui, "new-channel-error", error, AppAction::Escape)
                    }))
                    .child(
                        row()
                            .justify_end()
                            .gap_2()
                            .child(widgets::ghost(&ui, "new-channel-cancel", "Cancel", AppAction::Escape))
                            .child(
                                widgets::primary(
                                    &ui,
                                    "new-channel-create",
                                    if sheet_state.creating { "Making…" } else { "Make channel" },
                                    AppAction::NewChannel(NewChannelAction::Create),
                                )
                                .disabled(sheet_state.creating),
                            ),
                    )
                    .focus_trap("new-channel-trap", &self.name.focus_handle(cx))
                    .into_any_element()
            }
            Overlay::Shortcuts => sheet("shortcuts", "Keyboard shortcuts")
                .test_support()
                .track_focus(&self.sheet)
                .child(widgets::title("Keyboard shortcuts"))
                .children(SHORTCUTS.iter().map(|(keys, what)| {
                    row()
                        .gap_3()
                        .child(div().w(px(120.)).text_color(color(theme::ACCENT2)).child(*keys))
                        .child(div().text_color(color(theme::TEXT)).child(*what))
                }))
                .child(row().justify_end().child(widgets::primary(&ui, "shortcuts-close", "Close", AppAction::Escape)))
                .focus_trap("shortcuts-trap", &self.sheet)
                .into_any_element(),
        });
        let layer = div().absolute().top_0().left_0().size_full();
        let layer = match content {
            Some(content) => {
                let ui = ui.clone();
                layer.child(
                    div()
                        .id("scrim")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(theme::scrim())
                        .occlude()
                        .on_click(move |_, _, cx| ui.send(AppAction::Escape, cx)),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .flex()
                        .justify_center()
                        .pt(px(96.))
                        .child(
                            // Clicks inside the sheet stay there.
                            div().id("sheet-holder").occlude().on_click(|_, _, cx| cx.stop_propagation()).child(content),
                        ),
                )
            }
            None => layer,
        };
        layer.child(
            col()
                .absolute()
                .bottom(px(16.))
                .right(px(16.))
                .gap_2()
                .children(toasts.into_iter().map(|toast| {
                    row()
                        .id(("toast", toast.id as usize))
                        .role(Role::Status)
                        .aria_label(toast.text.clone())
                        .gap_3()
                        .px_3()
                        .py_2()
                        .w(px(360.))
                        .rounded(px(8.))
                        .bg(color(theme::TOAST))
                        .border_1()
                        .border_color(color(theme::TOAST_LINE))
                        .occlude()
                        .child(div().flex_1().child(toast.text.clone()))
                        .child(
                            widgets::ghost(&ui, ("toast-dismiss", toast.id as usize), "", AppAction::DismissToast(toast.id))
                                .icon(gpui_kit::assets::IconName::X)
                                .xsmall()
                                .accessibility_label("Dismiss"),
                        )
                        .test_support()
                })),
        )
    }
}
