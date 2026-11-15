//! The screen's panes (NV-28–NV-35): the layout state's tree drawn as
//! nested rows and columns, each pane with a header that focuses or closes it.
//! The layout state stays the source of truth; this only draws it.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_layout::{Axis, LayoutAction, Pane, PaneId, PaneKind, Tree};

use crate::bridge::Ui;
use crate::pages;
use crate::theme::{self, color};
use crate::widgets::{self, col, row};

/// The current screen's panes.
pub fn panes(ui: &Rc<Ui>, state: &AppState) -> AnyElement {
    let tree = state.panes.tree();
    let focus = state.panes.focus();
    node(ui, state, &tree, focus)
}

fn node(ui: &Rc<Ui>, state: &AppState, tree: &Tree, focus: Option<PaneId>) -> AnyElement {
    match tree {
        Tree::Pane(pane) => pane_view(ui, state, pane, focus == Some(pane.id)).into_any_element(),
        Tree::Split(split) => {
            let base = match split.axis {
                Axis::Horizontal => row(),
                Axis::Vertical => col(),
            };
            base.size_full()
                .min_w_0()
                .min_h_0()
                .gap_px()
                .bg(color(theme::LINE))
                .children(split.children.iter().zip(&split.sizes).map(|(child, share)| {
                    let cell = div().flex().flex_col().min_w_0().min_h_0().overflow_hidden();
                    let cell = match split.axis {
                        Axis::Horizontal => cell.h_full().w(relative(*share)),
                        Axis::Vertical => cell.w_full().h(relative(*share)),
                    };
                    cell.child(node(ui, state, child, focus))
                }))
                .into_any_element()
        }
    }
}

fn pane_view(ui: &Rc<Ui>, state: &AppState, pane: &Pane, focused: bool) -> impl IntoElement {
    let id = pane.id;
    let index = usize::try_from(id.0).unwrap_or(usize::MAX);
    let title = pane.kind.title(&state.workspace);
    col()
        .id(("pane", index))
        .role(Role::Group)
        .aria_label(title.clone())
        .size_full()
        .min_w_0()
        .min_h_0()
        .bg(color(theme::BG))
        .border_1()
        .border_color(color(if focused { theme::ACCENT } else { theme::BG }))
        .on_mouse_down(MouseButton::Left, {
            let ui = ui.clone();
            move |_, _, cx| ui.send(AppAction::Panes(LayoutAction::Focus(id)), cx)
        })
        .child(
            row()
                .h(px(28.))
                .flex_shrink_0()
                .px_2()
                .gap_2()
                .border_b_1()
                .border_color(color(theme::LINE))
                .child(div().flex_1().min_w_0().text_size(px(12.)).child(title.clone()))
                .child(
                    Button::new(("pane-close", index))
                        .icon(IconName::Close)
                        .ghost()
                        .xsmall()
                        .accessibility_label(format!("Close {title}"))
                        .on_click({
                            let ui = ui.clone();
                            move |_, _, cx| ui.send(AppAction::Panes(LayoutAction::Close(id)), cx)
                        }),
                ),
        )
        .child(div().flex_1().min_h_0().flex().flex_col().child(content(ui, state, &pane.kind)))
        .test_support()
}

fn content(ui: &Rc<Ui>, state: &AppState, kind: &PaneKind) -> AnyElement {
    match kind {
        PaneKind::Home => pages::home(ui, state).into_any_element(),
        PaneKind::Agent(id) => state.workspace.session(id).map_or_else(
            || widgets::empty("This session is gone.", "Close this pane or pick another session.").into_any_element(),
            |session| pages::terminal(session).into_any_element(),
        ),
        _ => widgets::empty("Nothing here yet.", "This pane is being built.").into_any_element(),
    }
}
