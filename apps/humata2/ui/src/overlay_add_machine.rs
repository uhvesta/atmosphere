//! The Add machine dialog (HO-1, HO-2, MP-12 to MP-17; frames 56 to 58).
//!
//! It draws an [`AddMachineState`] and sends [`AddMachineAction`]s; nothing
//! here decides anything. Every click is one action, and the text comes from
//! the state or is the frames' own wording. The login runs in the same
//! terminal pane the session screen uses, fed from the bytes in the state.
//!
//! This file is a child module of `overlays.rs` (it uses its `sheet` and
//! `sync` helpers through `super`).

use super::{sheet, sync};
use crate::bridge::Ui;
use crate::chrome;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use ghostty_vt::{Colors, Rgb, Size, Terminal};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::machines::{StepKind, StepStatus, UnattendedMethod};
use humata2_hosts::add::{AddMachineAction, AddMachineState, Choice, Step};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use terminal_pane::{Pty, TerminalPane};
use terminal_view::{CellSize, Theme};
use tokio::sync::mpsc;

const GREEN: u32 = 0x30d158;
const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;
const MONO: &str = "Menlo";

/// How the dialog's actions become the app's.
pub(crate) type Wrap = fn(AddMachineAction) -> AppAction;

/// An input and the text the view itself last put in it, so that putting a
/// value in doesn't send a change back.
struct Bound {
    input: Entity<InputState>,
    echo: Rc<RefCell<Option<String>>>,
}

impl Bound {
    fn set(&self, value: &str, window: &mut Window, cx: &mut App) {
        if self.input.read(cx).value().as_ref() != value {
            *self.echo.borrow_mut() = Some(value.to_owned());
            sync(&self.input, value, window, cx);
        }
    }
}

enum Io {
    Input(Vec<u8>),
    Resize(u16, u16),
}

struct LoginPty {
    tx: mpsc::UnboundedSender<Io>,
}

impl Pty for LoginPty {
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.tx.send(Io::Input(bytes.to_vec()));
    }
    fn resize(&mut self, size: Size, _: CellSize) {
        let _ = self.tx.send(Io::Resize(size.cols, size.rows));
    }
}

struct LoginTerminal {
    pane: Entity<TerminalPane>,
    read: usize,
    appearance: (u32, u32, u32),
}

/// The dialog's inputs and its login terminal. The overlays own one.
pub(crate) struct AddMachineDialog {
    ui: Rc<Ui>,
    wrap: Wrap,
    host: Bound,
    name: Bound,
    params: BTreeMap<String, Bound>,
    terminal: Option<LoginTerminal>,
    terminal_error: Option<String>,
    io: mpsc::UnboundedSender<Io>,
    /// What Enter does now; the view sets it as it draws.
    enter: Rc<RefCell<Option<AppAction>>>,
    _subscriptions: Vec<Subscription>,
}

fn colors(look: Look) -> Colors {
    let mut colors = Colors {
        foreground: Rgb::hex(look.term_text),
        background: Rgb::hex(look.term),
        ..Default::default()
    };
    colors.palette[8] = Rgb::hex(look.term_dim);
    colors.palette[2] = Rgb::hex(GREEN);
    colors
}

fn bind(
    ui: &Rc<Ui>,
    cx: &mut App,
    input: Entity<InputState>,
    enter: &Rc<RefCell<Option<AppAction>>>,
    change: impl Fn(String) -> AppAction + 'static,
    subscriptions: &mut Vec<Subscription>,
) -> Bound {
    let echo = Rc::new(RefCell::new(None::<String>));
    let (ui, enter, seen) = (ui.clone(), enter.clone(), echo.clone());
    subscriptions.push(cx.subscribe(
        &input,
        move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| match event {
            InputEvent::Change => {
                let text = input.read(cx).value().to_string();
                if seen.borrow().as_deref() == Some(text.as_str()) {
                    *seen.borrow_mut() = None;
                    return;
                }
                ui.send(change(text), cx);
            }
            InputEvent::PressEnter { .. } => {
                let action = enter.borrow().clone();
                if let Some(action) = action {
                    ui.send(action, cx);
                }
            }
            InputEvent::Focus | InputEvent::Blur => {}
        },
    ));
    Bound { input, echo }
}

impl AddMachineDialog {
    /// The dialog's inputs; `wrap` is the app action that carries a dialog
    /// action, such as `AppAction::AddMachine`.
    pub(crate) fn new(ui: Rc<Ui>, wrap: Wrap, window: &mut Window, cx: &mut App) -> Self {
        let enter = Rc::new(RefCell::new(None));
        let mut subscriptions = Vec::new();
        let host_input = cx.new(|cx| InputState::new(window, cx).placeholder("or type a host"));
        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name for the machine"));
        let host = bind(
            &ui,
            cx,
            host_input,
            &enter,
            move |text| wrap(AddMachineAction::SelectHost(text)),
            &mut subscriptions,
        );
        let name = bind(
            &ui,
            cx,
            name_input,
            &enter,
            move |text| wrap(AddMachineAction::SetName(text)),
            &mut subscriptions,
        );
        // One consumer keeps typed bytes and resizes in order.
        let (io, mut rx) = mpsc::unbounded_channel();
        let sender = ui.clone();
        cx.spawn(async move |cx| {
            while let Some(next) = rx.recv().await {
                let action = match next {
                    Io::Input(bytes) => AddMachineAction::LoginInput(bytes),
                    Io::Resize(columns, rows) => AddMachineAction::LoginResize { columns, rows },
                };
                cx.update(|cx| sender.send(wrap(action), cx));
            }
        })
        .detach();
        Self {
            ui,
            wrap,
            host,
            name,
            params: BTreeMap::new(),
            terminal: None,
            terminal_error: None,
            io,
            enter,
            _subscriptions: subscriptions,
        }
    }

    /// Puts the keyboard in the dialog's first input, as the dialog opens.
    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.host.input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn param(
        &mut self,
        key: &str,
        name: &str,
        label: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<InputState> {
        if let Some(bound) = self.params.get(key) {
            return bound.input.clone();
        }
        let label = label.to_owned();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(label));
        let wrap = self.wrap;
        let param = name.to_owned();
        let bound = bind(
            &self.ui,
            cx,
            input.clone(),
            &self.enter,
            move |text| wrap(AddMachineAction::SetParam(param.clone(), text)),
            &mut self._subscriptions,
        );
        self.params.insert(key.to_owned(), bound);
        input
    }

    /// Keeps the login terminal in step with the state: made when a login
    /// begins, fed the bytes it has not seen, dropped when the login ends.
    fn sync_terminal(
        &mut self,
        state: &AddMachineState,
        look: Look,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(login) = &state.login else {
            self.terminal = None;
            self.terminal_error = None;
            return;
        };
        if self
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.read > login.bytes.len())
        {
            self.terminal = None;
        }
        if self.terminal.is_none() && self.terminal_error.is_none() {
            match Terminal::new(100, 40, 10_000).and_then(|mut terminal| {
                terminal.set_default_colors(&colors(look))?;
                Ok(terminal)
            }) {
                Ok(terminal) => {
                    let pty = LoginPty {
                        tx: self.io.clone(),
                    };
                    let theme = Theme {
                        font_size: 11.5,
                        line_height: 17.,
                        ..Default::default()
                    };
                    let pane = cx.new(|cx| TerminalPane::new(terminal, pty, theme, window, cx));
                    let focus = pane.read(cx).focus_handle(cx);
                    window.focus(&focus, cx);
                    self.terminal = Some(LoginTerminal {
                        pane,
                        read: 0,
                        appearance: (look.term_text, look.term, look.term_dim),
                    });
                }
                Err(source) => {
                    self.terminal_error = Some(format!(
                        "The terminal could not start: {source}. Close this dialog and add the machine again."
                    ));
                }
            }
        }
        if let Some(terminal) = &mut self.terminal {
            let appearance = (look.term_text, look.term, look.term_dim);
            let changed = terminal.appearance != appearance;
            terminal.appearance = appearance;
            let read = terminal.read;
            terminal.pane.update(cx, |pane, cx| {
                if login.bytes.len() > read {
                    pane.feed(&login.bytes[read..], cx);
                }
                pane.view().update(cx, |view, cx| {
                    if changed && view.grid_mut().set_default_colors(&colors(look)).is_ok() {
                        cx.notify();
                    }
                });
            });
            terminal.read = login.bytes.len();
        }
    }

    /// The dialog for `state`.
    pub(crate) fn draw(
        &mut self,
        state: &AddMachineState,
        look: Look,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let wrap = self.wrap;
        let ui = self.ui.clone();
        self.sync_terminal(state, look, window, cx);
        let host_text = match &state.choice {
            Some(Choice::Host(host)) => host.as_str(),
            _ => "",
        };
        self.host.set(host_text, window, cx);
        self.name.set(&state.name, window, cx);
        let title = format!(
            "Add machine · {}",
            state.display_name().unwrap_or("new machine")
        );
        let body = sheet("add-machine", "Add machine", look)
            .w(px(620.))
            .max_w_full()
            .p(px(20.))
            .gap(px(12.))
            .text_size(px(12.5))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            );
        let (body, handle) = if let Some(unattended) = &state.unattended {
            *self.enter.borrow_mut() = Some(wrap(AddMachineAction::MakeUnattended));
            let name = state.display_name().unwrap_or("this machine");
            (
                Self::unattended(body, state, unattended.method, unattended.working, name, look, &ui, wrap),
                self.host.input.focus_handle(cx),
            )
        } else if let Some(login) = &state.login {
            *self.enter.borrow_mut() = None;
            let handle = match &self.terminal {
                Some(terminal) => terminal.pane.read(cx).focus_handle(cx),
                None => self.host.input.focus_handle(cx),
            };
            (
                self.login(body, state, login.finishing, login.error.as_deref(), look, &ui),
                handle,
            )
        } else {
            let body = self.main(body, state, look, window, cx);
            (body, self.host.input.focus_handle(cx))
        };
        body.test_support()
            .focus_trap("add-machine-trap", &handle)
            .into_any_element()
    }

    fn locked(state: &AddMachineState) -> bool {
        state.running || state.operation.is_some()
    }

    /// Whether Add has enough to start with.
    fn can_start(state: &AddMachineState) -> bool {
        match &state.choice {
            Some(Choice::Host(host)) => !host.trim().is_empty(),
            Some(Choice::Provider { id, params }) => state
                .providers
                .iter()
                .find(|provider| &provider.id == id)
                .is_some_and(|provider| {
                    provider.params.iter().all(|param| {
                        !param.required
                            || params
                                .get(&param.name)
                                .is_some_and(|value| !value.trim().is_empty())
                    })
                }),
            None => false,
        }
    }

    fn main(
        &mut self,
        body: Stateful<Div>,
        state: &AddMachineState,
        look: Look,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let wrap = self.wrap;
        let ui = self.ui.clone();
        let locked = Self::locked(state);
        let cancel = widgets::send(
            &ui,
            "add-machine-cancel",
            "Cancel",
            wrap(AddMachineAction::Cancel),
        );
        let mut body = body;

        // Where to add: a listed host, a provider, or a typed host.
        let mut chips = row().flex_wrap().gap(px(6.));
        for host in &state.hosts {
            let selected = matches!(&state.choice, Some(Choice::Host(name)) if name == &host.name);
            chips = chips.child(chip(
                &ui,
                format!("add-machine-host {}", host.name),
                &host.name,
                selected,
                locked,
                wrap(AddMachineAction::SelectHost(host.name.clone())),
                look,
            ));
        }
        for provider in &state.providers {
            let selected =
                matches!(&state.choice, Some(Choice::Provider { id, .. }) if id == &provider.id);
            chips = chips.child(chip(
                &ui,
                format!("add-machine-provider {}", provider.id),
                &provider.label,
                selected,
                locked,
                wrap(AddMachineAction::SelectProvider(provider.id.clone())),
                look,
            ));
        }
        let provider = match &state.choice {
            Some(Choice::Provider { id, params }) => state
                .providers
                .iter()
                .find(|provider| &provider.id == id)
                .map(|provider| (provider.clone(), params.clone())),
            _ => None,
        };
        body = body.child(
            row()
                .items_start()
                .gap(px(10.))
                .child(
                    div()
                        .w(px(50.))
                        .flex_shrink_0()
                        .pt(px(4.))
                        .text_color(rgb(look.secondary))
                        .child("Host"),
                )
                .child(
                    col()
                        .flex_1()
                        .min_w_0()
                        .gap(px(6.))
                        .child(chips)
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child("from ~/.ssh/config, or type a host"),
                        ),
                )
                .child(
                    div().w(px(150.)).flex_shrink_0().child(
                        Input::new(&self.host.input)
                            .disabled(locked || provider.is_some())
                            .aria_label("Host"),
                    ),
                ),
        );
        if let Some((provider, values)) = provider {
            for param in &provider.params {
                let key = format!("{}/{}", provider.id, param.name);
                let input = self.param(&key, &param.name, &param.label, window, cx);
                if let Some(bound) = self.params.get(&key) {
                    let value = values.get(&param.name).map_or("", String::as_str);
                    bound.set(value, window, cx);
                }
                body = body.child(
                    row()
                        .gap(px(10.))
                        .child(
                            div()
                                .w(px(110.))
                                .flex_shrink_0()
                                .text_color(rgb(look.secondary))
                                .child(param.label.clone()),
                        )
                        .child(
                            div().flex_1().child(
                                Input::new(&input)
                                    .disabled(locked)
                                    .aria_label(param.label.clone()),
                            ),
                        ),
                );
            }
            body = body.child(
                row()
                    .gap(px(10.))
                    .child(
                        div()
                            .w(px(110.))
                            .flex_shrink_0()
                            .text_color(rgb(look.secondary))
                            .child("Name"),
                    )
                    .child(
                        div().flex_1().child(
                            Input::new(&self.name.input)
                                .disabled(locked)
                                .aria_label("Machine name"),
                        ),
                    ),
            );
        }

        if !state.steps.is_empty() {
            body = body.child(steps(look, &state.steps));
        }
        if let Some(failure) = &state.failure {
            let mut card = col()
                .id("add-machine-failure")
                .role(Role::Alert)
                .aria_label(failure.title.clone())
                .gap(px(6.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(rgb(look.del))
                .border_1()
                .border_color(rgb(RED))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(RED))
                        .child(failure.title.clone()),
                )
                .child(div().child(failure.what.clone()));
            if let Some(fix) = &failure.fix {
                card = card
                    .child(div().child(format!(
                        "To fix it, run this on {}, then Try again:",
                        fix.where_to_run
                    )))
                    .child(
                        col()
                            .id("add-machine-fix")
                            .role(Role::Code)
                            .aria_label(fix.command.clone())
                            .px(px(8.))
                            .py(px(5.))
                            .rounded(px(6.))
                            .bg(rgb(look.term))
                            .border_1()
                            .border_color(rgb(look.line))
                            .child(mono(look.term_text, fix.command.clone()))
                            .child(mono(
                                look.term_dim,
                                format!("# run this on {}, then Try again", fix.where_to_run),
                            )),
                    );
            }
            body = body.child(card.test_support());
        }
        if let Some(fingerprint) = &state.host_key {
            body = body.child(
                col()
                    .id("add-machine-host-key")
                    .role(Role::Group)
                    .aria_label("Host key")
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(look.attention_bg))
                    .child(
                        row()
                            .items_start()
                            .gap(px(8.))
                            .child(chrome::icon(IconName::Info, ORANGE, 12.))
                            .child(div().child(
                                "This host's key is new to this computer. Confirm only if the fingerprint matches what the host shows.",
                            )),
                    )
                    .child(mono(look.text, fingerprint.clone()))
                    .child(
                        row()
                            .gap(px(8.))
                            .child(widgets::primary(
                                &ui,
                                "add-machine-confirm-key",
                                "Confirm",
                                wrap(AddMachineAction::ConfirmHostKey),
                            ))
                            .child(widgets::send(
                                &ui,
                                "add-machine-decline-key",
                                "Don't add",
                                wrap(AddMachineAction::DeclineHostKey),
                            )),
                    )
                    .test_support(),
            );
        }
        if let Some(prompt) = &state.login_offer {
            body = body.child(
                row()
                    .id("add-machine-login-offer")
                    .role(Role::Group)
                    .aria_label("Login needed")
                    .items_start()
                    .gap(px(8.))
                    .p(px(10.))
                    .rounded(px(8.))
                    .bg(rgb(look.fill))
                    .child(chrome::icon(IconName::SquareTerminal, look.accent, 12.))
                    .child(div().flex_1().child(format!(
                        "{prompt} The machine is waiting for you to log in."
                    )))
                    .child(widgets::primary(
                        &ui,
                        "add-machine-open-login",
                        "Open interactively",
                        wrap(AddMachineAction::OpenLogin),
                    ))
                    .test_support(),
            );
        }
        if let Some(error) = &state.error {
            body = body.child(
                div()
                    .id("add-machine-error")
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .text_color(rgb(RED))
                    .child(error.clone())
                    .test_support(),
            );
        }

        // Buttons, and what Enter does.
        let (primary, enter) = if state.running {
            (None, None)
        } else if state.failure.is_some() {
            let action = wrap(AddMachineAction::TryAgain);
            (
                Some(
                    widgets::primary(&ui, "add-machine-try-again", "Try again", action.clone())
                        .into_any_element(),
                ),
                Some(action),
            )
        } else if state.host_key.is_some() || state.login_offer.is_some() {
            (None, None)
        } else {
            let action = wrap(AddMachineAction::Start);
            let ready = Self::can_start(state) && state.operation.is_none() && !state.loading;
            (
                Some(
                    widgets::primary(&ui, "add-machine-start", "Add", action.clone())
                        .disabled(!ready)
                        .into_any_element(),
                ),
                ready.then_some(action),
            )
        };
        *self.enter.borrow_mut() = enter;
        body.child(
            row()
                .justify_end()
                .gap(px(8.))
                .pt(px(4.))
                .child(cancel)
                .children(primary),
        )
    }

    fn login(
        &self,
        body: Stateful<Div>,
        state: &AddMachineState,
        finishing: bool,
        error: Option<&str>,
        look: Look,
        ui: &Rc<Ui>,
    ) -> Stateful<Div> {
        let wrap = self.wrap;
        let intro = match &state.login_offer {
            Some(prompt) => format!(
                "{prompt} Humata opened the login here. Finish logging in, then press Done."
            ),
            None => "Humata opened the login here. Finish logging in, then press Done.".to_owned(),
        };
        let name = state.display_name().unwrap_or("this machine");
        let output = match (&self.terminal, &self.terminal_error) {
            (Some(terminal), _) => div()
                .id("add-machine-terminal")
                .role(Role::Log)
                .aria_label("Login terminal")
                .h(px(190.))
                .px(px(12.))
                .py(px(10.))
                .overflow_hidden()
                .child(terminal.pane.clone())
                .test_support()
                .into_any_element(),
            (None, Some(error)) => div()
                .id("add-machine-terminal")
                .role(Role::Alert)
                .aria_label(error.clone())
                .p(px(12.))
                .text_color(rgb(RED))
                .child(error.clone())
                .into_any_element(),
            (None, None) => div().into_any_element(),
        };
        body.child(
            row()
                .items_start()
                .gap(px(8.))
                .child(chrome::icon(IconName::SquareTerminal, look.accent, 12.))
                .child(div().child(intro)),
        )
        .child(
            div()
                .rounded(px(8.))
                .bg(rgb(look.term))
                .border_1()
                .border_color(rgb(look.line))
                .overflow_hidden()
                .child(output),
        )
        .child(
            row()
                .id("add-machine-notice")
                .role(Role::Status)
                .aria_label("Reconnecting will need you")
                .items_start()
                .gap(px(8.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(rgb(look.attention_bg))
                .child(chrome::icon(IconName::Info, ORANGE, 12.))
                .child(div().child(format!(
                    "Reconnecting will need you: sessions keep running, but {name} can't be reached until you log in again."
                )))
                .test_support(),
        )
        .children(error.map(|error| {
            div()
                .id("add-machine-login-error")
                .role(Role::Alert)
                .aria_label(error.to_owned())
                .text_color(rgb(RED))
                .child(error.to_owned())
                .test_support()
        }))
        .child(
            row()
                .justify_end()
                .gap(px(8.))
                .pt(px(4.))
                .child(widgets::send(
                    ui,
                    "add-machine-cancel",
                    "Cancel",
                    wrap(AddMachineAction::Cancel),
                ))
                .child(
                    widgets::primary(
                        ui,
                        "add-machine-login-done",
                        "Done, I'm logged in",
                        wrap(AddMachineAction::LoginDone),
                    )
                    .disabled(finishing),
                ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn unattended(
        body: Stateful<Div>,
        state: &AddMachineState,
        method: UnattendedMethod,
        working: bool,
        name: &str,
        look: Look,
        ui: &Rc<Ui>,
        wrap: Wrap,
    ) -> Stateful<Div> {
        let option = |id: &'static str,
                      selected: bool,
                      title: &'static str,
                      note: String,
                      action: AddMachineAction| {
            let ui = ui.clone();
            let action = wrap(action);
            row()
                .id(id)
                .role(Role::RadioButton)
                .aria_label(title)
                .aria_selected(selected)
                .items_start()
                .gap(px(8.))
                .cursor_pointer()
                .child(
                    div()
                        .mt(px(2.))
                        .size(px(14.))
                        .flex_shrink_0()
                        .rounded_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |x| {
                            x.bg(rgb(look.accent))
                                .child(div().size(px(5.)).rounded_full().bg(rgb(0xffffff)))
                        })
                        .when(!selected, |x| {
                            x.border_1()
                                .border_color(rgb(look.tertiary))
                                .bg(rgb(look.card))
                        }),
                )
                .child(
                    col()
                        .child(div().child(title))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child(note),
                        ),
                )
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
                .test_support()
        };
        body.child(
            row()
                .gap(px(7.))
                .child(chrome::icon(IconName::Check, GREEN, 12.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("Logged in to {name}")),
                ),
        )
        .child(steps(look, &state.steps))
        .child(div().h(px(1.)).w_full().flex_shrink_0().bg(rgb(look.line)))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Reconnect without you?"),
        )
        .child(
            col()
                .gap(px(9.))
                .child(option(
                    "add-machine-method-key",
                    method == UnattendedMethod::AddKey,
                    "Add a Humata key to authorized_keys",
                    format!("Humata makes its own key and adds it on {name}. Reconnects won't ask you."),
                    AddMachineAction::ChooseMethod(UnattendedMethod::AddKey),
                ))
                .child(option(
                    "add-machine-method-token",
                    method == UnattendedMethod::KeepToken,
                    "Keep the cached token",
                    "Reconnects work while the token is valid, then need you again.".to_owned(),
                    AddMachineAction::ChooseMethod(UnattendedMethod::KeepToken),
                ))
                .child(option(
                    "add-machine-method-hand",
                    false,
                    "Keep logging in by hand",
                    format!("Reconnects need you: {name} is unreachable until you log in."),
                    AddMachineAction::SkipUnattended,
                )),
        )
        .child(
            row()
                .justify_end()
                .gap(px(8.))
                .pt(px(4.))
                .child(widgets::send(
                    ui,
                    "add-machine-skip",
                    "Not now",
                    wrap(AddMachineAction::SkipUnattended),
                ))
                .child(
                    widgets::primary(
                        ui,
                        "add-machine-make-unattended",
                        if working {
                            "Making it unattended…"
                        } else {
                            "Make it unattended"
                        },
                        wrap(AddMachineAction::MakeUnattended),
                    )
                    .disabled(working),
                ),
        )
    }
}

fn mono(hex: u32, text: impl Into<SharedString>) -> Div {
    div()
        .whitespace_nowrap()
        .font_family(MONO)
        .text_size(px(11.5))
        .text_color(rgb(hex))
        .child(text.into())
}

/// A pick-one chip.
fn chip(
    ui: &Rc<Ui>,
    id: String,
    label: &str,
    selected: bool,
    locked: bool,
    action: AppAction,
    look: Look,
) -> Stateful<Div> {
    let ui = ui.clone();
    div()
        .id(SharedString::from(id))
        .role(Role::RadioButton)
        .aria_label(label.to_owned())
        .aria_selected(selected)
        .h(px(24.))
        .px(px(9.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .text_size(px(12.))
        .border_1()
        .border_color(rgb(if selected { look.accent } else { look.line }))
        .bg(rgb(look.fill))
        .when(selected, |x| x.font_weight(FontWeight::MEDIUM))
        .when(locked, |x| x.opacity(0.6))
        .when(!locked, |x| {
            x.cursor_pointer()
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
        })
        .child(label.to_owned())
        .test_support()
}

/// The step list, each with its mark and the line under it.
fn steps(look: Look, steps: &[Step]) -> Div {
    col()
        .id("add-machine-steps")
        .role(Role::List)
        .aria_label("Steps")
        .gap(px(9.))
        .p(px(12.))
        .rounded(px(8.))
        .bg(rgb(look.fill))
        .children(steps.iter().enumerate().map(|(index, step)| {
            let mark = match step.status {
                StepStatus::Done => chrome::icon(IconName::Check, GREEN, 12.).into_any_element(),
                StepStatus::Failed => {
                    chrome::icon(IconName::CircleX, RED, 12.).into_any_element()
                }
                StepStatus::Running => div()
                    .size(px(9.))
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(look.accent))
                    .into_any_element(),
                StepStatus::NotRun => div()
                    .size(px(9.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(look.tertiary))
                    .into_any_element(),
            };
            let state = match step.status {
                StepStatus::Done => "done",
                StepStatus::Failed => "failed",
                StepStatus::Running => "running",
                StepStatus::NotRun => "not run",
            };
            let detail = if step.detail.is_empty() && step.status == StepStatus::NotRun {
                "Not run".to_owned()
            } else {
                step.detail.clone()
            };
            let name = step_name(step.kind);
            row()
                .id(("add-machine-step", index))
                .role(Role::ListItem)
                .aria_label(format!("{name}: {state}"))
                .items_start()
                .gap(px(9.))
                .when(step.status == StepStatus::NotRun, |x| x.opacity(0.6))
                .child(
                    div()
                        .size(px(14.))
                        .flex_shrink_0()
                        .mt(px(1.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(mark),
                )
                .child(
                    col()
                        .flex_1()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(if step.status == StepStatus::Failed {
                                    RED
                                } else {
                                    look.text
                                }))
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child(detail),
                        ),
                )
                .test_support()
        }))
}

fn step_name(kind: StepKind) -> &'static str {
    kind.label()
}
