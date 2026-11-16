//! What opens over the screen: the command palette (NV-6–10), the New
//! session (SE-1) and New channel (CH-1) sheets, the shortcuts, and toasts.
//!
//! The top overlay takes the keyboard: it is a focus trap, so Tab stays in
//! it, and when it closes focus goes back where it was. Clicking the scrim
//! or pressing Escape closes it.

use std::rc::Rc;

use crate::theme::Look;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, NewChannelAction, NewSessionAction, Overlay};
use humata2_palette::PaletteAction;
use humata2_session::SessionAction;

use crate::bridge::{Part, Ui};
use crate::theme;
use crate::widgets::{self, col};

#[path = "overlay_desk.rs"]
mod desk;
#[path = "overlay_fork.rs"]
mod fork;
#[path = "overlay_new_channel.rs"]
mod new_channel;
#[path = "overlay_new_session.rs"]
mod new_session;
#[path = "overlay_palette.rs"]
mod palette;
#[path = "overlay_shortcuts.rs"]
mod shortcuts;
#[path = "overlay_toasts.rs"]
mod toasts;

/// Which overlay shows, to tell when another opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Desk,
    Fork,
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
    fork_title: Entity<InputState>,
    first_message: Entity<TextareaState>,
    first_message_placeholder: String,
    name: Entity<InputState>,
    repos: Entity<InputState>,
    sheet: FocusHandle,
    desk: desk::Inputs,
    shown: Option<Kind>,
    restore: Option<FocusHandle>,
    restore_target: Option<crate::terminals::FocusTarget>,
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
        let fork_title =
            cx.new(|cx| InputState::new(window, cx).placeholder("What should the fork do?"));
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
                &fork_title,
                |text| AppAction::Session(SessionAction::ForkTitle(text)),
                AppAction::Session(SessionAction::ConfirmFork),
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
        let desk = desk::Inputs::new(window, cx, &mut subscriptions);
        Self {
            ui,
            query,
            title,
            fork_title,
            first_message,
            first_message_placeholder: "Optional. Typed into the agent once it starts.".into(),
            name,
            repos,
            sheet: cx.focus_handle(),
            desk,
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
            self.restore_target = Some(self.ui.read(crate::terminals::focus_target));
        }
        self.shown = top;
        match top {
            Some(Kind::Palette) => self.query.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::NewSession) => self.title.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::Fork) => self
                .fork_title
                .update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::NewChannel) => self.name.update(cx, |input, cx| input.focus(window, cx)),
            Some(Kind::Shortcuts) => window.focus(&self.sheet, cx),
            Some(Kind::Desk) => self.desk_focus(window, cx),
            None => {
                let same_target = self.restore_target.take().as_ref()
                    == Some(&self.ui.read(crate::terminals::focus_target));
                if let Some(focus) = self.restore.take()
                    && same_target
                {
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
        let (fork, forking, fork_error, parent) = self.ui.read(|state| {
            (
                state.session.fork.clone(),
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
        let shown = top
            .as_ref()
            .map(kind)
            .or_else(|| fork.as_ref().map(|_| Kind::Fork))
            .or_else(|| self.desk_open().then_some(Kind::Desk));
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
            Overlay::Shortcuts => self.shortcuts(look, &ui),
        });
        let content = content.or_else(|| {
            fork.map(|fork| self.fork(fork, forking, fork_error, parent, look, &ui, window, cx))
        });
        let content = match content {
            Some(content) => Some(content),
            None => self.desk(look, &ui, window, cx),
        };
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
                                Some(Kind::NewSession | Kind::Desk) => 70.,
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
        layer.child(toasts::stack(toasts, look, &ui))
    }
}
