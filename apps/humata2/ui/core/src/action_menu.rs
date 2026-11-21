//! The action menu: a card of items over the content, with a clear layer behind
//! it that closes it on a click outside. Esc closes it through the overlay that
//! owns the menu's open flag, so the menu itself holds no state.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;

use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::col;

/// Duration of menu, popover, and overlay entrance animations in milliseconds.
pub const ENTRANCE_MS: u64 = 120;

/// Quintic ease-out curve (`1 - (1 - t)^5`) for crisp macOS entrance animations.
#[must_use]
pub fn ease_out_quint(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(5)
}

/// Wraps a menu card with a 120 ms `ease_out_quint` fade and slight drop-in.
/// Skipped in headless tests (`!chrome::native_traffic_lights()`) so single-frame
/// captures draw the card at full opacity instead of t = 0.
#[must_use]
pub fn entrance<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    card: E,
) -> AnyElement {
    if !crate::chrome::native_traffic_lights() {
        return card.into_any_element();
    }
    card.with_animation(
        id,
        Animation::new(Duration::from_millis(ENTRANCE_MS)).with_easing(ease_out_quint),
        |card, delta| card.opacity(delta).mt(px(-4. * (1. - delta))),
    )
    .into_any_element()
}

/// The card that holds a menu's items, in the frames' menu chrome. It is
/// absolute, so the caller places it; its items are its children.
/// Uses concentric corners (`rounded(9)` outer, `p(4)`, `rounded(5)` items).
#[must_use]
pub fn card(id: &'static str, width: f32, look: Look) -> Stateful<Div> {
    col()
        .id(id)
        .absolute()
        .occlude()
        .w(px(width))
        .p(px(4.))
        .gap(px(2.))
        .bg(rgb(look.card))
        .border(px(0.5))
        .border_color(look.hairline())
        .rounded(px(9.))
        .shadow(crate::theme::shadow_popover(look))
}

/// A horizontal hairline separator between groups of menu items.
#[must_use]
pub fn separator(look: Look) -> Div {
    div()
        .w_full()
        .h(px(0.5))
        .my(px(3.))
        .bg(look.hairline())
}

/// A clear layer covering the window from where its container sits. Pressing
/// the mouse on it sends `close`. Draw it before the card, so the card stays above it.
#[must_use]
pub fn scrim(ui: &Rc<Ui>, id: &'static str, close: AppAction) -> Stateful<Div> {
    let ui = ui.clone();
    div()
        .id(id)
        .absolute()
        .occlude()
        // Larger than any window, so the layer reaches every edge from the
        // container's corner.
        .top(px(-10_000.))
        .left(px(-10_000.))
        .w(px(20_000.))
        .h(px(20_000.))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            ui.send(close.clone(), cx);
        })
}

/// One item of a menu card, as the frames draw menu rows: its words at the
/// left, no border, and the hover fill across the card's width. A button
/// underneath, so it takes keyboard focus and shows the kit's focus ring.
#[derive(IntoElement)]
pub struct Item {
    ui: Rc<Ui>,
    id: ElementId,
    label: SharedString,
    action: AppAction,
    look: Look,
    destructive: bool,
}

impl Item {
    /// Styles this menu item as a destructive red action.
    #[must_use]
    pub const fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

/// A menu row that sends `action`. The click stops here, so the scrim
/// behind the card doesn't also take it.
#[must_use]
pub fn item(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
) -> Item {
    Item {
        ui: ui.clone(),
        id: id.into(),
        label: label.into(),
        action,
        look,
        destructive: false,
    }
}

/// A destructive (red) menu row that sends `action`.
#[must_use]
pub fn item_destructive(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
) -> Item {
    item(ui, id, label, action, look).destructive()
}

impl RenderOnce for Item {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let fg: Hsla = if self.destructive {
            rgb(crate::theme::RED_STATUS).into()
        } else {
            rgb(self.look.text).into()
        };
        let fill: Hsla = if self.destructive {
            rgb(self.look.del).into()
        } else {
            rgb(self.look.fill).into()
        };
        let (ui, action) = (self.ui, self.action);
        Button::new(self.id)
            .custom(
                ButtonCustomVariant::new(cx)
                    .foreground(fg)
                    .hover(fill)
                    .active(fill),
            )
            .accessibility_label(self.label.clone())
            .small()
            .w_full()
            .h(px(24.))
            .px(px(8.))
            .rounded(px(5.))
            .cursor_pointer()
            // The kit centres a label; a full-width child keeps the words at
            // the left, as a menu's are.
            .child(div().w_full().text_size(px(12.)).child(self.label))
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                ui.send(action.clone(), cx);
            })
    }
}

#[cfg(test)]
mod tests {
    use ::core::prelude::v1::test;

    use super::ease_out_quint;

    #[test]
    fn ease_out_quint_endpoints_and_monotonicity() {
        assert_eq!(ease_out_quint(-0.5), 0.0);
        assert_eq!(ease_out_quint(0.0), 0.0);
        assert_eq!(ease_out_quint(1.0), 1.0);
        assert_eq!(ease_out_quint(1.5), 1.0);
        assert!(ease_out_quint(0.25) > 0.7);
        assert!(ease_out_quint(0.5) > ease_out_quint(0.25));
    }
}
