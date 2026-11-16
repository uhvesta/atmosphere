mod support;
use gpui_kit::gpui;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{Modifiers, ModifiersChangedEvent, PlatformInput, TestAppContext};
use support::{Told, open};
#[gpui_kit::test]
fn modifier_click_opens_explicit_and_plain_links_without_mouse_reports(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx,b"\x1b]8;;https://example.com/hidden\x1b\\label\x1b]8;;\x1b\\\r\nhttps://example.com/plain\r\n\x1b[?1000h\x1b[?1006h");
    for (row, uri) in [
        (0, "https://example.com/hidden"),
        (1, "https://example.com/plain"),
    ] {
        pane.frame(cx, |window, cx| {
            let mut modifiers = Modifiers::default();
            if cfg!(target_os = "macos") {
                modifiers.platform = true
            } else {
                modifiers.control = true;
            }
            window.dispatch_event(
                PlatformInput::ModifiersChanged(ModifiersChangedEvent {
                    modifiers,
                    capslock: Default::default(),
                }),
                cx,
            );
            let position = pane.cell(window, 2, row, cx);
            window.drag(position, position, cx);
        });
        assert_eq!(pane.told(), vec![Told::Link(uri.into())]);
        assert!(
            pane.sent().is_empty(),
            "opening a URL must not send a click into the TUI"
        );
    }
}
