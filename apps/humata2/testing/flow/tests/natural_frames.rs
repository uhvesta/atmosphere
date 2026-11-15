//! The natural-frame driver catches what a full refresh hides: a view that
//! changes without `cx.notify()`. Runs headless; no window is shown.

use gpui_kit::base::TestTextExt as _;
use gpui_kit::prelude::*;
use gpui_kit::*;
// `gpui_kit::*` exports a `test` attribute; keep `#[test]` the standard one.
use ::std::prelude::v1::test;
use humata2_engine::Engine;
use humata2_flow::{Action, Driver as _, FrameMode, GpuiDriver, GpuiDriverError, Target};

/// A label another view changes later, from a task.
struct Status {
    text: &'static str,
}

impl Render for Status {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("status")
            .test_support()
            .role(Role::Status)
            .aria_label(self.text)
            .child(div().child(self.text).test_text(self.text))
    }
}

/// A button that starts a task; the task sets the status when it runs,
/// notifying it unless `notifies` is false.
struct Job {
    status: Entity<Status>,
    notifies: bool,
}

impl Render for Job {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .id("finish")
                    .test_support()
                    .role(Role::Button)
                    .aria_label("Finish")
                    .w(px(120.))
                    .h(px(24.))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        let (status, notifies) = (this.status.clone(), this.notifies);
                        cx.spawn(async move |_, cx| {
                            status.update(cx, |status, cx| {
                                status.text = "done";
                                if notifies {
                                    cx.notify();
                                }
                            });
                        })
                        .detach();
                    }))
                    .child("Finish"),
            )
            .child(self.status.clone())
    }
}

fn open(cx: &mut TestAppContext, notifies: bool) -> AnyWindowHandle {
    cx.update(gpui_kit::init);
    let (window, _) = cx
        .update(|cx| {
            let options = WindowOptions {
                show: false,
                focus: false,
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |_, cx| {
                cx.new(|cx| Job {
                    status: cx.new(|_| Status { text: "waiting" }),
                    notifies,
                })
            })
        })
        .expect("the test window opens");
    cx.run_until_parked();
    window
}

fn finish() -> Action {
    Action::Click(Target::new(Role::Button, "Finish"))
}

#[gpui_kit::test]
fn a_change_that_notifies_shows_in_natural_frames(cx: &mut TestAppContext) {
    let window = open(cx, true);
    let mut driver = GpuiDriver::with_frames(cx, window, FrameMode::Natural);
    driver.perform(&finish()).expect("the click lands");
    let view = driver.view().expect("the natural frame matches a refreshed one");
    assert!(view.contains("Status \"done\""), "{view}");
}

#[gpui_kit::test]
fn a_change_without_notify_fails_where_views_are_retained(cx: &mut TestAppContext) {
    let window = open(cx, false);
    let mut driver = GpuiDriver::with_frames(cx, window, FrameMode::Natural);
    driver.perform(&finish()).expect("the click lands");
    let view = driver.view();
    if Engine::CURRENT.retains_views() {
        assert!(
            matches!(view, Err(GpuiDriverError::Stale { .. })),
            "GPUI Fast left the status stale, and the driver should say so: {view:?}"
        );
    } else {
        // Ordinary GPUI builds every uncached view each frame it draws, so
        // the status is up to date and only the GPUI Fast run catches this.
        let view = view.expect("ordinary GPUI draws the change");
        assert!(view.contains("Status \"done\""), "{view}");
    }
}

#[gpui_kit::test]
fn a_full_refresh_hides_the_missing_notify(cx: &mut TestAppContext) {
    let window = open(cx, false);
    let mut driver = GpuiDriver::new(cx, window);
    driver.perform(&finish()).expect("the click lands");
    let view = driver.view().expect("a refreshed frame always draws");
    assert!(view.contains("Status \"done\""), "{view}");
}
