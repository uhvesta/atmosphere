use std::fmt;
use std::path::PathBuf;

use humata2_clients::model::MachineId;
use humata2_hosts::HostsAction;
use tca::Effect;

use crate::{App, AppAction, AppState, ToastAction};

#[path = "report_draw.rs"]
mod draw;

/// How many elements the pin list shows at once.
const MATCH_LIMIT: usize = 50;

/// What a report captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportKind {
    /// A recording of the screen over time.
    Recording,
    /// One saved view.
    Snapshot,
}

/// A position in window points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    /// Distance from the left edge.
    pub x: i32,
    /// Distance from the top edge.
    pub y: i32,
}

/// A box in window points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width.
    pub width: i32,
    /// Height.
    pub height: i32,
}

impl Rect {
    /// Whether `point` lies in the box, edges on the left and top only.
    pub fn contains(&self, point: Point) -> bool {
        let (px, py) = (i64::from(point.x), i64::from(point.y));
        px >= i64::from(self.x)
            && py >= i64::from(self.y)
            && px < i64::from(self.x) + i64::from(self.width)
            && py < i64::from(self.y) + i64::from(self.height)
    }

    /// The middle of the box.
    pub fn centre(&self) -> Point {
        Point {
            x: self.x + self.width / 2,
            y: self.y + self.height / 2,
        }
    }
}

/// What a stroke looks like.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A path that comes back to its start.
    Loop,
    /// A path that goes elsewhere.
    Line,
    /// A click.
    Dot,
}

/// One mark drawn on the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stroke {
    /// What it looks like.
    pub shape: Shape,
    /// The path, in window points.
    pub points: Vec<Point>,
    /// The elements it marks, at most five.
    pub covers: Vec<PinTarget>,
    /// How many more it marks.
    pub more: usize,
}

impl Stroke {
    /// The mark in words, as the prompt lists it.
    pub fn describe(&self) -> String {
        let lead = match self.shape {
            Shape::Loop => "a circle around ",
            Shape::Line => "a line across ",
            Shape::Dot => "a dot on ",
        };
        if self.covers.is_empty() {
            let at = match self.shape {
                Shape::Loop => bounds_centre(&self.points),
                _ => self.points.first().copied(),
            }
            .unwrap_or(Point { x: 0, y: 0 });
            return format!("{lead}an unlabelled area near ({}, {})", at.x, at.y);
        }
        let names: Vec<String> = self.covers.iter().map(ToString::to_string).collect();
        let mut text = format!("{lead}{}", names.join(", "));
        if self.more > 0 {
            text.push_str(&format!(" and {} more", self.more));
        }
        text
    }
}

fn bounds_centre(points: &[Point]) -> Option<Point> {
    let first = points.first()?;
    let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x, first.y);
    for p in points {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    Some(Point {
        x: x0 + (x1 - x0) / 2,
        y: y0 + (y1 - y0) / 2,
    })
}

/// An element a comment is pinned to, named as a report names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinTarget {
    /// Its role, such as `Button`.
    pub role: String,
    /// Its label.
    pub label: String,
    /// (role, label) of each enclosing labelled element, nearest first.
    pub within: Vec<(String, String)>,
}

impl fmt::Display for PinTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:?}", self.role, self.label)?;
        for (role, label) in &self.within {
            write!(f, " in {role} {label:?}")?;
        }
        Ok(())
    }
}

/// A comment pinned to an element.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedComment {
    /// What it is about.
    pub target: PinTarget,
    /// What it says.
    pub text: String,
}

/// The bug-report form over a saved report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReportForm {
    /// Where the report was saved.
    pub folder: PathBuf,
    /// What it captured.
    pub kind: ReportKind,
    /// What happened.
    pub happened: String,
    /// What should have happened.
    pub expected: String,
    /// What can be pinned: the labelled elements of the saved view tree.
    pub elements: Vec<PinTarget>,
    /// Filters `elements`.
    pub query: String,
    /// The pinned comments.
    pub comments: Vec<PinnedComment>,
    /// Copy report succeeded and nothing changed since.
    pub copied: bool,
    /// Why the last Copy failed.
    pub error: Option<String>,
    /// Draw mode is on.
    pub drawing: bool,
    /// The marks drawn on the window.
    pub strokes: Vec<Stroke>,
    /// Each element's box in window points, parallel to `elements`.
    pub places: Vec<Option<Rect>>,
    /// The window's size in points.
    pub window: (u32, u32),
}

impl ReportForm {
    /// A form with what happened set to `title` and the rest empty.
    pub fn new(
        folder: PathBuf,
        kind: ReportKind,
        title: String,
        elements: Vec<PinTarget>,
    ) -> Self {
        Self {
            folder,
            kind,
            happened: title,
            expected: String::new(),
            elements,
            query: String::new(),
            comments: Vec::new(),
            copied: false,
            error: None,
            drawing: false,
            strokes: Vec::new(),
            places: Vec::new(),
            window: (0, 0),
        }
    }

    /// The same form with where each element sits and how big the window is.
    pub fn with_places(mut self, places: Vec<Option<Rect>>, window: (u32, u32)) -> Self {
        self.places = places;
        self.window = window;
        self
    }

    /// Indices into `elements` whose name contains `query`, ignoring case,
    /// at most 50, in order.
    pub fn matches(&self) -> Vec<usize> {
        let query = self.query.to_lowercase();
        self.elements
            .iter()
            .enumerate()
            .filter(|(_, element)| element.to_string().to_lowercase().contains(&query))
            .map(|(index, _)| index)
            .take(MATCH_LIMIT)
            .collect()
    }
}

/// What a palette command asks of the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportRequest {
    /// Start or stop a recording.
    Toggle,
    /// Take a snapshot.
    Snapshot,
    /// Stop a recording, else take a snapshot.
    ReportBug,
}

/// Whether a report is being recorded and the form over the last one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportState {
    /// A recording is running.
    pub recording: bool,
    /// The open form.
    pub form: Option<ReportForm>,
    /// The last report of this run, as last edited.
    pub last: Option<ReportForm>,
    /// A command waiting for the shell to act on it.
    pub request: Option<ReportRequest>,
}

/// A change to the report state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportAction {
    /// A recording began.
    Started,
    /// A report was saved.
    Saved {
        /// Its form.
        form: ReportForm,
        /// Open the form now, else offer it in a toast.
        open: bool,
    },
    /// A report command failed.
    Failed {
        /// What to tell the person.
        message: String,
        /// Whether a recording is still running.
        recording: bool,
    },
    /// What happened changed.
    Happened(String),
    /// What should have happened changed.
    Expected(String),
    /// The element filter changed.
    Query(String),
    /// Pin a comment to the element at this index.
    Pin(usize),
    /// A comment's text changed.
    CommentText(usize, String),
    /// Remove the comment at this index.
    Unpin(usize),
    /// Copying the report finished.
    Copied(Result<(), String>),
    /// Close the form.
    Close,
    /// A palette command wants this done.
    Request(ReportRequest),
    /// The shell acted on the request.
    Handled,
    /// Open the last report again.
    OpenLast,
    /// Enter draw mode.
    Draw,
    /// A stroke was drawn.
    Stroke(Vec<Point>),
    /// Remove the newest stroke.
    UndoStroke,
    /// Remove every stroke.
    ClearStrokes,
    /// Remove the stroke at this index.
    RemoveStroke(usize),
    /// Leave draw mode.
    DrawDone,
}

impl App {
    /// Reads, once, the daemon version of each machine the app has none for,
    /// so a report lists it. Nothing repeats: a read that fails leaves the
    /// version unknown until the next report.
    fn read_daemon_versions(&self, state: &AppState) -> Effect<AppAction> {
        let missing: Vec<MachineId> = state
            .workspace
            .machines
            .iter()
            .map(|machine| machine.id.clone())
            .filter(|machine| state.daemon_version(machine).is_none())
            .collect();
        Effect::merge(missing.into_iter().map(|machine| {
            let call = self.clients.machines.detail(&machine);
            Effect::future(async move { AppAction::Hosts(HostsAction::Health(machine, call.await)) })
        }))
    }

    pub(super) fn report(&self, state: &mut AppState, action: ReportAction) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match action {
            ReportAction::Started => {
                state.report.recording = true;
                return self.read_daemon_versions(state);
            }
            ReportAction::Saved { form, open } => {
                if form.kind == ReportKind::Recording {
                    state.report.recording = false;
                }
                state.report.last = Some(form.clone());
                if open {
                    state.report.form = Some(form);
                } else {
                    let notice = state.toast("Report saved · Add details".to_owned(), &clock);
                    if let Some(toast) = state.toasts.last_mut() {
                        toast.action = Some(ToastAction::OpenReport(Box::new(form)));
                    }
                    return notice;
                }
            }
            ReportAction::Failed { message, recording } => {
                state.report.recording = recording;
                return state.toast(message, &clock);
            }
            ReportAction::Close => {
                if let Some(form) = state.report.form.take() {
                    state.report.last = Some(form);
                }
            }
            ReportAction::Request(request) => state.report.request = Some(request),
            ReportAction::Handled => state.report.request = None,
            ReportAction::OpenLast => {
                if state.report.form.is_some() {
                    return Effect::none();
                }
                match state.report.last.clone() {
                    Some(form) => state.report.form = Some(form),
                    None => {
                        return state.toast(
                            "No report yet. Press ⇧⌘S or run \"Report a bug…\" to make one."
                                .to_owned(),
                            &clock,
                        );
                    }
                }
            }
            action => {
                let Some(form) = state.report.form.as_mut() else {
                    return Effect::none();
                };
                match action {
                    ReportAction::Happened(text) => {
                        form.happened = text;
                        form.copied = false;
                    }
                    ReportAction::Expected(text) => {
                        form.expected = text;
                        form.copied = false;
                    }
                    ReportAction::Query(query) => form.query = query,
                    ReportAction::Pin(index) => {
                        if let Some(target) = form.elements.get(index) {
                            form.comments.push(PinnedComment {
                                target: target.clone(),
                                text: String::new(),
                            });
                            form.copied = false;
                        }
                    }
                    ReportAction::CommentText(index, text) => {
                        if let Some(comment) = form.comments.get_mut(index) {
                            comment.text = text;
                            form.copied = false;
                        }
                    }
                    ReportAction::Unpin(index) => {
                        if index < form.comments.len() {
                            form.comments.remove(index);
                            form.copied = false;
                        }
                    }
                    ReportAction::Draw => form.drawing = true,
                    ReportAction::DrawDone => form.drawing = false,
                    ReportAction::Stroke(points) => {
                        if form.drawing && !points.is_empty() {
                            let targets: Vec<(&PinTarget, Rect)> = form
                                .elements
                                .iter()
                                .zip(form.places.iter())
                                .filter_map(|(target, place)| place.map(|rect| (target, rect)))
                                .collect();
                            let shape = draw::classify(&points);
                            let (covers, more) = draw::covers(shape, &points, &targets);
                            form.strokes.push(Stroke {
                                shape,
                                points,
                                covers,
                                more,
                            });
                            form.copied = false;
                        }
                    }
                    ReportAction::UndoStroke => {
                        if form.strokes.pop().is_some() {
                            form.copied = false;
                        }
                    }
                    ReportAction::ClearStrokes => {
                        if !form.strokes.is_empty() {
                            form.strokes.clear();
                            form.copied = false;
                        }
                    }
                    ReportAction::RemoveStroke(index) => {
                        if index < form.strokes.len() {
                            form.strokes.remove(index);
                            form.copied = false;
                        }
                    }
                    ReportAction::Copied(Ok(())) => {
                        form.copied = true;
                        form.error = None;
                    }
                    ReportAction::Copied(Err(message)) => {
                        form.copied = false;
                        form.error = Some(message);
                    }
                    _ => {}
                }
            }
        }
        Effect::none()
    }
}
