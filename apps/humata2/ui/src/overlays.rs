//! What opens over the screen: the command palette (NV-6–10), the New
//! session (SE-1) and New channel (CH-1) sheets, the shortcuts, and toasts.
//!
//! The top overlay takes the keyboard: it is a focus trap, so Tab stays in
//! it, and when it closes focus goes back where it was. Clicking the scrim
//! or pressing Escape closes it.

use std::rc::Rc;

use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, NewChannelAction, NewSessionAction, NewSessionPicker, Overlay};
use humata2_clients::model::Agent;
use humata2_palette::{Command, PaletteAction};

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
    first_message: Entity<TextareaState>,
    first_message_placeholder: String,
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
        let query = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Type a command, a channel or a session")
        });
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("What should it do?"));
        let first_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Optional. Typed into the agent once it starts.")
                .rows(3)
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder("web"));
        let repos = cx.new(|cx| InputState::new(window, cx).placeholder("org/web, org/design"));
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
            |this: &mut Self, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.ui.send(
                        AppAction::NewSession(NewSessionAction::FirstMessage(
                            input.read(cx).value().to_string(),
                        )),
                        cx,
                    );
                }
            },
        ));
        Self {
            ui,
            query,
            title,
            first_message,
            first_message_placeholder: "Optional. Typed into the agent once it starts.".into(),
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

fn sheet(id: &'static str, label: &'static str, look: Look) -> Stateful<Div> {
    col()
        .id(id)
        .role(Role::Dialog)
        .aria_label(label)
        .w(px(theme::SHEET))
        .max_h(relative(0.8))
        .gap_3()
        .p_4()
        .rounded(px(10.))
        .bg(rgb(look.card))
        .text_color(rgb(look.text))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
}

fn field(label: &'static str, input: impl IntoElement) -> Div {
    col()
        .gap_1()
        .child(widgets::dim(label).text_size(px(12.)))
        .child(input)
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
        let shown = top.as_ref().map(kind);
        self.follow_focus(shown, window, cx);
        let ui = self.ui.clone();
        let content = top.map(|top| match top {
            Overlay::Palette => {
                sync(&self.query, &palette.query, window, cx);
                let group = |command: &Command| match command {
                    Command::Session(_) => 0,
                    Command::File { .. } => 1,
                    Command::Channel(_) => 2,
                    _ => 3,
                };
                let mut list = col()
                    .id("palette-entries")
                    .role(Role::ListBox)
                    .aria_label("Search results")
                    .overflow_y_scroll()
                    .max_h(px(420.));
                for (index, name) in ["Sessions", "Files", "Channels", "Commands"]
                    .into_iter()
                    .enumerate()
                {
                    if !palette
                        .entries
                        .iter()
                        .any(|entry| group(&entry.command) == index)
                    {
                        continue;
                    }
                    list = list.child(
                        div()
                            .px(px(16.))
                            .pt(px(8.))
                            .pb(px(3.))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(look.secondary))
                            .child(name),
                    );
                    for (at, entry) in palette
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| group(&entry.command) == index)
                    {
                        let selected = at == palette.cursor;
                        let ui = ui.clone();
                        let color = if selected { 0xffffff } else { look.secondary };
                        let glyph = match &entry.command {
                            Command::Session(id) => {
                                if palette
                                    .workspace
                                    .session(id)
                                    .is_some_and(|s| s.parent.is_some())
                                {
                                    IconName::GitFork
                                } else {
                                    IconName::SquareTerminal
                                }
                            }
                            Command::File { .. } => IconName::FileCode,
                            Command::Channel(_) => IconName::FolderGit2,
                            Command::Fork => IconName::GitFork,
                            Command::TerminalFullScreen => IconName::Maximize2,
                            Command::TogglePanel => IconName::Columns2,
                            _ => IconName::ArrowRight,
                        };
                        let mut item = row()
                            .id(("palette-entry", at))
                            .role(Role::ListBoxOption)
                            .aria_label(entry.title.clone())
                            .aria_selected(selected)
                            .h(px(32.))
                            .mx(px(6.))
                            .px(px(10.))
                            .gap(px(9.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .when(selected, |x| {
                                x.bg(rgb(look.selection)).text_color(rgb(0xffffff))
                            })
                            .child(chrome::icon(glyph, color, 14.))
                            .child(
                                div()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(entry.title.clone()),
                            );
                        if index == 3 {
                            item = item.child(div().flex_1()).child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(rgb(color))
                                    .child(entry.hint.clone()),
                            );
                        } else {
                            item = item.child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(px(11.5))
                                    .text_color(rgb(color))
                                    .child(entry.hint.clone()),
                            );
                        }
                        list = list.child(
                            item.on_click(move |_, _, cx| {
                                ui.send(AppAction::Palette(PaletteAction::Pick(at)), cx)
                            })
                            .test_support(),
                        );
                    }
                }
                if palette.entries.is_empty() {
                    list = list.child(
                        div()
                            .p(px(16.))
                            .text_color(rgb(look.secondary))
                            .child("Nothing matches. Try fewer letters."),
                    );
                }
                col()
                    .id("palette")
                    .role(Role::Dialog)
                    .aria_label("Command palette")
                    .w(px(600.))
                    .max_w_full()
                    .pb(px(6.))
                    .rounded(px(12.))
                    .bg(rgb(look.card))
                    .border_1()
                    .border_color(rgb(look.line))
                    .shadow_lg()
                    .text_size(px(13.))
                    .text_color(rgb(look.text))
                    .test_support()
                    .child(
                        row()
                            .h(px(48.))
                            .px(px(16.))
                            .gap(px(10.))
                            .border_b_1()
                            .border_color(rgb(look.line))
                            .child(chrome::icon(IconName::Search, look.secondary, 14.))
                            .child(
                                Input::new(&self.query)
                                    .aria_label("Search commands")
                                    .border_0()
                                    .bg(rgb(look.card))
                                    .text_size(px(16.)),
                            ),
                    )
                    .child(list)
                    .focus_trap("palette-trap", &self.query.focus_handle(cx))
                    .into_any_element()
            }
            Overlay::NewSession(sheet_state) => {
                sync(&self.title, &sheet_state.title, window, cx);
                if self.first_message.read(cx).value().as_ref() != sheet_state.first_message {
                    self.first_message.update(cx, |input, cx| {
                        input.set_value(sheet_state.first_message.clone(), window, cx)
                    });
                }
                let placeholder = sheet_state.item.as_ref().map_or_else(
                    || "Optional. Typed into the agent once it starts.".into(),
                    |id| {
                        format!(
                            "Optional. Typed into the agent once it starts, e.g. “Work on {id}”."
                        )
                    },
                );
                if self.first_message_placeholder != placeholder {
                    self.first_message_placeholder = placeholder.clone();
                    self.first_message.update(cx, |input, cx| {
                        input.set_placeholder(placeholder, window, cx)
                    });
                }
                let workspace = self.ui.read(|s| s.workspace.clone());
                let channel = sheet_state
                    .channel
                    .as_ref()
                    .and_then(|id| workspace.channel(id));
                let machine = sheet_state
                    .machine
                    .as_ref()
                    .and_then(|id| workspace.machine(id));
                let label = |text: &str| {
                    div()
                        .w(px(110.))
                        .flex_shrink_0()
                        .flex()
                        .justify_end()
                        .text_color(rgb(look.secondary))
                        .child(text.to_owned())
                };
                let form_row = |text: &str, value: AnyElement| {
                    row()
                        .gap(px(12.))
                        .child(label(text))
                        .child(div().flex_1().min_w_0().child(value))
                };
                let button = |id: ElementId, text: SharedString, action: AppAction| {
                    widgets::send(&ui, id, text, action)
                        .small()
                        .h(px(24.))
                        .px(px(10.))
                        .bg(rgb(look.fill))
                        .text_color(rgb(look.text))
                };
                let context = format!(
                    "in # {}{}",
                    channel.map_or("Choose a channel", |c| c.name.as_str()),
                    sheet_state
                        .item
                        .as_ref()
                        .map_or_else(String::new, |id| format!(" · for {id}"))
                );
                let context_ui = ui.clone();
                let mut content = col()
                    .id("new-session")
                    .role(Role::Dialog)
                    .aria_label("New session")
                    .w(px(560.))
                    .max_w_full()
                    .max_h(relative(0.85))
                    .overflow_y_scroll()
                    .gap(px(12.))
                    .p(px(20.))
                    .rounded(px(12.))
                    .bg(rgb(look.card))
                    .border_1()
                    .border_color(rgb(look.line))
                    .shadow_lg()
                    .text_size(px(12.5))
                    .text_color(rgb(look.text))
                    .test_support()
                    .child(
                        row()
                            .gap(px(8.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("New session"),
                            )
                            .child(
                                div()
                                    .id("new-session-channel-picker")
                                    .role(Role::Button)
                                    .aria_label("Choose channel")
                                    .test_support()
                                    .text_size(px(11.5))
                                    .text_color(rgb(look.secondary))
                                    .cursor_pointer()
                                    .child(context)
                                    .on_click(move |_, _, cx| {
                                        context_ui.send(
                                            AppAction::NewSession(NewSessionAction::Picker(Some(
                                                NewSessionPicker::Channel,
                                            ))),
                                            cx,
                                        )
                                    }),
                            ),
                    );
                if sheet_state.picker == Some(NewSessionPicker::Channel) {
                    content = content.child(row().flex_wrap().gap(px(6.)).children(
                        workspace.channels.iter().enumerate().map(|(index, c)| {
                            button(
                                ("new-session-channel", index).into(),
                                format!("# {}", c.name).into(),
                                AppAction::NewSession(NewSessionAction::Channel(c.id.clone())),
                            )
                        }),
                    ));
                }
                content = content.child(form_row(
                    "Title",
                    Input::new(&self.title)
                        .small()
                        .h(px(26.))
                        .aria_label("Title")
                        .into_any_element(),
                ));
                let agents = row().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).children(
                    Agent::ALL.into_iter().enumerate().map(|(index, agent)| {
                        let ui = ui.clone();
                        let name = match agent {
                            Agent::Claude => "Claude Code",
                            Agent::Shell => "Terminal",
                            _ => agent.name(),
                        };
                        div()
                            .id(("new-session-agent", index))
                            .role(Role::Tab)
                            .aria_label(name)
                            .aria_selected(sheet_state.agent == agent)
                            .test_support()
                            .px(px(8.))
                            .py(px(2.))
                            .text_size(px(11.5))
                            .rounded(px(6.))
                            .when(sheet_state.agent == agent, |x| {
                                x.bg(rgb(look.card)).shadow_sm()
                            })
                            .cursor_pointer()
                            .child(name)
                            .on_click(move |_, _, cx| {
                                ui.send(AppAction::NewSession(NewSessionAction::Agent(agent)), cx)
                            })
                    }),
                );
                content = content.child(form_row("Runs", agents.into_any_element()));
                let machine_picker = row()
                    .gap(px(8.))
                    .child(
                        button(
                            "new-session-machine-picker".into(),
                            machine
                                .map_or("Choose a machine", |m| m.name.as_str())
                                .to_owned()
                                .into(),
                            AppAction::NewSession(NewSessionAction::Picker(Some(
                                NewSessionPicker::Machine,
                            ))),
                        )
                        .icon(IconName::ChevronsUpDown),
                    )
                    .children(
                        machine
                            .and_then(|m| workspace.machine_health.get(&m.id))
                            .map(|h| {
                                div()
                                    .text_size(px(11.5))
                                    .text_color(rgb(look.secondary))
                                    .child(format!("{} GB free", h.disk_free_gb))
                            }),
                    );
                content = content.child(form_row("Machine", machine_picker.into_any_element()));
                if sheet_state.picker == Some(NewSessionPicker::Machine) {
                    content = content.child(
                        row().flex_wrap().gap(px(6.)).children(
                            workspace
                                .machines
                                .iter()
                                .filter(|m| {
                                    m.online
                                        && channel.is_some_and(|c| {
                                            c.machines.contains(&m.id)
                                                || workspace
                                                    .sessions
                                                    .iter()
                                                    .any(|s| s.channel == c.id && s.machine == m.id)
                                        })
                                })
                                .enumerate()
                                .map(|(index, m)| {
                                    button(
                                        ("new-session-machine", index).into(),
                                        m.name.clone().into(),
                                        AppAction::NewSession(NewSessionAction::Machine(
                                            m.id.clone(),
                                        )),
                                    )
                                }),
                        ),
                    );
                }
                let repos =
                    col()
                        .gap(px(6.))
                        .children(channel.into_iter().flat_map(|c| &c.repos).map(|repo| {
                            row()
                                .gap(px(8.))
                                .child(chrome::icon(IconName::FolderGit2, look.secondary, 12.))
                                .child(
                                    div()
                                        .w(px(120.))
                                        .child(repo.rsplit('/').next().unwrap_or(repo).to_owned()),
                                )
                                .child(button(
                                    SharedString::from(format!("new-session-repo-{repo}")).into(),
                                    if sheet_state.excluded_repos.contains(repo) {
                                        "Leave this repo out".into()
                                    } else {
                                        "Own clone, new branch from main".into()
                                    },
                                    AppAction::NewSession(NewSessionAction::ToggleRepo(
                                        repo.clone(),
                                    )),
                                ))
                        }));
                content = content
                    .child(form_row("Repos", repos.into_any_element()))
                    .child(form_row(
                        "First message",
                        Textarea::new(&self.first_message)
                            .h(px(70.))
                            .aria_label("First message")
                            .into_any_element(),
                    ));
                if let Some(error) = &sheet_state.error {
                    content = content.child(
                        div()
                            .id("new-session-error")
                            .role(Role::Alert)
                            .test_support()
                            .text_color(rgb(0xff453a))
                            .child(error.clone()),
                    );
                }
                content
                    .child(
                        row()
                            .gap(px(8.))
                            .pt(px(4.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(px(11.5))
                                    .text_color(rgb(look.secondary))
                                    .child(format!("Branch: {}", sheet_state.branch())),
                            )
                            .child(button(
                                "new-session-cancel".into(),
                                "Cancel".into(),
                                AppAction::Escape,
                            ))
                            .child(
                                widgets::primary(
                                    &ui,
                                    "new-session-create",
                                    if sheet_state.creating {
                                        "Starting…"
                                    } else {
                                        "Start session"
                                    },
                                    AppAction::NewSession(NewSessionAction::Create),
                                )
                                .small()
                                .h(px(24.))
                                .bg(rgb(look.accent))
                                .text_color(rgb(0xffffff))
                                .disabled(sheet_state.creating),
                            ),
                    )
                    .focus_trap("new-session-trap", &self.title.focus_handle(cx))
                    .into_any_element()
            }
            Overlay::NewChannel(sheet_state) => {
                sync(&self.name, &sheet_state.name, window, cx);
                sync(&self.repos, &sheet_state.repos, window, cx);
                sheet("new-channel", "New channel", look)
                    .test_support()
                    .child(widgets::title("New channel"))
                    .child(field(
                        "Name",
                        Input::new(&self.name).aria_label("Channel name"),
                    ))
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
                            .child(widgets::ghost(
                                &ui,
                                "new-channel-cancel",
                                "Cancel",
                                AppAction::Escape,
                            ))
                            .child(
                                widgets::primary(
                                    &ui,
                                    "new-channel-create",
                                    if sheet_state.creating {
                                        "Making…"
                                    } else {
                                        "Make channel"
                                    },
                                    AppAction::NewChannel(NewChannelAction::Create),
                                )
                                .disabled(sheet_state.creating),
                            ),
                    )
                    .focus_trap("new-channel-trap", &self.name.focus_handle(cx))
                    .into_any_element()
            }
            Overlay::Shortcuts => sheet("shortcuts", "Keyboard shortcuts", look)
                .test_support()
                .track_focus(&self.sheet)
                .child(widgets::title("Keyboard shortcuts"))
                .children(SHORTCUTS.iter().map(|(keys, what)| {
                    row()
                        .gap_3()
                        .child(
                            div()
                                .w(px(120.))
                                .text_color(color(theme::ACCENT2))
                                .child(*keys),
                        )
                        .child(div().text_color(color(theme::TEXT)).child(*what))
                }))
                .child(row().justify_end().child(widgets::primary(
                    &ui,
                    "shortcuts-close",
                    "Close",
                    AppAction::Escape,
                )))
                .focus_trap("shortcuts-trap", &self.sheet)
                .into_any_element(),
        });
        let layer = div().absolute().top_0().left_0().size_full();
        let layer = match content {
            Some(content) => {
                let ui = ui.clone();
                layer
                    .child(
                        div()
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
                            .pt(px(match shown {
                                Some(Kind::Palette) => 110.,
                                Some(Kind::NewSession) => 70.,
                                _ => 96.,
                            }))
                            .child(
                                // Clicks inside the sheet stay there.
                                div()
                                    .id("sheet-holder")
                                    .max_w(relative(0.94))
                                    .occlude()
                                    .on_click(|_, _, cx| cx.stop_propagation())
                                    .child(content),
                            ),
                    )
            }
            None => layer,
        };
        layer.child(
            col()
                .absolute()
                .bottom(px(24.))
                .left_0()
                .w_full()
                .items_center()
                .gap(px(8.))
                .children(toasts.into_iter().map(|toast| {
                    row()
                        .id(("toast", toast.id as usize))
                        .role(Role::Status)
                        .aria_label(toast.text.clone())
                        .gap(px(14.))
                        .px(px(14.))
                        .py(px(9.))
                        .max_w(relative(0.94))
                        .rounded(px(10.))
                        .bg(rgb(if look.content == 0xffffff {
                            0x2c2c2e
                        } else {
                            0x3a3a3d
                        }))
                        .text_color(rgb(0xffffff))
                        .text_size(px(12.5))
                        .shadow_lg()
                        .occlude()
                        .child(chrome::icon(
                            if toast.action.is_some() {
                                IconName::Archive
                            } else {
                                IconName::Info
                            },
                            0xffffff,
                            12.,
                        ))
                        .child(div().child(toast.text.clone()))
                        .when(toast.action.is_some(), |x| {
                            x.child(
                                widgets::ghost(
                                    &ui,
                                    SharedString::from(format!("toast-undo-{}", toast.id)),
                                    if toast.working {
                                        "Restoring…"
                                    } else {
                                        "Undo"
                                    },
                                    AppAction::UndoToast(toast.id),
                                )
                                .text_color(rgb(0x64a8ff))
                                .font_weight(FontWeight::SEMIBOLD)
                                .disabled(toast.working),
                            )
                        })
                        .when(toast.action.is_none(), |x| {
                            x.child(
                                widgets::ghost(
                                    &ui,
                                    ("toast-dismiss", toast.id as usize),
                                    "",
                                    AppAction::DismissToast(toast.id),
                                )
                                .icon(IconName::X)
                                .xsmall()
                                .accessibility_label("Dismiss"),
                            )
                        })
                        .test_support()
                })),
        )
    }
}
