//! A drag through real input: the item is picked up, the drop zone that only
//! exists while something is dragged is found after that, and the drop lands.
use gpui_kit::base::TestSupportExt as _;
use gpui_kit::gpui;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyWindowHandle, Context, Entity, Role, TestAppContext, Window, WindowOptions, div, px,
};
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, GpuiDriverError, Target};

/// What is being dragged.
#[derive(Clone)]
struct Held;

impl Render for Held {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().child("held")
    }
}

struct Board {
    dropped: usize,
}

impl Render for Board {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dragging = cx.has_active_drag();
        div()
            .size_full()
            .child(
                div()
                    .id("item")
                    .test_support()
                    .role(Role::Tab)
                    .aria_label("Item")
                    .w(px(120.))
                    .h(px(30.))
                    .on_drag(Held, |held, _, _, cx| cx.new(|_| held.clone())),
            )
            .when(dragging, |board| {
                board.child(
                    div()
                        .id("zone")
                        .test_support()
                        .role(Role::Region)
                        .aria_label("Zone")
                        .absolute()
                        .right(px(0.))
                        .bottom(px(0.))
                        .w(px(100.))
                        .h(px(100.))
                        .on_drop(cx.listener(|this, _: &Held, _, cx| {
                            this.dropped += 1;
                            cx.notify();
                        })),
                )
            })
    }
}

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Board>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                show: false,
                focus: false,
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| Board { dropped: 0 }),
        )
    })
    .expect("headless window opens")
}

/// A drag drops on a zone that appears only once the drag has begun.
#[gpui_kit::test]
fn a_drag_drops_on_a_zone_shown_only_while_dragging(cx: &mut TestAppContext) {
    let (window, board) = open(cx);
    let mut driver = GpuiDriver::with_frames(cx, window, FrameMode::Natural);
    driver
        .perform(&Action::Drag {
            from: Target::new(Role::Tab, "Item"),
            to: Target::new(Role::Region, "Zone"),
        })
        .expect("the drag finds the zone once it has started");
    drop(driver);
    assert_eq!(board.read_with(cx, |board, _| board.dropped), 1);
}

/// A drop target that never appears is an error, and the drag is let go.
#[gpui_kit::test]
fn a_drag_to_nothing_fails_and_leaves_no_drag_open(cx: &mut TestAppContext) {
    let (window, board) = open(cx);
    let mut driver = GpuiDriver::with_frames(cx, window, FrameMode::Natural);
    let error = driver
        .perform(&Action::Drag {
            from: Target::new(Role::Tab, "Item"),
            to: Target::new(Role::Region, "Elsewhere"),
        })
        .expect_err("there is no such zone");
    drop(driver);
    assert!(matches!(error, GpuiDriverError::NotFound { .. }));
    assert_eq!(board.read_with(cx, |board, _| board.dropped), 0);
    assert!(!cx.update(|cx| cx.has_active_drag()));
}
