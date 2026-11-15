//! The review chat's agent working the view (PR-22–PR-26, D59): it focuses
//! files and lines, highlights ranges with labels, leaves notes for the user
//! and orders the files. Each change applies at once, is listed with Undo,
//! and Back returns to where the user was before the agent last moved the
//! view. All of it is saved with the session, so it survives restarts.

use serde::{Deserialize, Serialize};

use super::anchor::{LineRange, Placement, place, quote};
use super::memory::ReviewMemory;
use super::prepared::{Layout, NoteAt, PreparedReview, Showing};
use super::review::{FileKey, Review, Side};
use crate::error::ActionError;

/// The most agent changes kept for Undo; older ones can't be undone.
const KEPT_CHANGES: usize = 50;

/// What the agent may do to the view, from the review's Info panel
/// ("Agent may: Focus, Notes, Order", PR-25). All on by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMay {
    pub focus: bool,
    pub notes: bool,
    pub order: bool,
}

impl Default for AgentMay {
    fn default() -> Self {
        Self {
            focus: true,
            notes: true,
            order: true,
        }
    }
}

/// One of the Info panel's switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Switch {
    Focus,
    Notes,
    Order,
}

impl std::fmt::Display for Switch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Focus => "Focus",
            Self::Notes => "Notes",
            Self::Order => "Order",
        })
    }
}

impl AgentMay {
    pub fn allows(self, switch: Switch) -> bool {
        match switch {
            Switch::Focus => self.focus,
            Switch::Notes => self.notes,
            Switch::Order => self.order,
        }
    }

    pub fn set(&mut self, switch: Switch, on: bool) {
        match switch {
            Switch::Focus => self.focus = on,
            Switch::Notes => self.notes = on,
            Switch::Order => self.order = on,
        }
    }
}

/// Where on a file a tool points: lines, a hunk, or the whole file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub file: FileKey,
    pub side: Side,
    /// The lines; with neither lines nor hunk, the whole file.
    pub lines: Option<LineRange>,
    /// A hunk's index in the file's diff, used when `lines` is none.
    pub hunk: Option<usize>,
}

impl Target {
    pub fn lines(file: FileKey, side: Side, lines: LineRange) -> Self {
        Self {
            file,
            side,
            lines: Some(lines),
            hunk: None,
        }
    }

    pub fn file(file: FileKey) -> Self {
        Self {
            file,
            side: Side::New,
            lines: None,
            hunk: None,
        }
    }
}

/// A range to highlight, with its short label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightAt {
    pub target: Target,
    pub label: String,
}

/// What the agent asked for, as the MCP tools say it (PR-23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tool {
    /// `review_focus`: open the file, scroll to the target, select it if
    /// `select`, and highlight `highlights` (in any files), replacing the
    /// highlights shown. With no highlights, those shown stay.
    Focus {
        target: Target,
        select: bool,
        highlights: Vec<HighlightAt>,
    },
    /// `review_note`: a note for the user, under its target.
    Note { target: Target, text: String },
    /// `review_prepare`: order, group and annotate the files.
    Prepare(PreparedReview),
}

/// One tool call from the review chat's agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAction {
    /// The call's id, unique in the session; Undo names it.
    pub id: String,
    /// The agent's name, shown on its notes ("Claude Code").
    pub agent: String,
    pub tool: Tool,
}

impl AgentAction {
    /// The switch that allows it.
    pub fn switch(&self) -> Switch {
        match self.tool {
            Tool::Focus { .. } => Switch::Focus,
            Tool::Note { .. } => Switch::Notes,
            Tool::Prepare(_) => Switch::Order,
        }
    }

    /// Whether it moves the view, so it waits while the user is scrolling,
    /// typing or selecting (PR-24). A note only adds a card.
    pub fn moves_view(&self) -> bool {
        !matches!(self.tool, Tool::Note { .. })
    }

    /// The chat's line for it: "review_focus app/refund.rs:140".
    pub fn summary(&self) -> String {
        let at = |target: &Target| match (target.lines, target.hunk) {
            (Some(lines), _) if lines.len() == 1 => format!("{}:{}", target.file, lines.first),
            (Some(lines), _) => format!("{}:{}-{}", target.file, lines.first, lines.last),
            (None, Some(hunk)) => format!("{} hunk {}", target.file, hunk + 1),
            (None, None) => target.file.to_string(),
        };
        match &self.tool {
            Tool::Focus { target, .. } => format!("review_focus {}", at(target)),
            Tool::Note { target, .. } => format!("review_note {}", at(target)),
            Tool::Prepare(view) => format!("review_prepare “{}”", view.title),
        }
    }
}

/// A highlighted range, following its lines as the file changes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Highlight {
    pub file: FileKey,
    pub side: Side,
    pub lines: LineRange,
    pub label: String,
    pub quoted: Vec<String>,
    /// Its lines changed since; it's no longer drawn.
    pub outdated: bool,
}

/// A note, numbered within its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NoteId(pub u64);

impl std::fmt::Display for NoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A note from the agent for the user, never posted (PR-23).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentNote {
    pub id: NoteId,
    pub file: FileKey,
    /// The lines, or none for the whole file.
    pub at: Option<NoteAt>,
    pub text: String,
    pub agent: String,
    /// Its lines changed since; it's kept, under the file (RV-34).
    pub outdated: bool,
}

/// Lines selected on one side of a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Selection {
    pub file: FileKey,
    pub side: Side,
    pub lines: LineRange,
}

/// Where the user is: the file, and what's selected.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spot {
    pub current: Option<FileKey>,
    pub selection: Option<Selection>,
}

/// How to take an agent change back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Undo {
    Focus { before: Spot, highlights: Vec<Highlight> },
    Note(NoteId),
    Prepare {
        before: Layout,
        prepared: Option<Box<Showing>>,
        notes: Vec<NoteId>,
    },
}

/// An agent change, as the chat lists it with Undo.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChange {
    pub id: String,
    pub summary: String,
    pub undo: Undo,
    pub undone: bool,
}

/// What the agent did to one session's view.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentView {
    #[serde(default)]
    pub may: AgentMay,
    pub highlights: Vec<Highlight>,
    pub notes: Vec<AgentNote>,
    next_note: u64,
    /// Newest last.
    pub changes: Vec<AgentChange>,
    /// What the agent last selected, shown again after a restart.
    pub selection: Option<Selection>,
    /// Where the user was before the agent last moved the view.
    pub back: Option<Spot>,
}

impl AgentView {
    pub fn note(&self, id: NoteId) -> Option<&AgentNote> {
        self.notes.iter().find(|note| note.id == id)
    }

    /// Takes the note away. The change that added it is no longer listed
    /// with Undo: the note is gone, or the user's own now.
    pub fn remove_note(&mut self, id: NoteId) -> Option<AgentNote> {
        let index = self.notes.iter().position(|note| note.id == id)?;
        for change in &mut self.changes {
            if change.undo == Undo::Note(id) {
                change.undone = true;
            }
        }
        Some(self.notes.remove(index))
    }

    /// The highlights over `line` on `side` of `file`.
    pub fn highlights_at<'a>(
        &'a self,
        file: &'a FileKey,
        side: Side,
        line: u32,
    ) -> impl Iterator<Item = &'a Highlight> + 'a {
        self.highlights
            .iter()
            .filter(move |h| !h.outdated && &h.file == file && h.side == side && h.lines.contains(line))
    }

    /// Adds `notes` from an agent-prepared review, quoting their lines.
    pub(crate) fn add_prepared_notes(&mut self, review: &Review, view: &PreparedReview) -> Vec<NoteId> {
        view.notes
            .iter()
            .map(|note| {
                let at = note.at.clone().map(|mut at| {
                    if let Some(diff) = review.file(&note.file).and_then(|file| file.text()) {
                        at.quoted = quote(&diff.side_lines(at.side), at.lines);
                    }
                    at
                });
                self.push_note(note.file.clone(), at, note.text.clone(), view.agent.clone())
            })
            .collect()
    }

    fn push_note(&mut self, file: FileKey, at: Option<NoteAt>, text: String, agent: String) -> NoteId {
        self.next_note += 1;
        let id = NoteId(self.next_note);
        self.notes.push(AgentNote {
            id,
            file,
            at,
            text,
            agent,
            outdated: false,
        });
        id
    }

    /// The file changed: highlights and notes on it follow their lines, or
    /// are outdated.
    pub(crate) fn reanchor(&mut self, file: &FileKey, old: &[(u32, &str)], new: &[(u32, &str)]) {
        let pick = |side: Side| match side {
            Side::Old => old,
            Side::New => new,
        };
        for h in self.highlights.iter_mut().filter(|h| &h.file == file) {
            match place(&h.quoted, h.lines, pick(h.side)) {
                Placement::Unchanged => {}
                Placement::Moved { lines } => h.lines = lines,
                Placement::Outdated => h.outdated = true,
            }
        }
        for note in self.notes.iter_mut().filter(|note| &note.file == file) {
            let Some(at) = &mut note.at else { continue };
            match place(&at.quoted, at.lines, pick(at.side)) {
                Placement::Unchanged => {}
                Placement::Moved { lines } => at.lines = lines,
                Placement::Outdated => note.outdated = true,
            }
        }
    }

    fn record(&mut self, id: String, summary: String, undo: Undo) {
        self.changes.push(AgentChange {
            id,
            summary,
            undo,
            undone: false,
        });
        if self.changes.len() > KEPT_CHANGES {
            self.changes.remove(0);
        }
    }
}

/// The lines `target` means in `review`: none for a whole file.
fn resolve(review: &Review, target: &Target) -> Result<Option<LineRange>, ActionError> {
    let file = review.file(&target.file).ok_or_else(|| ActionError::NoFile {
        file: target.file.clone(),
    })?;
    if let Some(lines) = target.lines {
        let Some(diff) = file.text() else {
            return Err(ActionError::NoLines {
                file: target.file.clone(),
                lines,
            });
        };
        let side = diff.side_lines(target.side);
        let has = |line: u32| side.iter().any(|(n, _)| *n == line);
        if !has(lines.first) || !has(lines.last) {
            return Err(ActionError::NoLines {
                file: target.file.clone(),
                lines,
            });
        }
        return Ok(Some(lines));
    }
    let Some(hunk) = target.hunk else {
        return Ok(None);
    };
    let numbers: Vec<u32> = file
        .text()
        .and_then(|diff| diff.hunks.get(hunk))
        .map(|h| h.lines.iter().filter_map(|line| line.number(target.side)).collect())
        .unwrap_or_default();
    match (numbers.iter().min(), numbers.iter().max()) {
        (Some(&first), Some(&last)) => Ok(Some(LineRange::new(first, last))),
        _ => Err(ActionError::NoHunk {
            file: target.file.clone(),
            hunk,
        }),
    }
}

fn note_at(review: &Review, target: &Target, lines: Option<LineRange>) -> Option<NoteAt> {
    let lines = lines?;
    let quoted = review
        .file(&target.file)
        .and_then(|file| file.text())
        .map(|diff| quote(&diff.side_lines(target.side), lines))
        .unwrap_or_default();
    Some(NoteAt {
        side: target.side,
        lines,
        quoted,
    })
}

/// Applies `action` to the view, with the user at `now`. Where the view
/// should go, if it moves. Nothing changes on an error.
pub(crate) fn apply(
    review: &Review,
    memory: &mut ReviewMemory,
    action: AgentAction,
    now: Spot,
) -> Result<Option<Spot>, ActionError> {
    let switch = action.switch();
    if !memory.agent.may.allows(switch) {
        return Err(ActionError::TurnedOff { switch });
    }
    if memory.agent.changes.iter().any(|change| change.id == action.id) {
        // The same call again, such as a retry: it's done already.
        return Ok(None);
    }
    let summary = action.summary();
    match action.tool {
        Tool::Focus {
            target,
            select,
            highlights,
        } => {
            let lines = resolve(review, &target)?;
            let mut made = Vec::with_capacity(highlights.len());
            for at in &highlights {
                let lines = resolve(review, &at.target)?.unwrap_or(LineRange::FILE);
                let quoted = review
                    .file(&at.target.file)
                    .and_then(|file| file.text())
                    .map(|diff| quote(&diff.side_lines(at.target.side), lines))
                    .unwrap_or_default();
                made.push(Highlight {
                    file: at.target.file.clone(),
                    side: at.target.side,
                    lines,
                    label: at.label.clone(),
                    quoted,
                    outdated: false,
                });
            }
            let before = Spot {
                current: now.current.clone(),
                selection: now.selection.clone(),
            };
            let old = if made.is_empty() {
                memory.agent.highlights.clone()
            } else {
                std::mem::replace(&mut memory.agent.highlights, made)
            };
            let selection = lines.filter(|_| select).map(|lines| Selection {
                file: target.file.clone(),
                side: target.side,
                lines,
            });
            let to = Spot {
                current: Some(target.file.clone()),
                selection: selection.clone().or_else(|| {
                    lines.map(|lines| Selection {
                        file: target.file.clone(),
                        side: target.side,
                        lines: LineRange::line(lines.first),
                    })
                }),
            };
            memory.current = to.current.clone();
            memory.collapsed.remove(&target.file);
            memory.agent.selection = selection;
            memory.agent.back = Some(now);
            memory.agent.record(
                action.id,
                summary,
                Undo::Focus {
                    before,
                    highlights: old,
                },
            );
            Ok(Some(to))
        }
        Tool::Note { target, text } => {
            let lines = resolve(review, &target)?;
            let at = note_at(review, &target, lines);
            let id = memory.agent.push_note(target.file, at, text, action.agent);
            memory.agent.record(action.id, summary, Undo::Note(id));
            Ok(None)
        }
        Tool::Prepare(view) => {
            let before = memory.layout();
            let prepared = memory.prepared.clone().map(Box::new);
            let notes = memory.agent.add_prepared_notes(review, &view);
            memory.show_prepared(view, None, notes.clone());
            memory.agent.back = Some(now);
            memory.agent.record(
                action.id,
                summary,
                Undo::Prepare {
                    before,
                    prepared,
                    notes,
                },
            );
            Ok(Some(Spot {
                current: memory.current.clone(),
                selection: None,
            }))
        }
    }
}

/// Takes back the agent change `id`. Where the view should go, if it moves.
pub(crate) fn undo(memory: &mut ReviewMemory, id: &str) -> Result<Option<Spot>, ActionError> {
    let index = memory
        .agent
        .changes
        .iter()
        .position(|change| change.id == id && !change.undone)
        .ok_or_else(|| ActionError::NoChange { id: id.to_owned() })?;
    let change = &mut memory.agent.changes[index];
    change.undone = true;
    let undo = change.undo.clone();
    // A later focus replaced this one's highlights and moved the view since:
    // there's nothing of this one left to take back.
    let replaced = memory.agent.changes[index + 1..]
        .iter()
        .any(|later| !later.undone && matches!(later.undo, Undo::Focus { .. }));
    match undo {
        Undo::Focus { .. } if replaced => Ok(None),
        Undo::Focus { before, highlights } => {
            memory.agent.highlights = highlights;
            memory.agent.selection = None;
            memory.current = before.current.clone();
            memory.agent.back = None;
            Ok(Some(before))
        }
        Undo::Note(note) => {
            memory.agent.remove_note(note);
            Ok(None)
        }
        Undo::Prepare {
            before,
            prepared,
            notes,
        } => {
            memory.restore(before);
            memory.prepared = prepared.map(|showing| *showing);
            memory.agent.notes.retain(|note| !notes.contains(&note.id));
            memory.agent.back = None;
            Ok(Some(Spot {
                current: memory.current.clone(),
                selection: None,
            }))
        }
    }
}
