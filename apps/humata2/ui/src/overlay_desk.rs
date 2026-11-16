//! The Desk's dialogs (DK-4, DK-11, DK-13, DK-14): New desk session,
//! Clean up… and Copy to a channel…. Their text fields live here, and the
//! one entry point the overlay layer calls.

use super::Overlays;
use crate::bridge::Ui;
use crate::chrome;
use crate::desk::runs;
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::SessionId;
use humata2_desk::{DeskAction, DeskState, Menu as DeskMenu, NewAction};
use std::rc::Rc;

#[path = "overlay_desk_add_repo.rs"]
mod add_repo;
#[path = "overlay_desk_clean_up.rs"]
mod clean_up;
#[path = "overlay_desk_copy.rs"]
mod copy;
#[path = "overlay_desk_new.rs"]
mod new;

/// The orange of a warning line.
const ORANGE: u32 = 0xff9f0a;

/// The session whose folder bar asked for the Add repo sheet, if it is open.
fn adding_repo_to(desk: &DeskState) -> Option<SessionId> {
    match &desk.menu {
        Some(DeskMenu::AddRepo(session)) => Some(session.clone()),
        _ => None,
    }
}

/// The Desk dialogs' text fields.
pub(super) struct Inputs {
    title: Entity<InputState>,
    path: Entity<InputState>,
    repo: Entity<InputState>,
}

impl Inputs {
    /// Makes the fields and subscribes them: typing sends `NewAction::Title`
    /// and `NewAction::Path`, Enter in the title starts the session, and
    /// Enter in the repo field adds the repo.
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<Overlays>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("What is it for?"));
        let path = cx.new(|cx| InputState::new(window, cx).placeholder("~/notes"));
        let repo = cx.new(|cx| InputState::new(window, cx).placeholder("org/repo"));
        for (input, change, enter) in [
            (
                &title,
                (|text: String| Some(NewAction::Title(text))) as fn(String) -> Option<NewAction>,
                Some(NewAction::Start),
            ),
            (&path, |text| Some(NewAction::Path(text)), None),
        ] {
            subscriptions.push(cx.subscribe(
                input,
                move |this: &mut Overlays, input, event: &InputEvent, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        if let Some(action) = change(text) {
                            this.ui
                                .send(AppAction::Desk(DeskAction::New(action)), cx);
                        }
                    }
                    InputEvent::PressEnter { .. } => {
                        if let Some(action) = enter.clone() {
                            this.ui
                                .send(AppAction::Desk(DeskAction::New(action)), cx);
                        }
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            ));
        }
        subscriptions.push(cx.subscribe(
            &repo,
            |this: &mut Overlays, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    let text = input.read(cx).value().to_string();
                    // The same field serves the New desk session form and a
                    // session's Add repo sheet; it is emptied when either
                    // next opens.
                    let action = match this.ui.read(|state| adding_repo_to(&state.desk)) {
                        Some(session) => DeskAction::AddRepo(session, text),
                        None => DeskAction::New(NewAction::AddRepo(text)),
                    };
                    this.ui.send(AppAction::Desk(action), cx);
                }
            },
        ));
        Self { title, path, repo }
    }
}

impl Overlays {
    /// Whether a Desk dialog is open: one of the three, or a session's Add
    /// repo sheet.
    pub(super) fn desk_open(&self) -> bool {
        self.ui
            .read(|state| state.desk.sheet_open() || adding_repo_to(&state.desk).is_some())
    }

    /// Puts focus in the open Desk dialog: the title field for New desk
    /// session, the sheet itself otherwise.
    pub(super) fn desk_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ui.read(|state| state.desk.new.is_some()) {
            self.desk
                .title
                .update(cx, |input, cx| input.focus(window, cx));
        } else if self.ui.read(|state| adding_repo_to(&state.desk).is_some()) {
            self.desk.repo.update(cx, |input, cx| {
                input.set_value(String::new(), window, cx);
                input.focus(window, cx);
            });
        } else {
            window.focus(&self.sheet, cx);
        }
    }

    /// The open Desk dialog, if one is.
    pub(super) fn desk(
        &self,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let (new, clean_up, copy, workspace) = self.ui.read(|state| {
            (
                state.desk.new.clone(),
                state.desk.clean_up.clone(),
                state.desk.copy.clone(),
                state.desk.workspace.clone(),
            )
        });
        if let Some(form) = new {
            // The path the folder will get, with a suffix when it is taken.
            let preview = self
                .ui
                .read(|state| state.desk.scratch_preview())
                .unwrap_or_else(|| form.scratch_path());
            return Some(self.desk_new(&form, &preview, &workspace, look, ui, window, cx));
        }
        if let Some(dialog) = clean_up {
            return Some(self.desk_clean_up(&dialog, &workspace, look, ui));
        }
        let Some(dialog) = copy else {
            let session = self.ui.read(|state| adding_repo_to(&state.desk))?;
            return Some(self.desk_add_repo(&session, &workspace, look, ui, cx));
        };
        let machines: Vec<_> = self.ui.read(|state| {
            state
                .desk
                .copy_machines()
                .into_iter()
                .map(|machine| (machine.id.clone(), machine.name.clone()))
                .collect()
        });
        Some(self.desk_copy(&dialog, &workspace, &machines, look, ui))
    }
}

/// The dialog's card, `width` wide. Every dialog ends with
/// `.focus_trap(<the same id>, ..)`: the trap is the element the window
/// sees, and it draws this card under the trap's id, so a trap with another
/// id would hide the dialog's id from tests and flows.
fn card(
    id: &'static str,
    label: &'static str,
    width: f32,
    look: Look,
) -> ObservedElement<Stateful<Div>> {
    col()
        .id(id)
        .role(Role::Dialog)
        .aria_label(label)
        .w(px(width))
        .max_w_full()
        .max_h(relative(0.85))
        .overflow_y_scroll()
        .p(px(20.))
        .rounded(px(12.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .text_size(px(12.5))
        .text_color(rgb(look.text))
        .test_support()
}

/// A dialog's heading and the quiet words beside it.
fn header(look: Look, title: &'static str, beside: impl Into<SharedString>) -> Div {
    row()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(secondary(look, beside))
}

/// Quiet text.
fn secondary(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}

/// A form row as the frames draw it: the key on the left, right-aligned in
/// a column that gives way when the value needs the room, so a long line of
/// the value stays on one line (frame 40's folder caption).
fn form_row(look: Look, key: &'static str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(
            div()
                .w(px(110.))
                .pt(px(3.))
                .flex()
                .justify_end()
                .text_color(rgb(look.secondary))
                .child(key),
        )
        .child(div().flex_1().child(value))
}

/// A form row whose key column keeps its width and whose value wraps: for a
/// value too long to fit on one line however far the key gives way.
fn form_row_wrapping(look: Look, key: &'static str, value: impl IntoElement) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(12.))
        .child(
            div()
                .w(px(110.))
                .flex_shrink_0()
                .pt(px(3.))
                .flex()
                .justify_end()
                .text_color(rgb(look.secondary))
                .child(key),
        )
        .child(div().flex_1().min_w_0().child(value))
}

/// A secondary button sending `action`.
fn button(
    ui: &Rc<Ui>,
    look: Look,
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    action: AppAction,
) -> gpui_kit::component::button::Button {
    // The shared push button: the frames' 12 pt label, 24 pt high.
    crate::widgets::push_quiet(ui, id, text, action, look)
}

/// The main button, sending `action`; `busy` disables it.
fn primary(
    ui: &Rc<Ui>,
    look: Look,
    id: &'static str,
    text: impl Into<SharedString>,
    action: AppAction,
    busy: bool,
) -> gpui_kit::component::button::Button {
    use gpui_kit::base::Disableable as _;
    crate::widgets::push_primary(ui, id, text, action, look).disabled(busy)
}

/// A pop-up button showing `text`, sending `action`.
fn popup(
    ui: &Rc<Ui>,
    look: Look,
    id: &'static str,
    text: SharedString,
    action: AppAction,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::Button)
        .aria_label(text.clone())
        .test_support()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(9.))
        .pr(px(5.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(12.))
        .flex_shrink_0()
        .cursor_pointer()
        .child(text)
        .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

/// A check box with its words; clicking sends `action` unless `enabled` is
/// false.
fn check(
    ui: &Rc<Ui>,
    look: Look,
    id: ElementId,
    text: SharedString,
    on: bool,
    enabled: bool,
    action: AppAction,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(text.clone())
        .aria_selected(on)
        .test_support()
        .gap(px(7.))
        .child(
            row()
                .size(px(14.))
                .flex_shrink_0()
                .rounded(px(4.))
                .justify_center()
                .when(on, |x| {
                    x.bg(rgb(look.accent))
                        .child(chrome::icon(IconName::Check, 0xffffff, 10.))
                })
                .when(!on, |x| {
                    x.border_1()
                        .border_color(rgb(look.tertiary))
                        .bg(rgb(look.card))
                }),
        )
        .child(div().min_w_0().child(text))
        .when(enabled, |x| {
            x.cursor_pointer()
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
        })
}

/// A radio with its words and the note under them.
fn radio(
    ui: &Rc<Ui>,
    look: Look,
    id: &'static str,
    on: bool,
    text: &'static str,
    note: String,
    action: AppAction,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::RadioButton)
        .aria_label(text)
        .aria_selected(on)
        .test_support()
        .flex()
        .items_start()
        .gap(px(8.))
        .cursor_pointer()
        .child(
            row()
                .mt(px(2.))
                .size(px(14.))
                .flex_shrink_0()
                .rounded_full()
                .justify_center()
                .when(on, |x| {
                    x.bg(rgb(look.accent))
                        .child(div().size(px(5.)).rounded_full().bg(rgb(0xffffff)))
                })
                .when(!on, |x| {
                    x.border_1()
                        .border_color(rgb(look.tertiary))
                        .bg(rgb(look.card))
                }),
        )
        .child(
            col()
                .min_w_0()
                .child(div().child(text))
                .child(secondary(look, note)),
        )
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}
