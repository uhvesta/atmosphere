//! Small building blocks every screen uses: rows and columns, text styles,
//! buttons, pills, status dots and empty states. Each control that does
//! something carries a role and a label, so a person with a screen reader,
//! a flow test and a recording all find it by what it says.

use std::rc::Rc;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::Status;

use crate::bridge::Ui;
use crate::theme::{self, color};

/// A horizontal flex row, items centered.
#[must_use]
pub fn row() -> Div {
    div().flex().flex_row().items_center()
}

/// A vertical flex column.
#[must_use]
pub fn col() -> Div {
    div().flex().flex_col()
}

/// A page's title.
#[must_use]
pub fn title(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(20.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color(theme::TEXT))
        .child(text.into())
}

/// Secondary text.
#[must_use]
pub fn dim(text: impl Into<SharedString>) -> Div {
    div().text_color(color(theme::DIM)).child(text.into())
}

/// Hint text.
#[must_use]
pub fn faint(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.))
        .text_color(color(theme::FAINT))
        .child(text.into())
}

/// What shows where a list is empty: what is missing and what to do.
#[must_use]
pub fn empty(what: impl Into<SharedString>, next: impl Into<SharedString>) -> Div {
    col()
        .items_center()
        .gap_1()
        .py_6()
        .child(dim(what))
        .child(faint(next))
}

/// A status's color.
#[must_use]
pub const fn status_color(status: Status) -> u32 {
    match status {
        Status::Working => theme::GREEN,
        Status::NeedsYou => theme::ORANGE,
        Status::Idle | Status::ProbablyIdle => theme::DIM,
        Status::Exited { .. } => theme::RED,
        Status::Offline => theme::FAINT,
    }
}

/// A status dot.
#[must_use]
pub fn dot(status: Status) -> Div {
    div()
        .size(px(8.))
        .flex_shrink_0()
        .rounded_full()
        .bg(color(status_color(status)))
}

/// A button that sends `action`.
#[must_use]
pub fn send(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
) -> Button {
    let ui = ui.clone();
    Button::new(id)
        .label(label)
        .small()
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

/// A quiet button that sends `action`.
#[must_use]
pub fn ghost(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
) -> Button {
    send(ui, id, label, action).ghost()
}

/// The main button of a form, sending `action`.
#[must_use]
pub fn primary(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
) -> Button {
    send(ui, id, label, action).primary()
}

/// One of a set of choices, selected when `on`.
#[must_use]
pub fn choice(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on: bool,
    action: AppAction,
) -> Button {
    let button = send(ui, id, label, action);
    if on { button.primary() } else { button.ghost() }
}

/// A banner saying what went wrong, with a button to dismiss it.
#[must_use]
pub fn error_banner(
    ui: &Rc<Ui>,
    id: &'static str,
    text: impl Into<SharedString>,
    dismiss: AppAction,
) -> impl IntoElement {
    let text = text.into();
    row()
        .id(id)
        .role(Role::Alert)
        .aria_label(text.clone())
        .gap_3()
        .px_3()
        .py_2()
        .rounded(px(6.))
        .bg(color(0x2a1717))
        .border_1()
        .border_color(color(theme::RED))
        .child(div().flex_1().text_color(color(theme::TEXT)).child(text))
        .child(ghost(ui, (id, 1usize), "Dismiss", dismiss))
        .test_support()
}

/// A clickable row in a list, with role `role` and `label`, highlighted
/// when `selected`. Finish it with `.test_support()` so queries see it.
pub fn list_item(
    id: impl Into<ElementId>,
    role: Role,
    label: impl Into<SharedString>,
    selected: bool,
) -> Stateful<Div> {
    row()
        .id(id)
        .role(role)
        .aria_label(label)
        .aria_selected(selected)
        .gap_2()
        .px_2()
        .rounded(px(5.))
        .cursor_pointer()
        .when(selected, |item| item.bg(color(theme::SELECTED)))
        .when(!selected, |item| {
            item.hover(|item| item.bg(color(theme::PANEL2)))
        })
}
