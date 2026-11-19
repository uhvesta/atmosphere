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
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};
use ghostty_vt::{Colors, Rgb, Size, Terminal};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::machines::{LoginReason, StepKind, StepStatus, UnattendedMethod};
use humata2_hosts::add::{
    AddMachineAction, AddMachineState, Choice, MigrateStage, Step, Unattended, login_intro,
};
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

/// The banner that offers Open interactively: the line the connect stopped at, when it
/// printed one, then what the person has to do.
fn offer_text(reason: &LoginReason) -> String {
    match reason {
        LoginReason::Prompted { line, .. } if !line.trim().is_empty() => {
            format!("{line} The machine is waiting for you to log in.")
        }
        LoginReason::Prompted { .. } => "The machine is waiting for you to log in.".to_owned(),
        LoginReason::Hung => {
            "The connect command didn't finish on its own, so the machine may be waiting for you to log in."
                .to_owned()
        }
    }
}

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
        let _ = self.write_accepted(bytes);
    }
    fn write_accepted(&mut self, bytes: &[u8]) -> bool {
        self.tx.send(Io::Input(bytes.to_vec())).is_ok()
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
    /// The dialog's own focus scope. The trap tracks it, not an input's
    /// handle: an input's handle registered on the card too would make
    /// every press on the card (and every focus restore inside it) land in
    /// that input instead of the one clicked.
    trap: FocusHandle,
    /// The provider the pop-up picked when focus last moved into its
    /// fields, so a pick moves focus once and typing stays where it is.
    focused_pick: Option<String>,
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
            move |text| wrap(AddMachineAction::TypeHost(text)),
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
            trap: cx.focus_handle(),
            focused_pick: None,
            _subscriptions: subscriptions,
        }
    }

    /// Puts the keyboard in the dialog's first input, as the dialog opens.
    pub(crate) fn focus(&mut self, window: &mut Window, cx: &mut App) {
        self.focused_pick = None;
        self.host
            .input
            .update(cx, |input, cx| input.focus(window, cx));
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
        self.host.set(&state.typed, window, cx);
        self.name.set(&state.name, window, cx);
        let title = format!(
            "Add machine · {}",
            state.display_name().unwrap_or("new machine")
        );
        let body = sheet("add-machine", "Add machine", look)
            .w(px(620.))
            .max_w_full()
            // Grows to its content; scrolls inside when the window is too short.
            .max_h(window.viewport_size().height - px(60. + 24.))
            .overflow_y_scroll()
            .rounded(px(12.))
            .p(px(20.))
            .gap(px(12.))
            .text_size(px(12.5))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            );
        let body = if let Some(unattended) = &state.unattended {
            *self.enter.borrow_mut() = Some(wrap(AddMachineAction::MakeUnattended));
            let name = state.display_name().unwrap_or("this machine");
            Self::unattended(body, state, unattended, name, look, &ui, wrap)
        } else if let Some(login) = &state.login {
            *self.enter.borrow_mut() = None;
            self.login(
                body,
                state,
                login.finishing,
                login.error.as_deref(),
                look,
                &ui,
            )
        } else {
            self.main(body, state, look, window, cx)
        };
        // With focus on the card (after a failed step, or the unattended offer)
        // rather than in an input, Enter does what `self.enter` says, as the
        // inputs' PressEnter does.
        let (keys, enter, trap) = (ui.clone(), self.enter.clone(), self.trap.clone());
        let body = body.on_key_down(move |event, window, cx| {
            if event.keystroke.key == "enter"
                && !event.keystroke.modifiers.modified()
                && trap.is_focused(window)
            {
                let action = enter.borrow().clone();
                if let Some(action) = action {
                    cx.stop_propagation();
                    keys.send(action, cx);
                }
            }
        });
        body.test_support()
            .focus_trap("add-machine-trap", &self.trap)
            .into_any_element()
    }

    fn locked(state: &AddMachineState) -> bool {
        state.running || state.operation.is_some()
    }

    /// Whether Add has enough to start with.
    fn can_start(state: &AddMachineState) -> bool {
        if state.needs_variables().is_some() {
            return false;
        }
        match &state.effective_choice() {
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
        let cancel = widgets::push_quiet(
            &ui,
            "add-machine-cancel",
            "Cancel",
            wrap(AddMachineAction::Cancel),
            look,
        );
        let mut body = body;

        // Where to add: the pop-up's pick (a listed host or a provider), or a
        // typed host, which wins.
        let provider = match state.effective_choice() {
            Some(Choice::Provider { id, params }) => state
                .providers
                .iter()
                .find(|provider| provider.id == id)
                .map(|provider| (provider.clone(), params)),
            _ => None,
        };
        let picked = match &state.choice {
            Some(Choice::Host(name)) if !name.trim().is_empty() => name.clone(),
            Some(Choice::Provider { id, .. }) => state
                .providers
                .iter()
                .find(|provider| &provider.id == id)
                .map_or_else(|| id.clone(), |provider| provider.label.clone()),
            _ => "Choose a host".to_owned(),
        };
        body = body.child(
            row()
                .gap(px(10.))
                .child(
                    div()
                        .w(px(50.))
                        .flex_shrink_0()
                        .text_color(rgb(look.secondary))
                        .child("Host"),
                )
                .child(host_popup(&ui, state, picked, locked, look, wrap))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child("from ~/.ssh/config"),
                )
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.host.input)
                            .disabled(locked)
                            .aria_label("or type a host"),
                    ),
                ),
        );
        // A pick of a provider moves the keyboard into its first field.
        let pick = match &state.choice {
            Some(Choice::Provider { id, .. }) => Some(id.clone()),
            _ => None,
        };
        let moved = pick != self.focused_pick;
        self.focused_pick = pick;
        if let Some((provider, values)) = provider {
            let mut first = None;
            for param in &provider.params {
                let key = format!("{}/{}", provider.id, param.name);
                let input = self.param(&key, &param.name, &param.label, window, cx);
                if let Some(bound) = self.params.get(&key) {
                    let value = values.get(&param.name).map_or("", String::as_str);
                    bound.set(value, window, cx);
                }
                if first.is_none() {
                    first = Some(input.clone());
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
            if moved && !locked {
                let first = first.unwrap_or_else(|| self.name.input.clone());
                first.update(cx, |input, cx| input.focus(window, cx));
            }
        }

        if !state.steps.is_empty() {
            body = body.child(steps(look, &state.steps).test_support());
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
                            ))
                            // Named for tests and assistive tools: the command to copy.
                            .test_support(),
                    );
            }
            body = body.child(card.test_support());
        }
        if let Some(migrate) = &state.migrate {
            let mut card = col()
                .id("add-machine-needs-migrate")
                .role(Role::Alert)
                .aria_label("This Mac needs migrating")
                .gap(px(8.))
                .p(px(10.))
                .rounded(px(8.))
                .bg(rgb(look.attention_bg))
                .child(
                    row()
                        .items_start()
                        .gap(px(8.))
                        .child(chrome::icon(IconName::Info, ORANGE, 12.))
                        .child(div().flex_1().min_w_0().child(migrate.what.clone())),
                );
            match &migrate.stage {
                MigrateStage::Blocked => {}
                MigrateStage::Migrating(_) => {
                    card = card.child(
                        div()
                            .text_color(rgb(look.secondary))
                            .child("Migrating This Mac… the add runs again when it is done."),
                    );
                }
                MigrateStage::Failed(failure) => {
                    card = card.child(
                        div()
                            .text_color(rgb(RED))
                            .child(format!("{} {}", failure.message, failure.fix)),
                    );
                }
            }
            if state.migrate_offer().is_some() {
                card = card.child(row().gap(px(8.)).child(widgets::push_primary(
                    &ui,
                    "add-machine-migrate",
                    "Migrate This Mac",
                    wrap(AddMachineAction::MigrateThisMac),
                    look,
                )));
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
                            .child(div().flex_1().min_w_0().child(
                                "This host's key is new to this computer. Confirm only if the fingerprint matches what the host shows.",
                            )),
                    )
                    // The fingerprint is what the person checks against the
                    // host, so a screen reader (and the view) reads it too.
                    .child(mono(look.text, fingerprint.clone()).test_text(fingerprint.clone()))
                    .child(
                        row()
                            .gap(px(8.))
                            .child(widgets::push_primary(
                                &ui,
                                "add-machine-confirm-key",
                                "Confirm",
                                wrap(AddMachineAction::ConfirmHostKey),
                                look,
                            ))
                            .child(widgets::push_quiet(
                                &ui,
                                "add-machine-decline-key",
                                "Don't add",
                                wrap(AddMachineAction::DeclineHostKey),
                                look,
                            )),
                    )
                    .test_support(),
            );
        }
        if let Some(reason) = &state.login_offer {
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
                    .child(div().flex_1().child(offer_text(reason)))
                    .child(widgets::push_primary(
                        &ui,
                        "add-machine-open-login",
                        "Open interactively",
                        wrap(AddMachineAction::OpenLogin),
                        look,
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
        // The picked provider reads a host variable this Mac doesn't have.
        if let Some(needs) = state.needs_variables() {
            body = body.child(
                row()
                    .id("add-machine-needs-variable")
                    .role(Role::Alert)
                    .aria_label(needs.clone())
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().text_color(rgb(RED)).child(needs))
                    .child(widgets::push_quiet(
                        &ui,
                        "add-machine-open-settings",
                        "Open Settings",
                        wrap(AddMachineAction::ManageProviders),
                        look,
                    ))
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
                    widgets::push_primary(
                        &ui,
                        "add-machine-try-again",
                        "Try again",
                        action.clone(),
                        look,
                    )
                    .into_any_element(),
                ),
                Some(action),
            )
        } else if state.host_key.is_some() || state.login_offer.is_some() || state.migrate.is_some() {
            (None, None)
        } else {
            let action = wrap(AddMachineAction::Start);
            let ready = Self::can_start(state) && state.operation.is_none() && !state.loading;
            (
                Some(
                    widgets::push_primary(&ui, "add-machine-start", "Add", action.clone(), look)
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
        let name = state.display_name().unwrap_or("this machine");
        // Why it opened, said once when the terminal opens (D167).
        let intro = state
            .login
            .as_ref()
            .map(|login| login_intro(&state.who(), login.why, name))
            .unwrap_or_default();
        let output = match (&self.terminal, &self.terminal_error) {
            (Some(terminal), _) => div()
                .id("add-machine-terminal")
                .role(Role::Log)
                .aria_label("Login terminal")
                // Five 17 pt rows, as frame 57 draws it (D660).
                .h(px(107.))
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
                .child(div().flex_1().min_w_0().child(intro)),
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
                .child(div().flex_1().min_w_0().child(format!(
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
                .child(widgets::push_quiet(
                    ui,
                    "add-machine-cancel",
                    "Cancel",
                    wrap(AddMachineAction::Cancel),
                    look,
                ))
                .child(
                    widgets::push_primary(
                        ui,
                        "add-machine-login-done",
                        "Done, I'm logged in",
                        wrap(AddMachineAction::LoginDone),
                        look,
                    )
                    .disabled(finishing),
                ),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn unattended(
        body: Stateful<Div>,
        state: &AddMachineState,
        offer: &Unattended,
        name: &str,
        look: Look,
        ui: &Rc<Ui>,
        wrap: Wrap,
    ) -> Stateful<Div> {
        let (method, working) = (offer.method, offer.working);
        let headline = match &offer.version {
            Some(version) => format!("Logged in to {name} \u{b7} humatad {version} running"),
            None => format!("Logged in to {name}"),
        };
        let option = |id: &'static str,
                      selected: bool,
                      title: String,
                      note: String,
                      action: AddMachineAction| {
            let ui = ui.clone();
            let action = wrap(action);
            row()
                .id(id)
                .role(Role::RadioButton)
                .aria_label(title.clone())
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
                    col().child(div().child(title)).child(
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
                .child(div().font_weight(FontWeight::SEMIBOLD).child(headline)),
        )
        .child(steps(look, &state.steps).test_support())
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
                    "Add a Humata key to authorized_keys".to_owned(),
                    format!(
                        "Humata makes its own key and adds it on {name}. Reconnects won't ask you."
                    ),
                    AddMachineAction::ChooseMethod(UnattendedMethod::AddKey),
                ))
                // Only a provider has a token of its own to keep (MP-17).
                .children(offer.provider.as_ref().map(|provider| {
                    option(
                        "add-machine-method-token",
                        method == UnattendedMethod::KeepToken,
                        format!("Keep {provider}'s cached token"),
                        "Reconnects work while the token is valid, then need you again.".to_owned(),
                        AddMachineAction::ChooseMethod(UnattendedMethod::KeepToken),
                    )
                }))
                .child(option(
                    "add-machine-method-hand",
                    false,
                    "Keep logging in by hand".to_owned(),
                    format!("Reconnects need you: {name} is unreachable until you log in."),
                    AddMachineAction::SkipUnattended,
                )),
        )
        .child(
            row()
                .justify_end()
                .gap(px(8.))
                .pt(px(4.))
                .child(widgets::push_quiet(
                    ui,
                    "add-machine-skip",
                    "Not now",
                    wrap(AddMachineAction::SkipUnattended),
                    look,
                ))
                .child(
                    widgets::push_primary(
                        ui,
                        "add-machine-make-unattended",
                        if working {
                            "Making it unattended…"
                        } else {
                            "Make it unattended"
                        },
                        wrap(AddMachineAction::MakeUnattended),
                        look,
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

/// The Host pop-up (frame 56): its button says the pick, and while it is open
/// the SSH hosts, a divider and the providers hang under it, drawn over the
/// dialog. Whether it is open is the dialog state's (`host_menu`).
fn host_popup(
    ui: &Rc<Ui>,
    state: &AddMachineState,
    picked: String,
    locked: bool,
    look: Look,
    wrap: Wrap,
) -> Div {
    let toggle = wrap(AddMachineAction::HostMenu(!state.host_menu));
    let send = ui.clone();
    let button = row()
        .id("add-machine-host-popup")
        .role(Role::Button)
        .aria_label("Host")
        .w_full()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(9.))
        .pr(px(5.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(12.))
        .when(locked, |x| x.opacity(0.6))
        .when(!locked, |x| {
            x.cursor_pointer()
                .on_click(move |_, _, cx| send.send(toggle.clone(), cx))
        })
        .child(picked)
        .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
        .test_support();
    div()
        .relative()
        .w(px(180.))
        .flex_shrink_0()
        .child(button)
        .when(state.host_menu && !locked, |wrapper| {
            // Deferred, so both paint over the dialog's rows and take the
            // clicks there; the scrim first, the menu over it.
            wrapper
                .child(deferred(host_scrim(ui, wrap)))
                .child(deferred(host_menu(ui, state, look, wrap)))
        })
}

/// A transparent layer behind the open pop-up: a click on it closes the
/// pop-up and reaches nothing else.
fn host_scrim(ui: &Rc<Ui>, wrap: Wrap) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("add-machine-host-scrim")
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| ui.send(wrap(AddMachineAction::HostMenu(false)), cx))
}

/// The pop-up's entries: each SSH host, then each provider by its label, and
/// last, under a divider, Manage providers….
fn host_menu(ui: &Rc<Ui>, state: &AddMachineState, look: Look, wrap: Wrap) -> impl IntoElement {
    let entry = |id: String, label: String, selected: bool, action: AddMachineAction| {
        let ui = ui.clone();
        let action = wrap(action);
        row()
            .id(SharedString::from(id))
            .role(Role::ListBoxOption)
            .aria_label(label.clone())
            .aria_selected(selected)
            .gap(px(8.))
            .h(px(26.))
            .px(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            .hover(move |x| x.bg(rgb(look.fill)))
            .child(
                div()
                    .w(px(12.))
                    .flex_shrink_0()
                    .children(selected.then(|| chrome::icon(IconName::Check, look.accent, 12.))),
            )
            .child(div().flex_1().min_w_0().whitespace_nowrap().child(label))
            .on_click(move |_, _, cx| ui.send(action.clone(), cx))
            .test_support()
    };
    col()
        .id("add-machine-host-menu")
        .role(Role::ListBox)
        .aria_label("Hosts and providers")
        .absolute()
        .top(px(28.))
        .left(px(0.))
        .w(px(260.))
        .max_h(px(260.))
        .overflow_y_scroll()
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .when(
            state.hosts.is_empty() && state.providers.is_empty(),
            |list| {
                list.child(
                    div()
                        .px(px(8.))
                        .py(px(5.))
                        .text_color(rgb(look.tertiary))
                        .child("No hosts in ~/.ssh/config. Type one instead."),
                )
            },
        )
        .children(state.hosts.iter().map(|host| {
            let selected = matches!(&state.choice, Some(Choice::Host(name)) if name == &host.name);
            entry(
                format!("add-machine-host {}", host.name),
                host.name.clone(),
                selected,
                AddMachineAction::SelectHost(host.name.clone()),
            )
        }))
        .when(
            !state.hosts.is_empty() && !state.providers.is_empty(),
            |list| {
                list.child(
                    div()
                        .h(px(1.))
                        .mx(px(4.))
                        .my(px(3.))
                        .flex_shrink_0()
                        .bg(rgb(look.line)),
                )
            },
        )
        .children(state.providers.iter().map(|provider| {
            let selected = matches!(
                &state.choice,
                Some(Choice::Provider { id, .. }) if id == &provider.id
            );
            entry(
                format!("add-machine-provider {}", provider.id),
                provider.label.clone(),
                selected,
                AddMachineAction::SelectProvider(provider.id.clone()),
            )
        }))
        .child(
            div()
                .h(px(1.))
                .mx(px(4.))
                .my(px(3.))
                .flex_shrink_0()
                .bg(rgb(look.line)),
        )
        .child(entry(
            "add-machine-manage-providers".to_owned(),
            "Manage providers…".to_owned(),
            false,
            AddMachineAction::ManageProviders,
        ))
        .test_support()
}

/// The step list, each with its mark and the line under it.
fn steps(look: Look, steps: &[Step]) -> Stateful<Div> {
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
                StepStatus::Failed => chrome::icon(IconName::CircleX, RED, 12.).into_any_element(),
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
                        )
                        .children(sweeps(step.status).then(|| {
                            widgets::progress_bar(
                                look,
                                None,
                                widgets::BarMetrics::line(look),
                                ("add-machine-sweep", index),
                            )
                        })),
                )
                .test_support()
        }))
}

/// Only a running step shows the sweep: the machine's progress is not
/// measured, so the bar says work is on, not how far along it is.
fn sweeps(status: StepStatus) -> bool {
    status == StepStatus::Running
}

fn step_name(kind: StepKind) -> &'static str {
    kind.label()
}

#[cfg(test)]
mod tests {
    // Named, not `super::*`: the module's gpui glob brings in gpui's own
    // `test` attribute, which would shadow the built-in one.
    use ::core::prelude::v1::test;

    use super::sweeps;
    use humata2_clients::machines::StepStatus;

    #[test]
    fn only_a_running_step_sweeps() {
        assert!(sweeps(StepStatus::Running));
        assert!(!sweeps(StepStatus::Done));
        assert!(!sweeps(StepStatus::Failed));
        assert!(!sweeps(StepStatus::NotRun));
    }
}
