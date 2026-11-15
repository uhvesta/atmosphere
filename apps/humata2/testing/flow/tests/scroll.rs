//! Scrolling and platform shortcuts through real input on both engines.
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::gpui;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, Context, Entity, FocusHandle, KeyDownEvent, Role, ScrollHandle,
    TestAppContext, Window, WindowOptions, div, px,
};
use humata2_flow::{Action, Driver as _, Flow, FrameMode, GpuiDriver, SpecFlow, Target};

struct Pane {
    focus: FocusHandle,
    scroll: ScrollHandle,
    shortcut: bool,
}

impl Render for Pane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(
            div()
                .id("scroll-area")
                .track_focus(&self.focus)
                .test_support()
                .role(Role::Region)
                .aria_label("Scrollable content")
                .w(px(300.))
                .h(px(160.))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.modifiers.secondary(cx) && event.keystroke.key == "k" {
                        this.shortcut = true;
                    }
                }))
                .children((0..100).map(|i| div().h(px(24.)).flex_none().child(format!("Row {i}")))),
        )
    }
}

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Pane>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                show: false,
                focus: false,
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    Pane {
                        focus,
                        scroll: ScrollHandle::new(),
                        shortcut: false,
                    }
                })
            },
        )
    })
    .expect("headless window opens")
}

/// I4: scroll deltas reach the target with the same sign in both frame modes.
#[gpui_kit::test]
fn a_flow_scrolls_content_in_both_directions(cx: &mut TestAppContext) {
    for frames in [FrameMode::FullRefresh, FrameMode::Natural] {
        let (window, pane) = open(cx);
        let target = Target::new(Role::Region, "Scrollable content");
        let flow = Flow::new(SpecFlow::new("I4", "Scroll content"))
            .scroll(target.clone(), 0, -100)
            .scroll(target, 0, 100);
        let mut driver = GpuiDriver::with_frames(cx, window, frames);
        driver
            .perform(flow.steps()[0].action())
            .expect("scroll down");
        drop(driver);
        assert!(pane.read_with(cx, |pane, _| pane.scroll.offset().y) < px(0.));
        let mut driver = GpuiDriver::with_frames(cx, window, frames);
        driver.perform(flow.steps()[1].action()).expect("scroll up");
        drop(driver);
        assert_eq!(pane.read_with(cx, |pane, _| pane.scroll.offset().y), px(0.));
    }
}

/// I4: secondary shortcuts parse for the actual platform in both frame modes.
#[gpui_kit::test]
fn secondary_shortcuts_reach_the_platform_modifier(cx: &mut TestAppContext) {
    for frames in [FrameMode::FullRefresh, FrameMode::Natural] {
        let (window, pane) = open(cx);
        // A key listener on the root receives unfocused window key events.
        let mut driver = GpuiDriver::with_frames(cx, window, frames);
        driver
            .perform(&Action::Key("secondary-k".into()))
            .expect("secondary key parses");
        drop(driver);
        assert!(pane.read_with(cx, |pane, _| pane.shortcut));
    }
}
