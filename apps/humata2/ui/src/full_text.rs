//! One shared way to keep the full text of a shortened label (DS-80).
//!
//! Any text drawn with an ellipsis (a file path, repo, branch, base chip, PR
//! line, a title-bar name) must still show everything on hover and carry it
//! as its accessibility label, so a reader or a hover never sees only "src/…".

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::{prelude::*, *};

/// Gives a shortened element its full `text` as a hover tooltip and as its
/// accessibility label.
///
/// The element needs an id (`.id(..)`) first, as every stateful element does:
///
/// ```ignore
/// full_text(
///     div().id("changes-file-path").min_w_0().overflow_hidden()
///         .whitespace_nowrap().text_ellipsis().child(path.clone()),
///     path,
/// )
/// ```
///
/// Pass the same string the element draws, unshortened. Do not use it on an
/// element that already sets its own tooltip or label: the later call wins.
///
/// An element is only reported to assistive technology with a role, so this
/// gives it `Role::Label`. For a clickable element (a button, a link) call
/// `.role(..)` after `full_text(..)` so its own role wins.
pub(crate) fn full_text<E: StatefulInteractiveElement>(
    element: E,
    text: impl Into<SharedString>,
) -> E {
    let text: SharedString = text.into();
    let shown = text.clone();
    element
        .role(Role::Label)
        .aria_label(text)
        .tooltip(move |window, cx| Tooltip::new(shown.clone()).build(window, cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    // The gpui glob above brings in its own `test` attribute; the built-in
    // one wins over a glob by name.
    use ::core::prelude::v1::test;
    use gpui_kit::test::{TestQueryExt as _, TestWindowExt as _};

    const PATH: &str = "apps/humata2/ui/src/panel_changes.rs";

    struct View;

    impl Render for View {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(full_text(
                div()
                    .id("path")
                    .w(px(40.))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(PATH),
                PATH,
            )
            // Observed, so the headless window can read its label.
            .test_support())
        }
    }

    #[gpui_kit::test]
    fn a_shortened_element_carries_its_full_text(cx: &mut TestAppContext) {
        let (window, _) = cx
            .update(|cx| {
                gpui_kit::init(cx);
                gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| cx.new(|_| View))
            })
            .expect("the test window opens");
        let found = cx
            .update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find_by_label(PATH).is_some()
            })
            .expect("the test window is open");
        assert!(found);
    }
}
