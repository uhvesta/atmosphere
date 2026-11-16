//! Taking a report: a [`Recording`] from start to stop, or a [`snapshot`]
//! of one moment. Both give a [`Captured`], which is scrubbed before it can
//! be written.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::change::{self, Hunk};
use crate::event::{
    ActionStep, Context, DaemonError, Event, Gesture, Interaction, Kind, LogLine, Rpc, TreeChange,
    Trees,
};

/// How much a recording keeps before it folds its oldest events into its
/// start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The most events kept.
    pub max_events: usize,
    /// Roughly the most bytes the kept events hold.
    pub max_bytes: usize,
}

impl Default for Limits {
    /// 50,000 events or about 64 MiB, whichever is reached first.
    fn default() -> Self {
        Self {
            max_events: 50_000,
            max_bytes: 64 << 20,
        }
    }
}

/// A report taken and not yet scrubbed: its context, the state, view and
/// layout at its start, the events in order, and the trees at its end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Captured {
    pub(crate) kind: Kind,
    pub(crate) context: Context,
    /// When the start trees were taken: 0, or the time of the last event
    /// folded into them.
    pub(crate) start_ms: u64,
    /// How many events were folded into the start because the recording
    /// was full.
    pub(crate) dropped: usize,
    pub(crate) start: Trees,
    pub(crate) events: Vec<Event>,
    pub(crate) end_ms: u64,
    pub(crate) end: Trees,
}

impl Captured {
    /// A recording or a snapshot.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Where it was taken.
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// The trees at its start.
    pub fn start(&self) -> &Trees {
        &self.start
    }

    /// When the start trees were taken, in milliseconds since the report
    /// started: 0 unless older events were folded into them.
    pub fn start_ms(&self) -> u64 {
        self.start_ms
    }

    /// How many of the oldest events were folded into the start because
    /// the recording reached its [`Limits`]. A report that dropped events
    /// starts mid-way, so its replay does too.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// The events, in order.
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// When it stopped, in milliseconds since it started.
    pub fn end_ms(&self) -> u64 {
        self.end_ms
    }

    /// The trees at its end.
    pub fn end(&self) -> &Trees {
        &self.end
    }
}

/// The state, view and layout of one moment, as a report: the trees, the
/// context, and no events.
pub fn snapshot(context: Context, now: Trees) -> Captured {
    let now = normal(&now);
    Captured {
        kind: Kind::Snapshot,
        context,
        start_ms: 0,
        dropped: 0,
        start: now.clone(),
        events: Vec::new(),
        end_ms: 0,
        end: now,
    }
}

/// `trees` with each line ending in one newline.
fn normal(trees: &Trees) -> Trees {
    Trees {
        state: change::normal(&trees.state),
        view: change::normal(&trees.view),
        layout: change::normal(&trees.layout),
    }
}

/// A recording in progress. Start it with the trees as they are, tell it
/// each interaction, action, tree and daemon call in order, and stop it.
///
/// It keeps at most [`Limits`] of events. When a push goes over, the
/// oldest events are folded into the start until it fits: their state,
/// view and layout changes are applied to the start trees, and their
/// interactions, calls and log lines are dropped and counted (see
/// [`Captured::dropped`]). The end of a recording, where the problem
/// usually shows, is never lost.
///
/// ```ignore
/// let mut recording = Recording::start(context, trees, Limits::default());
/// recording.interaction(120, Gesture::Click { target, button: Button::Left });
/// recording.action(121, "AddTapped", &format!("{state:#?}"));
/// recording.view(125, &view_snapshot);
/// let captured = recording.stop(4_000);
/// ```
#[derive(Debug)]
pub struct Recording {
    context: Context,
    limits: Limits,
    start_ms: u64,
    dropped: usize,
    /// The start trees, as lines, so folding an event in is a splice.
    base: Base,
    /// Each event with the number it was pushed as, so a [`Mark`] can tell
    /// what came after it even when a daemon event was placed among older
    /// ones.
    events: VecDeque<(u64, Event)>,
    next_seq: u64,
    /// The highest number among the events folded into the start, if any.
    folded_seq: Option<u64>,
    id: u64,
    bytes: usize,
    /// The trees as of the last event, which the next change is made
    /// against.
    current: Trees,
}

static RECORDINGS: AtomicU64 = AtomicU64::new(0);

/// A point in a [`Recording`] it can be rewound to, see
/// [`Recording::mark`].
#[derive(Clone, Debug)]
pub struct Mark {
    recording: u64,
    seq: u64,
    current: Trees,
}

#[derive(Debug)]
struct Base {
    state: Vec<String>,
    view: Vec<String>,
    layout: Vec<String>,
}

impl Recording {
    /// Starts recording, at time 0, with the trees as they are now.
    pub fn start(context: Context, now: Trees, limits: Limits) -> Self {
        let now = normal(&now);
        Self {
            context,
            limits,
            start_ms: 0,
            dropped: 0,
            base: Base {
                state: change::lines(&now.state),
                view: change::lines(&now.view),
                layout: change::lines(&now.layout),
            },
            events: VecDeque::new(),
            next_seq: 0,
            folded_seq: None,
            id: RECORDINGS.fetch_add(1, Ordering::Relaxed),
            bytes: 0,
            current: now,
        }
    }

    /// Where it is being taken.
    pub fn context(&self) -> &Context {
        &self.context
    }

    /// The context, to add machines and sessions as they become involved.
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }

    /// How many of the oldest events have been folded into the start so
    /// far, so the app can say the recording is full.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// Remembers where the recording is now, to [`rewind`](Self::rewind)
    /// to.
    pub fn mark(&self) -> Mark {
        Mark {
            recording: self.id,
            seq: self.next_seq,
            current: self.current.clone(),
        }
    }

    /// Undoes everything pushed since `mark`: its interactions, actions,
    /// tree changes, daemon errors and log lines, and the trees go back to
    /// what they were. Daemon calls are kept, since they happened.
    ///
    /// Returns `false`, changing nothing, when `mark` is from another
    /// recording or when the recording became full since and folded events
    /// from after the mark into its start, which can't be undone.
    pub fn rewind(&mut self, mark: Mark) -> bool {
        if mark.recording != self.id || self.folded_seq.is_some_and(|folded| folded >= mark.seq) {
            return false;
        }
        let mut freed = 0;
        self.events.retain(|(seq, event)| {
            let keep = *seq < mark.seq || matches!(event, Event::Rpc(_));
            if !keep {
                freed += event.weight();
            }
            keep
        });
        self.bytes = self.bytes.saturating_sub(freed);
        self.current = mark.current;
        true
    }

    /// Records what the person did.
    pub fn interaction(&mut self, at_ms: u64, gesture: Gesture) {
        self.push(Event::Interaction(Interaction { at_ms, gesture }));
    }

    /// Records a reducer action (as `{:?}` shows it) and the state after it
    /// (as `{:#?}` shows it).
    pub fn action(&mut self, at_ms: u64, action: &str, state: &str) {
        let state = change::normal(state);
        let change = change::diff(&self.current.state, &state);
        self.current.state = state;
        self.push(Event::Action(ActionStep {
            at_ms,
            action: action.to_owned(),
            change,
        }));
    }

    /// Records the view tree after a step. Nothing is recorded when it
    /// didn't change.
    pub fn view(&mut self, at_ms: u64, view: &str) {
        let change = change::diff(&self.current.view, view);
        if !change.is_empty() {
            self.current.view = change::normal(view);
            self.push(Event::View(TreeChange { at_ms, change }));
        }
    }

    /// Records the layout tree after a step. Nothing is recorded when it
    /// didn't change.
    pub fn layout(&mut self, at_ms: u64, layout: &str) {
        let change = change::diff(&self.current.layout, layout);
        if !change.is_empty() {
            self.current.layout = change::normal(layout);
            self.push(Event::Layout(TreeChange { at_ms, change }));
        }
    }

    /// Records a daemon call, placed at its `at_ms` among the events
    /// already recorded: a call is often told after the interactions that
    /// happened while it ran.
    pub fn rpc(&mut self, rpc: Rpc) {
        self.push_at_time(Event::Rpc(rpc));
    }

    /// Records an error a daemon reported, placed at its `at_ms`.
    pub fn daemon_error(&mut self, error: DaemonError) {
        self.push_at_time(Event::DaemonError(error));
    }

    /// Records a line of a daemon's log, placed at its `at_ms`.
    pub fn log(&mut self, line: LogLine) {
        self.push_at_time(Event::Log(line));
    }

    /// Stops at `at_ms`, with the trees as the last events left them.
    pub fn stop(self, at_ms: u64) -> Captured {
        Captured {
            kind: Kind::Recording,
            context: self.context,
            start_ms: self.start_ms,
            dropped: self.dropped,
            start: Trees {
                state: change::join(&self.base.state),
                view: change::join(&self.base.view),
                layout: change::join(&self.base.layout),
            },
            events: self.events.into_iter().map(|(_, event)| event).collect(),
            end_ms: at_ms,
            end: self.current,
        }
    }

    /// Adds an interaction, action or tree change at the end: these come
    /// from the app's own thread in the order they happened, and each tree
    /// change is made against the one before it.
    fn push(&mut self, event: Event) {
        self.bytes += event.weight();
        let seq = self.next_seq();
        self.events.push_back((seq, event));
        self.fit();
    }

    /// Adds a daemon call, error or log line after every event that
    /// happened at or before it. These change no tree, so moving them
    /// keeps every change applying.
    fn push_at_time(&mut self, event: Event) {
        self.bytes += event.weight();
        let at_ms = event.at_ms();
        let mut index = self.events.len();
        while index > 0 && self.events[index - 1].1.at_ms() > at_ms {
            index -= 1;
        }
        let seq = self.next_seq();
        self.events.insert(index, (seq, event));
        self.fit();
    }

    fn next_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// Folds the oldest events into the start until the rest fit the
    /// limits.
    fn fit(&mut self) {
        while self.events.len() > self.limits.max_events || self.bytes > self.limits.max_bytes {
            let Some((seq, oldest)) = self.events.pop_front() else {
                break;
            };
            self.folded_seq = self.folded_seq.max(Some(seq));
            self.bytes -= oldest.weight();
            self.fold(oldest);
        }
    }

    /// Moves `event` into the start trees.
    fn fold(&mut self, event: Event) {
        // A late daemon event can be older than what was folded before it.
        self.start_ms = self.start_ms.max(event.at_ms());
        self.dropped += 1;
        let (lines, hunks): (&mut Vec<String>, &[Hunk]) = match &event {
            Event::Action(step) => (&mut self.base.state, &step.change),
            Event::View(change) => (&mut self.base.view, &change.change),
            Event::Layout(change) => (&mut self.base.layout, &change.change),
            Event::Interaction(_) | Event::Rpc(_) | Event::DaemonError(_) | Event::Log(_) => return,
        };
        // Each change was made against the text the one before it left,
        // starting from the base, so it always applies.
        let applied = change::apply(lines, hunks);
        debug_assert!(applied.is_ok(), "a folded change didn't apply: {applied:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trees(n: usize) -> Trees {
        let lines = |what: &str| (0..n).map(|i| format!("{what} {i}\n")).collect::<String>();
        Trees {
            state: lines("item"),
            view: lines("ListItem"),
            layout: lines("ListItem at"),
        }
    }

    /// The start trees with every kept change applied, as a replay would
    /// rebuild them.
    fn rebuilt(captured: &Captured) -> Trees {
        let mut state = change::lines(&captured.start.state);
        let mut view = change::lines(&captured.start.view);
        let mut layout = change::lines(&captured.start.layout);
        for event in &captured.events {
            let applied = match event {
                Event::Action(step) => change::apply(&mut state, &step.change),
                Event::View(tree) => change::apply(&mut view, &tree.change),
                Event::Layout(tree) => change::apply(&mut layout, &tree.change),
                _ => Ok(()),
            };
            assert!(applied.is_ok(), "{applied:?}");
        }
        Trees {
            state: change::join(&state),
            view: change::join(&view),
            layout: change::join(&layout),
        }
    }

    #[test]
    fn a_full_recording_still_rebuilds_its_end_from_its_start() {
        let limits = Limits {
            max_events: 7,
            max_bytes: usize::MAX,
        };
        let mut recording = Recording::start(Context::default(), trees(0), limits);
        for n in 1..=20 {
            let at = n as u64 * 10;
            let now = trees(n);
            recording.action(at, "Add", &now.state);
            recording.view(at + 1, &now.view);
            recording.layout(at + 2, &now.layout);
        }
        let captured = recording.stop(500);
        assert!(captured.events.len() <= 7);
        assert_eq!(captured.dropped, 60 - captured.events.len());
        assert_eq!(captured.end, trees(20));
        assert_eq!(rebuilt(&captured), captured.end);
    }
}
