mod support;

use gpui_kit as gpui;
use gpui_kit::{
    Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, PlatformInput, TestAppContext,
};

use support::{Told, open};

fn click_link_modifiers() -> Modifiers {
    Modifiers {
        platform: cfg!(target_os = "macos"),
        control: !cfg!(target_os = "macos"),
        ..Default::default()
    }
}

#[gpui_kit::test]
fn cmd_or_ctrl_clicking_osc8_and_plain_file_links_emits_open_link(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(
        cx,
        b"\x1b]8;;file:///session/SPEC.md:10\x1b\\SPEC.md:10\x1b]8;;\x1b\\\r\nfeatures/app/src/lib.rs:1856-1866\r\n",
    );
    pane.told();

    let modifiers = click_link_modifiers();
    pane.frame(cx, |window, cx| {
        let osc8_pos = pane.cell(window, 2, 0, cx);
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                position: osc8_pos,
                button: MouseButton::Left,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                position: osc8_pos,
                button: MouseButton::Left,
                modifiers,
                click_count: 1,
            }),
            cx,
        );

        let plain_pos = pane.cell(window, 5, 1, cx);
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                position: plain_pos,
                button: MouseButton::Left,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                position: plain_pos,
                button: MouseButton::Left,
                modifiers,
                click_count: 1,
            }),
            cx,
        );
    });

    assert_eq!(
        pane.told(),
        vec![
            Told::Link("file:///session/SPEC.md:10".to_owned()),
            Told::Link("features/app/src/lib.rs:1856-1866".to_owned()),
        ]
    );
}
