mod support;
use gpui_kit::TestAppContext;
use gpui_kit::gpui;
use gpui_kit::test::TestWindowExt;
use support::open;

#[gpui_kit::test]
fn in_band_resize_is_delivered_without_waiting_for_more_output(cx: &mut TestAppContext) {
    let pane = open(cx, 100);
    pane.feed(cx, b"\x1b[?2048h");
    pane.sent();
    pane.frame(cx, |window, cx| {
        window.resize(gpui_kit::size(gpui_kit::px(600.), gpui_kit::px(350.)));
        window.bounds_changed(cx);
    });
    pane.frame(cx, |_, _| {});
    let sent = String::from_utf8(pane.sent()).unwrap();
    assert!(
        sent.contains("\x1b[48;"),
        "resize must immediately deliver CSI 48 for applications that requested DEC 2048: {sent:?}"
    );
}
