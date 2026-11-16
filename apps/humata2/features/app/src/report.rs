use std::fmt;
use std::path::PathBuf;

use tca::Effect;

use crate::{App, AppAction, AppState, ToastAction};

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
        }
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

/// Whether a report is being recorded and the form over the last one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReportState {
    /// A recording is running.
    pub recording: bool,
    /// The open form.
    pub form: Option<ReportForm>,
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
}

impl App {
    pub(super) fn report(&self, state: &mut AppState, action: ReportAction) -> Effect<AppAction> {
        let clock = self.clients.clock.clone();
        match action {
            ReportAction::Started => state.report.recording = true,
            ReportAction::Saved { form, open } => {
                if form.kind == ReportKind::Recording {
                    state.report.recording = false;
                }
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
            ReportAction::Close => state.report.form = None,
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
