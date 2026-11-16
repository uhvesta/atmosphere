//! Small building blocks every screen uses: rows and columns, text styles,
//! buttons, pills, status dots and empty states. Each control that does
//! something carries a role and a label, so a person with a screen reader,
//! a flow test and a recording all find it by what it says.

use std::rc::Rc;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;

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

/// A push button as the frames draw it: 24 pt high, radius 6, with a 12 pt
/// medium label. The kit draws a `.label()` at its own smaller size, so the
/// label is a child and the button carries it as its accessibility label.
#[must_use]
pub fn push(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    let label = label.into();
    Button::new(id)
        .accessibility_label(label.clone())
        .small()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .child(div().text_size(px(12.)).child(label))
}

/// The frames' grey push button with a hairline. It has no click handler yet.
#[must_use]
pub(crate) fn push_quiet_shape(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    look: theme::Look,
) -> Button {
    push(id, label)
        .bg(rgb(look.fill))
        .border_color(rgb(look.line))
        .text_color(rgb(look.text))
}

/// The frames' filled push button. It has no click handler yet.
#[must_use]
pub(crate) fn push_primary_shape(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    look: theme::Look,
) -> Button {
    push(id, label)
        .primary()
        .bg(rgb(look.accent))
        .text_color(rgb(0xffffff))
}

/// The frames' grey push button, sending `action`. The click stops here, so
/// it can sit in a row that is itself clickable.
#[must_use]
pub(crate) fn push_quiet(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: theme::Look,
) -> Button {
    let ui = ui.clone();
    push_quiet_shape(id, label, look).on_click(move |_, _, cx| {
        cx.stop_propagation();
        ui.send(action.clone(), cx);
    })
}

/// The frames' filled push button, sending `action`. The click stops here.
#[must_use]
pub(crate) fn push_primary(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: theme::Look,
) -> Button {
    let ui = ui.clone();
    push_primary_shape(id, label, look).on_click(move |_, _, cx| {
        cx.stop_propagation();
        ui.send(action.clone(), cx);
    })
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
        .flex_shrink_0()
        .items_start()
        .gap_3()
        .px_3()
        .py_2()
        .rounded(px(6.))
        .bg(color(0x2a1717))
        .border_1()
        .border_color(color(theme::RED))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(color(theme::TEXT))
                .child(text),
        )
        .child(
            div()
                .flex_shrink_0()
                .child(ghost(ui, (id, 1usize), "Dismiss", dismiss).text_color(color(theme::TEXT))),
        )
        .test_support()
}
