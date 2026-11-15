//! Proves the snapshot and flow tools on a throwaway view: a counter with a
//! list. One state snapshot, one view snapshot, one layout snapshot and one
//! flow, plus an image and a platform-font layout snapshot on macOS.
//! Everything runs headless; no window is shown.
//!
//! The first run has nothing recorded, so run it once with
//! `HUMATA2_SNAPSHOT=record`, read the files it writes in `__snapshots__/`,
//! and commit them.

use gpui_kit::component::Disableable as _;
use gpui_kit::component::button::Button;
use gpui_kit::prelude::*;
use gpui_kit::*;
use gpui_kit::base::TestTextExt;
use gpui_kit::test::TestWindowExt;
// `gpui_kit::*` exports a `test` attribute; keep `#[test]` the standard one.
use ::std::prelude::v1::test;
use humata2_flow::{Flow, FrameMode, GpuiDriver, SpecFlow, artifact_root, run};
use humata2_snapshot::{StateRecorder, layout, snapshots, view};
use tca::{Effect, LocalStore, Reducer, TestStore};
use tokio::runtime::{Builder, Runtime};

#[derive(Clone, Debug, Default, PartialEq)]
struct Tally {
    count: i64,
    draft: String,
    items: Vec<String>,
    selected: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
enum TallyAction {
    Increment,
    Decrement,
    Typed(String),
    Erase,
    Add,
    Select(usize),
    Clear,
}

struct TallyReducer;

impl Reducer for TallyReducer {
    type State = Tally;
    type Action = TallyAction;

    fn reduce(&self, state: &mut Tally, action: TallyAction) -> Effect<TallyAction> {
        match action {
            TallyAction::Increment => state.count += 1,
            TallyAction::Decrement => state.count -= 1,
            TallyAction::Typed(text) => state.draft.push_str(&text),
            TallyAction::Erase => {
                state.draft.pop();
            }
            TallyAction::Add => {
                let item = state.draft.trim().to_owned();
                if !item.is_empty() {
                    state.items.push(item);
                    state.draft.clear();
                }
            }
            TallyAction::Select(index) => {
                if index < state.items.len() {
                    state.selected = Some(index);
                }
            }
            TallyAction::Clear => {
                state.items.clear();
                state.selected = None;
            }
        }
        Effect::none()
    }
}

struct TallyView {
    // Before the runtime, so it is dropped first.
    store: LocalStore<TallyReducer>,
    _runtime: Runtime,
    draft_focus: FocusHandle,
}

impl TallyView {
    fn new(state: Tally, cx: &mut Context<Self>) -> Self {
        let runtime = Builder::new_current_thread()
            .build()
            .expect("a current-thread runtime starts");
        let store = LocalStore::on(runtime.handle(), state, TallyReducer);
        Self {
            store,
            _runtime: runtime,
            draft_focus: cx.focus_handle(),
        }
    }

    fn send(&mut self, action: TallyAction, cx: &mut Context<Self>) {
        self.store.send(action);
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.secondary(cx) && keystroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.send(TallyAction::Typed(text), cx);
            }
        } else if keystroke.key == "backspace" {
            self.send(TallyAction::Erase, cx);
        } else if keystroke.key == "enter" {
            self.send(TallyAction::Add, cx);
        } else if !modifiers.control && !modifiers.platform {
            if let Some(text) = keystroke.key_char.clone() {
                self.send(TallyAction::Typed(text), cx);
            }
        }
    }
}

impl Render for TallyView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.with_state(Tally::clone);
        let count = state.count.to_string();
        let draft = state.draft.clone();
        let rows = state.items.iter().enumerate().map(|(index, item)| {
            div()
                .id(("item", index))
                .test_support()
                .role(Role::ListItem)
                .aria_label(item.clone())
                .aria_selected(state.selected == Some(index))
                .px_2()
                .when(state.selected == Some(index), |row| row.bg(rgb(0x2f4a72)))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.send(TallyAction::Select(index), cx);
                }))
                .child(div().child(item.clone()).test_text(item.clone()))
        });

        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .child(div().child("Shopping").test_text("Shopping"))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(Button::new("decrement").label("Decrement").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| this.send(TallyAction::Decrement, cx),
                    )))
                    .child(
                        div()
                            .id("count")
                            .test_support()
                            .role(Role::Status)
                            .aria_label("Count")
                            .child(div().child(count.clone()).test_text(count)),
                    )
                    .child(Button::new("increment").label("Increment").on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| this.send(TallyAction::Increment, cx),
                    ))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .child(
                        div()
                            .id("draft")
                            .test_support()
                            .role(Role::TextInput)
                            .aria_label("New item")
                            .aria_value(draft.clone())
                            .track_focus(&self.draft_focus)
                            .w(px(200.))
                            .h(px(24.))
                            .border_1()
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                window.focus(&this.draft_focus, cx);
                            }))
                            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                                this.key_down(event, cx);
                            }))
                            .child(div().child(draft.clone()).test_text(draft)),
                    )
                    .child(
                        Button::new("add")
                            .label("Add")
                            .disabled(state.draft.trim().is_empty())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.send(TallyAction::Add, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("items")
                    .test_support()
                    .role(Role::List)
                    .aria_label("Items")
                    .flex()
                    .flex_col()
                    .children(rows),
            )
            .child(
                Button::new("clear")
                    .label("Clear")
                    .disabled(state.items.is_empty())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.send(TallyAction::Clear, cx);
                    })),
            )
    }
}

fn options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(360.), px(320.)),
        })),
        focus: false,
        show: false,
        ..Default::default()
    }
}

/// Opens the view in a test window, which is never shown.
fn open(cx: &mut TestAppContext, state: Tally) -> AnyWindowHandle {
    cx.update(gpui_kit::init);
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options(), cx, |_, cx| cx.new(|cx| TallyView::new(state, cx)))
        })
        .expect("the test window opens");
    cx.run_until_parked();
    window
}

fn with_items() -> Tally {
    Tally {
        count: 2,
        draft: "tea".to_owned(),
        items: vec!["milk".to_owned(), "eggs".to_owned()],
        selected: Some(1),
    }
}

#[test]
fn adding_an_item_moves_the_draft_into_the_list() {
    let mut recorder = StateRecorder::new(TestStore::new(Tally::default(), TallyReducer));
    recorder.send(TallyAction::Typed("milk".to_owned()), |state| {
        state.draft = "milk".to_owned();
    });
    recorder.send(TallyAction::Add, |state| {
        state.draft.clear();
        state.items.push("milk".to_owned());
    });
    recorder.send(TallyAction::Add, |_| {});
    recorder.send(TallyAction::Increment, |state| state.count = 1);
    recorder.send(TallyAction::Select(0), |state| state.selected = Some(0));

    snapshots!()
        .expect("snapshots are kept beside this file")
        .assert_text("state-add-item", &recorder.finish());
}

#[gpui_kit::test]
fn the_view_shows_the_count_the_items_and_which_is_selected(cx: &mut TestAppContext) {
    let window = open(cx, with_items());
    let tree = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            view::capture(window)
        })
        .expect("the window is open")
        .expect("the view can be read");

    snapshots!()
        .expect("snapshots are kept beside this file")
        .assert_text("view-with-items", &tree);
}

#[gpui_kit::test]
fn the_view_lays_out_the_rows_with_the_selected_one_highlighted(cx: &mut TestAppContext) {
    let window = open(cx, with_items());
    let layout = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            layout::capture(window)
        })
        .expect("the window is open");

    // Test fonts measure the same everywhere: one file, view-with-items.layout.txt.
    snapshots!()
        .expect("snapshots are kept beside this file")
        .assert_layout("view-with-items", &layout);
}

/// Not a SPEC.md flow: the demo proves the runner itself.
const COUNT_AND_LIST: SpecFlow = SpecFlow::new("demo", "Count and list items");

fn count_and_list_flow() -> Flow {
    Flow::new(COUNT_AND_LIST)
        .snapshot()
        .click(Role::Button, "Increment")
        .click(Role::Button, "Increment")
        .click(Role::Button, "Decrement")
        .snapshot()
        .click(Role::TextInput, "New item")
        .type_text("milk")
        .snapshot()
        .click(Role::Button, "Add")
        .snapshot()
        .click(Role::TextInput, "New item")
        .paste("eggs")
        .key("enter")
        .snapshot()
        .click(Role::ListItem, "milk")
        .snapshot_layout()
        .click(Role::Button, "Clear")
        .snapshot()
}

#[gpui_kit::test]
fn count_and_list_items(cx: &mut TestAppContext) {
    let window = open(cx, Tally::default());
    let snapshots = snapshots!().expect("snapshots are kept beside this file");
    let out = artifact_root(&target_tmp());
    let mut driver = GpuiDriver::new(cx, window);
    match run(&count_and_list_flow(), &mut driver, &snapshots, &out) {
        Ok(report) => println!("flow artifact: {}", report.artifact().display()),
        Err(error) => panic!("{error}"),
    }
}

/// The same flow drawn only as the app draws, checked against the same
/// snapshots: what natural frames leave on screen is what a full refresh
/// shows.
#[gpui_kit::test]
fn count_and_list_items_in_natural_frames(cx: &mut TestAppContext) {
    let window = open(cx, Tally::default());
    let snapshots = snapshots!().expect("snapshots are kept beside this file");
    let out = artifact_root(&target_tmp()).join("natural-frames");
    let mut driver = GpuiDriver::with_frames(cx, window, FrameMode::Natural);
    match run(&count_and_list_flow(), &mut driver, &snapshots, &out) {
        Ok(report) => println!("flow artifact: {}", report.artifact().display()),
        Err(error) => panic!("{error}"),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_view_draws_as_recorded() {
    let mut cx = humata2_snapshot::headless_context();
    let (window, _) = cx
        .update(|cx| {
            gpui_kit::open_window(options(), cx, |_, cx| cx.new(|cx| TallyView::new(with_items(), cx)))
        })
        .expect("the headless window opens");
    cx.run_until_parked();
    let (image, layout) = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            (window.render_to_image(), layout::capture(window))
        })
        .expect("the window is open");
    let image = image.expect("Metal draws the window off screen");

    let snapshots = snapshots!().expect("snapshots are kept beside this file");
    snapshots.assert_image("view-with-items", &image, humata2_snapshot::Tolerance::default());
    // Measured with macOS's fonts, so kept as view-with-items.macos.layout.txt.
    snapshots.assert_layout("view-with-items", &layout);
}

/// Cargo's scratch directory for this test, or the system's under Bazel,
/// which sends artifacts to its test outputs instead (see `artifact_root`).
fn target_tmp() -> std::path::PathBuf {
    option_env!("CARGO_TARGET_TMPDIR").map_or_else(std::env::temp_dir, std::path::PathBuf::from)
}
