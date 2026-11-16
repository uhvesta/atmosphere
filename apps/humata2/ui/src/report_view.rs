//! Bug reports (I9, D133): the dot that shows while recording, the sheet
//! where the person says what happened and pins comments, the shortcuts
//! that start, stop and snapshot, and the socket `humata report` talks to.
//!
//! While recording, this view also tells the recorder what the person
//! does: each click and scroll names the element under the pointer, each
//! key names what had focus, and the view and layout trees are kept as
//! frames are drawn.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::base::{Disableable as _, Selectable as _};
use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::{prelude::*, *};
use humata2_app::{
    AppAction, PinnedComment, Point as Mark, ReportAction, ReportForm, ReportKind, ReportRequest,
};
use humata2_report::remote::{self, Answer, Listener, Refusal, RefusalKind, Reply, Request, RemoteError};
use humata2_report::{Button as Pointer, Gesture};

use crate::bridge::{Part, Ui};
use crate::keys;
use crate::record::{self, SHEET_LABEL, STOP_LABEL};
use crate::theme::{self, Look};
use crate::widgets::{self, col, row};

#[path = "report_draw.rs"]
mod draw;

/// How often the trees are kept while recording, so changes nothing here
/// caused (an agent's output, a daemon's events) are in the report too.
const TICK: Duration = Duration::from_millis(250);

/// The height of a pixel-line when a scroll counts lines.
const LINE: f32 = 20.;

/// The bug-report view, drawn over everything.
pub struct ReportView {
    ui: Rc<Ui>,
    happened: Entity<TextareaState>,
    expected: Entity<TextareaState>,
    query: Entity<InputState>,
    comments: Vec<(Entity<InputState>, Subscription)>,
    shown: Option<PathBuf>,
    restore: Option<FocusHandle>,
    ticking: bool,
    /// The stroke being drawn, while the button is down.
    drag: Option<Vec<Mark>>,
    /// A palette request is waiting out its frames.
    acting: bool,
    _subscriptions: Vec<Subscription>,
}

impl ReportView {
    /// The view on `ui`.
    pub fn new(ui: Rc<Ui>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        ui.attach(Part::Report, cx.entity_id());
        let happened = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What went wrong, or what looks off")
                .rows(3)
        });
        let expected = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What you thought would happen")
                .rows(3)
        });
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Find a control to pin"));
        let mut subscriptions = Vec::new();
        for (input, change) in [
            (&happened, ReportAction::Happened as fn(String) -> ReportAction),
            (&expected, ReportAction::Expected),
        ] {
            subscriptions.push(cx.subscribe(
                input,
                move |this: &mut Self, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let text = input.read(cx).value().to_string();
                        this.ui.send(AppAction::Report(change(text)), cx);
                    }
                },
            ));
        }
        subscriptions.push(cx.subscribe(
            &query,
            |this: &mut Self, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    this.ui
                        .send(AppAction::Report(ReportAction::Query(text)), cx);
                }
            },
        ));
        // Keys: after the app handled each one, so a recording tells what
        // the person pressed and what had focus.
        let recorder = ui.recorder().clone();
        let undoing = ui.clone();
        subscriptions.push(cx.observe_keystrokes(move |_: &mut Self, event, window, cx| {
            let keystroke = &event.keystroke;
            if keystroke.key == "z"
                && keystroke.modifiers.platform
                && !keystroke.modifiers.shift
                && undoing.read(|state| state.report.form.as_ref().is_some_and(|form| form.drawing))
            {
                undoing.send(AppAction::Report(ReportAction::UndoStroke), cx);
            }
            if !recorder.is_recording() {
                return;
            }
            let elements = snapshots(window);
            let clipboard = cx.read_from_clipboard().and_then(|item| item.text());
            if let Some(gesture) = record::keystroke_gesture(
                &event.keystroke,
                record::focused(&elements),
                clipboard,
            ) {
                recorder.gesture(gesture);
            }
            capture(&recorder, window);
        }));
        Self {
            ui,
            happened,
            expected,
            query,
            comments: Vec::new(),
            shown: None,
            restore: None,
            ticking: false,
            drag: None,
            acting: false,
            _subscriptions: subscriptions,
        }
    }

    /// Keeps the trees while a recording runs.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.ticking {
            return;
        }
        self.ticking = true;
        let recorder = self.ui.recorder().clone();
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let keep = this.update_in(cx, |this, window, _| {
                    let recording = recorder.is_recording();
                    if recording {
                        capture(&recorder, window);
                    } else {
                        this.ticking = false;
                    }
                    recording
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
    }

    /// Opens or closes the sheet with the form: focus goes in when it
    /// opens and back where it was when it closes; the inputs follow the
    /// state.
    fn follow(&mut self, form: Option<&ReportForm>, window: &mut Window, cx: &mut Context<Self>) {
        let folder = form.map(|form| form.folder.clone());
        if folder != self.shown {
            if self.shown.is_none() {
                self.restore = window.focused(cx);
            }
            self.comments.clear();
            match form {
                Some(_) => window.focus(&self.happened.focus_handle(cx), cx),
                None => {
                    if let Some(focus) = self.restore.take() {
                        window.focus(&focus, cx);
                    }
                }
            }
            self.shown = folder;
        }
        let Some(form) = form else { return };
        sync_text(&self.happened, &form.happened, window, cx);
        sync_text(&self.expected, &form.expected, window, cx);
        sync_input(&self.query, &form.query, window, cx);
        while self.comments.len() < form.comments.len() {
            let input = cx.new(|cx| {
                InputState::new(window, cx).placeholder("What about this control?")
            });
            let subscription = cx.subscribe(
                &input,
                |this: &mut Self, input, event: &InputEvent, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    if let Some(at) = this.comments.iter().position(|(have, _)| *have == input) {
                        let text = input.read(cx).value().to_string();
                        this.ui.send(
                            AppAction::Report(ReportAction::CommentText(at, text)),
                            cx,
                        );
                    }
                },
            );
            self.comments.push((input, subscription));
        }
        self.comments.truncate(form.comments.len());
        for ((input, _), comment) in self.comments.iter().zip(&form.comments) {
            sync_input(input, &comment.text, window, cx);
        }
    }

    /// Runs what the palette asked for: marks it handled at once, then
    /// acts two frames later so the palette is not in what gets saved.
    fn request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.acting {
            return;
        }
        let Some(request) = self.ui.read(|state| state.report.request) else {
            return;
        };
        self.acting = true;
        self.ui
            .send(AppAction::Report(ReportAction::Handled), cx);
        let ui = self.ui.clone();
        let this = cx.entity();
        window.on_next_frame(move |window, _| {
            window.on_next_frame(move |window, cx| {
                run_request(&ui, request, window, cx);
                this.update(cx, |this, _| this.acting = false);
            });
        });
    }

    fn dot(&self) -> impl IntoElement {
        let ui = self.ui.clone();
        div()
            .absolute()
            .top(px(12.))
            .right(px(16.))
            .child(
                Button::new("report-stop")
                    .ghost()
                    .small()
                    .icon(IconName::Circle)
                    .label(STOP_LABEL)
                    .accessibility_label(STOP_LABEL)
                    .tooltip("Recording a bug report. Click to stop.")
                    .text_color(rgb(draw::RED))
                    .on_click(move |_, window, cx| toggle(&ui, window, cx)),
            )
    }

    fn sheet(&self, form: &ReportForm, look: Look, cx: &mut Context<Self>) -> Div {
        let ui = &self.ui;
        let matches = form.matches();
        let pinned: Vec<_> = form.comments.iter().enumerate().collect();
        let copied = form.copied;
        let content = col()
            .id("report-sheet")
            .role(Role::Dialog)
            .aria_label(SHEET_LABEL)
            .w(px(theme::SHEET))
            .max_h(relative(0.86))
            .overflow_y_scroll()
            .gap_3()
            .p_4()
            .rounded(px(10.))
            .bg(rgb(look.card))
            .text_color(rgb(look.text))
            .border_1()
            .border_color(rgb(look.line))
            .shadow_lg()
            .test_support()
            .child(widgets::title(match form.kind {
                ReportKind::Recording => "Bug report from your recording",
                ReportKind::Snapshot => "Bug report from a snapshot",
            }))
            .child(widgets::faint(format!("Saved in {}", form.folder.display())))
            .child(field(
                "What happened",
                Textarea::new(&self.happened)
                    .h(px(70.))
                    .aria_label("What happened"),
            ))
            .child(field(
                "What you expected",
                Textarea::new(&self.expected)
                    .h(px(70.))
                    .aria_label("What you expected"),
            ))
            .child(field(
                "Pin a comment to",
                col()
                    .gap_1()
                    .child(Input::new(&self.query).aria_label("Pin a comment to"))
                    .child(
                        col()
                            .id("report-pins")
                            .max_h(px(150.))
                            .overflow_y_scroll()
                            .children(matches.iter().map(|&at| {
                                widgets::ghost(
                                    ui,
                                    ("report-pin", at),
                                    form.elements[at].to_string(),
                                    AppAction::Report(ReportAction::Pin(at)),
                                )
                            }))
                            .when(matches.is_empty(), |list| {
                                list.child(widgets::faint("Nothing on screen matches."))
                            }),
                    ),
            ))
            .children(pinned.into_iter().map(|(at, comment)| {
                self.comment(at, comment, cx)
            }))
            .child(self.marks(form, look, cx))
            .children(form.error.clone().map(|error| {
                div()
                    .id("report-error")
                    .role(Role::Alert)
                    .aria_label(error.clone())
                    .test_support()
                    .text_color(rgb(0xff453a))
                    .child(error)
            }))
            .child(
                row()
                    .justify_end()
                    .gap_2()
                    .children(copied.then(|| widgets::faint("Copied")))
                    .child({
                        let ui = ui.clone();
                        Button::new("report-copy")
                            .small()
                            .label("Copy report")
                            .on_click(move |_, _, cx| copy(&ui, cx))
                    })
                    .child(widgets::primary(
                        ui,
                        "report-done",
                        "Done",
                        AppAction::Report(ReportAction::Close),
                    )),
            )
            .focus_trap("report-trap", &self.happened.focus_handle(cx));
        let ui = ui.clone();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(
                div()
                    .id("report-scrim")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(if look.content == 0xffffff {
                        rgba(0x0000_0030)
                    } else {
                        rgba(0x0000_0080)
                    })
                    .occlude()
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::Report(ReportAction::Close), cx);
                    }),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .justify_center()
                    .pt(px(70.))
                    .child(
                        div()
                            .id("report-holder")
                            .max_w(relative(0.94))
                            .occlude()
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(content),
                    ),
            )
    }

    /// The "Marks" section: the button that enters draw mode and a line
    /// per stroke.
    fn marks(&self, form: &ReportForm, look: Look, cx: &mut Context<Self>) -> Div {
        let header = if form.strokes.is_empty() {
            "Marks".to_owned()
        } else {
            format!("Marks · {}", form.strokes.len())
        };
        col()
            .gap_1()
            .child(widgets::dim(header).text_size(px(12.)))
            .children(form.strokes.iter().enumerate().map(|(at, stroke)| {
                let number = at + 1;
                row()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .text_color(rgb(look.secondary))
                            .child(format!("{number}. {}", stroke.describe())),
                    )
                    .child(
                        Button::new(("report-unmark", at))
                            .ghost()
                            .small()
                            .label("Remove")
                            .accessibility_label(format!("Remove mark {number}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.ui.send(
                                    AppAction::Report(ReportAction::RemoveStroke(at)),
                                    cx,
                                );
                            })),
                    )
            }))
            .child(
                row().child(widgets::ghost(
                    &self.ui,
                    "report-draw",
                    "Draw on the window",
                    AppAction::Report(ReportAction::Draw),
                )),
            )
    }

    /// Draw mode: a full-window layer that takes every pointer event, the
    /// marks painted on it and the pill of tools.
    fn drawing(&self, form: &ReportForm, look: Look, window: &Window, cx: &mut Context<Self>) -> Div {
        let viewport = window.viewport_size();
        let extent = (f32::from(viewport.width), f32::from(viewport.height));
        let finished: Vec<Vec<Mark>> = form.strokes.iter().map(|s| s.points.clone()).collect();
        let live = self.drag.clone().unwrap_or_default();
        let none = form.strokes.is_empty();
        let layer = div()
            .id("report-layer")
            .role(Role::Canvas)
            .aria_label("Drawing layer")
            .test_support()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .cursor(CursorStyle::Crosshair)
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.drag = Some(vec![draw::mark(event.position)]);
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if let Some(points) = this.drag.as_mut() {
                    let next = draw::mark(event.position);
                    if points.last() != Some(&next) {
                        points.push(next);
                        cx.notify();
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.finish_stroke(event.position, cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| this.finish_stroke(event.position, cx)),
            )
            .child(draw::strokes(finished, live))
            .children(
                form.strokes
                    .iter()
                    .enumerate()
                    .filter_map(|(at, stroke)| {
                        stroke.points.first().map(|&first| draw::badge(at + 1, first, extent))
                    }),
            );
        let tool = |id: &'static str, text: &'static str, label: &'static str, action: ReportAction| {
            let ui = self.ui.clone();
            Button::new(id)
                .ghost()
                .small()
                .label(text)
                .accessibility_label(label)
                .disabled(none)
                .on_click(move |_, _, cx| ui.send(AppAction::Report(action.clone()), cx))
        };
        let done = self.ui.clone();
        let pill = row()
            .id("report-tools")
            .role(Role::Toolbar)
            .aria_label("Draw tools")
            .test_support()
            .h(px(36.))
            .px_3()
            .gap_2()
            .items_center()
            .rounded(px(10.))
            .bg(rgb(look.card))
            .border_1()
            .border_color(rgb(look.line))
            .shadow_lg()
            .cursor(CursorStyle::Arrow)
            .occlude()
            .child(
                Button::new("report-pen")
                    .ghost()
                    .small()
                    .label("Pen")
                    .accessibility_label("Pen")
                    .selected(true)
                    .text_color(rgb(look.accent)),
            )
            .child(div().w(px(1.)).h(px(18.)).bg(rgb(look.line)))
            .child(tool("report-undo", "Undo", "Undo last mark", ReportAction::UndoStroke))
            .child(tool("report-clear", "Clear", "Clear all marks", ReportAction::ClearStrokes))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child("Draw around what's wrong · Esc to finish"),
            )
            .child(
                Button::new("report-finish")
                    .primary()
                    .small()
                    .label("Done")
                    .accessibility_label("Finish drawing")
                    .on_click(move |_, _, cx| {
                        done.send(AppAction::Report(ReportAction::DrawDone), cx);
                    }),
            );
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(layer)
            .child(
                div()
                    .absolute()
                    .top(px(theme::TOP_BAR + 12.))
                    .left_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(pill),
            )
    }

    /// The button went up: the stroke being drawn is done.
    fn finish_stroke(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(mut points) = self.drag.take() else { return };
        let last = draw::mark(position);
        if points.last() != Some(&last) {
            points.push(last);
        }
        self.ui
            .send(AppAction::Report(ReportAction::Stroke(points)), cx);
        cx.notify();
    }

    fn comment(&self, at: usize, comment: &PinnedComment, cx: &mut Context<Self>) -> Div {
        col()
            .gap_1()
            .child(widgets::dim(comment.target.to_string()).text_size(px(12.)))
            .child(
                row()
                    .gap_2()
                    .children(self.comments.get(at).map(|(input, _)| {
                        div().flex_1().child(
                            Input::new(input).aria_label(format!("Comment on {}", comment.target)),
                        )
                    }))
                    .child(
                        Button::new(("report-unpin", at))
                            .ghost()
                            .small()
                            .label("Remove")
                            .accessibility_label(format!("Remove comment on {}", comment.target))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if at < this.comments.len() {
                                    this.comments.remove(at);
                                }
                                this.ui
                                    .send(AppAction::Report(ReportAction::Unpin(at)), cx);
                            })),
                    ),
            )
    }
}

fn field(label: &'static str, input: impl IntoElement) -> Div {
    col()
        .gap_1()
        .child(widgets::dim(label).text_size(px(12.)))
        .child(input)
}

fn sync_input(input: &Entity<InputState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        let value = value.to_owned();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

fn sync_text(input: &Entity<TextareaState>, value: &str, window: &mut Window, cx: &mut App) {
    if input.read(cx).value().as_ref() != value {
        let value = value.to_owned();
        input.update(cx, |input, cx| input.set_value(value, window, cx));
    }
}

/// Keeps the view and layout trees of the frame drawn after what just
/// happened. The first hop waits out the frame being drawn, the second
/// reads the one after it.
fn capture(recorder: &record::Recorder, window: &Window) {
    let recorder = recorder.clone();
    window.on_next_frame(move |window, _| {
        window.on_next_frame(move |window, _| recorder.frame(window));
    });
}

/// Copies the report's prompt and tells the form how it went.
fn copy(ui: &Rc<Ui>, cx: &mut App) {
    let Some(form) = ui.read(|state| state.report.form.clone()) else {
        return;
    };
    let outcome = match ui.recorder().copy(&form) {
        Ok(prompt) => {
            cx.write_to_clipboard(ClipboardItem::new_string(prompt));
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    };
    ui.send(AppAction::Report(ReportAction::Copied(outcome)), cx);
}

/// Starts a recording. Errors are shown in a toast too.
fn start(ui: &Rc<Ui>, window: &mut Window, cx: &mut App, title: Option<String>) -> Result<(), String> {
    ui.send(AppAction::Report(ReportAction::Started), cx);
    match ui
        .read(|state| ui.recorder().begin(window, state, title))
    {
        Ok(()) => {
            capture(ui.recorder(), window);
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            ui.send(
                AppAction::Report(ReportAction::Failed {
                    message: message.clone(),
                    recording: ui.recorder().is_recording(),
                }),
                cx,
            );
            Err(message)
        }
    }
}

/// Stops the recording and saves it; `open` shows the form at once.
fn stop(
    ui: &Rc<Ui>,
    window: &mut Window,
    cx: &mut App,
    title: Option<String>,
    open: bool,
) -> Result<PathBuf, String> {
    saved(ui, cx, open, ui.recorder().finish(window, title))
}

/// Saves a snapshot; `open` shows the form at once.
fn snap(
    ui: &Rc<Ui>,
    window: &mut Window,
    cx: &mut App,
    title: Option<String>,
    open: bool,
) -> Result<PathBuf, String> {
    let made = ui.read(|state| ui.recorder().snapshot(window, state, title));
    saved(ui, cx, open, made)
}

fn saved(
    ui: &Rc<Ui>,
    cx: &mut App,
    open: bool,
    made: Result<ReportForm, record::ReportError>,
) -> Result<PathBuf, String> {
    match made {
        Ok(form) => {
            let folder = form.folder.clone();
            ui.send(AppAction::Report(ReportAction::Saved { form, open }), cx);
            if let Some(warning) = ui.recorder().take_warning() {
                ui.send(
                    AppAction::Report(ReportAction::Failed {
                        message: warning.to_string(),
                        recording: ui.recorder().is_recording(),
                    }),
                    cx,
                );
            }
            Ok(folder)
        }
        Err(error) => {
            let message = error.to_string();
            ui.send(
                AppAction::Report(ReportAction::Failed {
                    message: message.clone(),
                    recording: ui.recorder().is_recording(),
                }),
                cx,
            );
            Err(message)
        }
    }
}

/// ⇧⌘R and the dot: stop when recording, else start.
fn toggle(ui: &Rc<Ui>, window: &mut Window, cx: &mut App) {
    let _ = if ui.recorder().is_recording() {
        stop(ui, window, cx, None, true).map(drop)
    } else {
        start(ui, window, cx, None)
    };
}

/// The report shortcuts, on the window's root.
pub(crate) fn actions<E: InteractiveElement>(root: E, ui: &Rc<Ui>) -> E {
    let toggling = ui.clone();
    let snapping = ui.clone();
    root.on_action(move |_: &keys::ToggleRecording, window, cx| toggle(&toggling, window, cx))
        .on_action(move |_: &keys::TakeSnapshot, window, cx| take_snapshot(&snapping, window, cx))
}

/// ⇧⌘S: a snapshot, with the form unless a recording runs.
fn take_snapshot(ui: &Rc<Ui>, window: &mut Window, cx: &mut App) {
    let open = !ui.recorder().is_recording();
    let _ = snap(ui, window, cx, None, open);
}

/// What a palette command asked for, run after its palette is gone.
fn run_request(ui: &Rc<Ui>, request: ReportRequest, window: &mut Window, cx: &mut App) {
    match request {
        ReportRequest::Toggle => toggle(ui, window, cx),
        ReportRequest::Snapshot => take_snapshot(ui, window, cx),
        ReportRequest::ReportBug => {
            let _ = if ui.recorder().is_recording() {
                stop(ui, window, cx, None, true).map(drop)
            } else {
                snap(ui, window, cx, None, true).map(drop)
            };
        }
    }
}

/// Answers `humata report record|stop|snapshot` on the socket at `path`
/// for the life of the app. The window is the one the app opened; saves the
/// CLI asks for show the toast, not the form (D133.5).
///
/// # Errors
///
/// [`RemoteError`] when the socket can't be bound, such as when another
/// Humata 2 already answers there.
pub fn serve(
    cx: &mut App,
    ui: &Rc<Ui>,
    window: AnyWindowHandle,
    runtime: &tokio::runtime::Handle,
    path: &Path,
) -> Result<(), RemoteError> {
    let mut listener = {
        let _inside = runtime.enter();
        Listener::bind(path)?
    };
    let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
    runtime.spawn(async move {
        loop {
            match listener.next().await {
                Ok(asked) => {
                    if send.send(asked).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    eprintln!("Humata's report socket stopped answering: {error}");
                    break;
                }
            }
        }
    });
    let ui = ui.clone();
    let runtime = runtime.clone();
    cx.spawn(async move |cx| {
        while let Some((request, responder)) = receive.recv().await {
            let answer = cx.update(|cx| {
                window
                    .update(cx, |_, window, cx| answer(&ui, request, window, cx))
                    .unwrap_or_else(|_| {
                        Err(Refusal {
                            kind: RefusalKind::Failed,
                            message: "Humata's window is closed. Open the app again and rerun \
                                      the command."
                                .to_owned(),
                        })
                    })
            });
            runtime.spawn(async move {
                if let Err(error) = responder.reply(answer).await {
                    eprintln!("Humata couldn't answer a report command: {error}");
                }
            });
        }
    })
    .detach();
    Ok(())
}

/// [`serve`] at the default socket (`HUMATA_APP_SOCKET`, else
/// `~/.humata2/app.sock`).
///
/// # Errors
///
/// [`RemoteError`] when there is no home folder or the socket can't be
/// bound.
pub fn serve_default(
    cx: &mut App,
    ui: &Rc<Ui>,
    window: AnyWindowHandle,
    runtime: &tokio::runtime::Handle,
) -> Result<(), RemoteError> {
    serve(cx, ui, window, runtime, &remote::socket_path()?)
}

fn refuse(kind: RefusalKind, message: impl Into<String>) -> Answer {
    Err(Refusal {
        kind,
        message: message.into(),
    })
}

fn answer(ui: &Rc<Ui>, request: Request, window: &mut Window, cx: &mut App) -> Answer {
    let recording = || ui.recorder().is_recording();
    match request {
        Request::Record { title } => {
            if recording() {
                return refuse(
                    RefusalKind::AlreadyRecording,
                    "A recording is already running. Stop it first with `humata report stop`.",
                );
            }
            match start(ui, window, cx, title) {
                Ok(()) => Ok(Reply {
                    folder: None,
                    recording: true,
                }),
                Err(message) => refuse(RefusalKind::Failed, message),
            }
        }
        Request::Stop => {
            if !recording() {
                return refuse(
                    RefusalKind::NotRecording,
                    "No recording is running. Start one with `humata report record`.",
                );
            }
            match stop(ui, window, cx, None, false) {
                Ok(folder) => Ok(Reply {
                    folder: Some(folder.display().to_string()),
                    recording: false,
                }),
                Err(message) => refuse(RefusalKind::Failed, message),
            }
        }
        Request::Snapshot { title } => match snap(ui, window, cx, title, false) {
            Ok(folder) => Ok(Reply {
                folder: Some(folder.display().to_string()),
                recording: recording(),
            }),
            Err(message) => refuse(RefusalKind::Failed, message),
        },
    }
}

impl Render for ReportView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::of(cx);
        let (recording, form) = self
            .ui
            .read(|state| (state.report.recording, state.report.form.clone()));
        if recording {
            self.tick(window, cx);
        }
        self.follow(form.as_ref(), window, cx);
        let drawing = form.as_ref().is_some_and(|form| form.drawing);
        if !drawing {
            self.drag = None;
        }
        self.request(window, cx);
        // Clicks and scrolls, before anything handles them, so a
        // recording names what the pointer was over.
        let recorder = self.ui.recorder().clone();
        let quiet = self.ui.clone();
        let hook = canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                let on_down = recorder.clone();
                window.on_mouse_event(move |event: &MouseDownEvent, phase, window, _| {
                    if phase != DispatchPhase::Capture
                        || !on_down.is_recording()
                        || quiet.read(|state| state.report.form.as_ref().is_some_and(|f| f.drawing))
                    {
                        return;
                    }
                    let elements = snapshots(window);
                    if let Some(target) = record::under(&elements, event.position)
                        && target.label.as_deref() != Some(STOP_LABEL)
                    {
                        let button = match event.button {
                            MouseButton::Right => Pointer::Right,
                            MouseButton::Middle => Pointer::Middle,
                            _ => Pointer::Left,
                        };
                        on_down.gesture(Gesture::Click { target, button });
                    }
                    capture(&on_down, window);
                });
                let on_scroll = recorder.clone();
                window.on_mouse_event(move |event: &ScrollEvent, phase, window, _| {
                    if phase != DispatchPhase::Capture || !on_scroll.is_recording() {
                        return;
                    }
                    let delta = event.delta.pixel_delta(px(LINE));
                    let (dx, dy) = (whole(delta.x), whole(delta.y));
                    if dx == 0 && dy == 0 {
                        return;
                    }
                    let elements = snapshots(window);
                    if let Some(target) = record::under(&elements, event.position) {
                        on_scroll.gesture(Gesture::Scroll { target, dx, dy });
                    }
                    capture(&on_scroll, window);
                });
            },
        )
        .absolute()
        .size_full();
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .child(hook)
            .when(recording, |layer| layer.child(self.dot()))
            .children(form.map(|form| {
                if form.drawing {
                    self.drawing(&form, look, window, cx)
                } else {
                    self.sheet(&form, look, cx)
                }
            }))
    }
}

/// `length` in whole logical pixels.
#[allow(clippy::cast_possible_truncation)]
fn whole(length: Pixels) -> i32 {
    f32::from(length).round() as i32
}
