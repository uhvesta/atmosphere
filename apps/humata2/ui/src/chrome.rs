//! Small native window controls shared by the D76 shell and pages.
use crate::bridge::Ui;
use crate::theme::Look;
use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::Status;
use std::rc::Rc;

pub(crate) fn icon(name: IconName, color: u32, size: f32) -> Icon {
    Icon::new(name).size(px(size)).text_color(rgb(color))
}
pub(crate) fn action(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    glyph: IconName,
    event: AppAction,
    color: u32,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .size(px(16.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(icon(glyph, color, 15.))
        .on_click(move |_, _, cx| ui.send(event.clone(), cx))
        .test_support()
}
pub(crate) fn status(value: Status) -> Div {
    let dot = div().size(px(8.)).rounded_full().flex_shrink_0();
    let dot = match value {
        Status::Working => dot.bg(rgb(0x30d158)),
        Status::NeedsYou => dot.bg(rgb(0xff9f0a)),
        Status::Idle | Status::ProbablyIdle => dot.border_1().border_color(rgb(0x8e8e93)),
        Status::Offline => dot.bg(rgb(0x8e8e93)),
        Status::Exited { .. } => {
            return div()
                .size(px(10.))
                .flex_shrink_0()
                .child(icon(IconName::X, 0xff453a, 10.));
        }
    };
    div()
        .size(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .child(dot)
}
pub(crate) fn lights() -> Div {
    div().flex().gap(px(8.)).flex_shrink_0().children(
        [
            ("window-close", "Close window", 0xff5f57),
            ("window-minimize", "Minimize window", 0xfebc2e),
            ("window-zoom", "Zoom window", 0x28c840),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (id, label, color))| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .size(px(12.))
                .rounded_full()
                .bg(rgb(color))
                .on_click(move |_, window, _| match index {
                    0 => window.remove_window(),
                    1 => window.minimize_window(),
                    _ => window.zoom_window(),
                })
                .test_support()
        }),
    )
}
pub(crate) fn section(look: Look, title: &'static str) -> Div {
    div()
        .flex()
        .items_center()
        .px(px(18.))
        .pt(px(12.))
        .pb(px(3.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.secondary))
        .child(div().flex_1().child(title))
}
