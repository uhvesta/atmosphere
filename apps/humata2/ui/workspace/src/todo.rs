//! The To-do page: one ranked list for every channel and session, quick add,
//! the filters, the folded groups and the item drawer (frames b1-1 to b9-1).
//!
//! The page reads a [`TodoState`] and sends [`TodoAction`]s through a
//! [`Send`] the shell supplies, so it does not know how the app wraps them.
//! What the page decides (which groups show and fold, the Do next label, the
//! preview line) is in small pure functions at the top, apart from the
//! drawing.

use crate::{
    theme::Look,
    widgets::{col, push_primary_shape, push_quiet_shape, row},
};
use gpui_kit::component::{
    Sizable as _,
    button::Button,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
};
use gpui_kit::{prelude::*, *};
use humata2_clients::todo::{BlockerGoneResolution, Entry, TodoHistoryEntry, TodoRunsResult};
use humata2_todo::{
    Ask, AskKind, BlockerGone, CheckpointState, Day, DueChip, Effort, FuzzyDue, HeldBackBy, Impact,
    LifecyclePrompt, Need, OwnerKind, Row, SessionSignal, SortBy, State, StuckAgent, Todo, Tone,
    Weekday, WordDiffKind, format_cost_usd,
};
use humata2_todo_feature::{
    ChannelPick, DuePick, EditField, OnlyPick, SessionPick, TodoAction as A, TodoState, UndoToast,
    ViewFilter,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// How the page sends an action: the shell wraps it into the app's own.
pub type Send = Rc<dyn Fn(A, &mut App)>;

/// A session as a chip shows it: its name and its status color.
pub struct SessionChip {
    /// The session's title.
    pub name: String,
    /// The status dot's color.
    pub dot: u32,
}

/// Looks a session up by the id an item names it by.
pub type Sessions<'a> = &'a dyn Fn(&str) -> Option<SessionChip>;

// ---- What the page decides

/// The groups of the list, in the order they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Group {
    Attention,
    InProgress,
    WatchingPr,
    Next,
    Waiting,
    Hidden,
    Closed,
}

impl Group {
    const fn bit(self) -> u32 {
        1 << self as u32
    }

    fn title(self) -> &'static str {
        match self {
            Self::Attention => "Needs attention",
            Self::InProgress => "In progress",
            Self::WatchingPr => "Watching PR",
            Self::Next => "Next",
            Self::Waiting => "Waiting on another item",
            Self::Hidden => "Hidden until a date",
            Self::Closed => "Closed",
        }
    }
}

/// A group with something in it.
pub(crate) struct Section {
    pub(crate) group: Group,
    pub(crate) rows: Vec<Row>,
}

/// Each group that has rows, in drawing order.
pub(crate) fn sections(state: &TodoState) -> Vec<Section> {
    let ranked = state.ranked();
    [
        (Group::Attention, ranked.needs_attention),
        (Group::InProgress, ranked.in_progress),
        (Group::WatchingPr, ranked.watching_pr),
        (Group::Next, ranked.next),
        (Group::Waiting, ranked.waiting),
        (Group::Hidden, ranked.hidden),
        (Group::Closed, ranked.closed),
    ]
    .into_iter()
    .filter(|(_, rows)| !rows.is_empty())
    .map(|(group, rows)| Section { group, rows })
    .collect()
}

/// A list this long folds Needs attention and In progress above the Next
/// rows (frame b8-3).
const LARGE: usize = 40;

/// Whether `group` is folded to one summary line. Waiting and Hidden fold
/// unless a channel is picked; a large list folds the two above Next.
/// `toggled` is whether the person flipped it.
pub(crate) fn folded(state: &TodoState, group: Group, toggled: bool) -> bool {
    let by_default = match group {
        Group::Waiting | Group::Hidden => state.view.filter.channel.is_none(),
        Group::Attention | Group::InProgress | Group::WatchingPr => state.shown().len() > LARGE,
        Group::Next | Group::Closed => false,
    };
    by_default != toggled
}

/// The header line: "10 open across 4 channels".
pub(crate) fn header_text(state: &TodoState) -> String {
    let (open, channels) = state.header();
    if state.items.is_empty() {
        return "no items".to_owned();
    }
    match &state.view.filter.channel {
        Some(ChannelPick::Channel(id)) => format!("{open} open in {}", state.channel_name(id)),
        Some(ChannelPick::NoChannel) => format!("{open} open with no channel"),
        None => {
            let unit = if channels == 1 { "channel" } else { "channels" };
            format!("{open} open across {channels} {unit}")
        }
    }
}

/// The Do next button: its text, and the item it would open.
pub(crate) fn do_next(state: &TodoState) -> (String, Option<String>) {
    match state.ranked().do_next {
        Some(key) => (format!("Do next · {key}"), Some(key)),
        None => ("Do next · nothing yet".to_owned(), None),
    }
}

/// The line under the quick-add field.
pub(crate) fn preview_line(state: &TodoState) -> Option<String> {
    state.quick_add.preview.as_ref().map(|p| p.text())
}

/// What the quick-add field asks for.
pub(crate) fn add_placeholder(state: &TodoState) -> String {
    match &state.view.filter.channel {
        Some(ChannelPick::Channel(id)) => {
            format!("Add an item to {}… try “fix login redirect fri #bug”", state.channel_name(id))
        }
        _ => "Add an item… try “fix login redirect fri #bug”".to_owned(),
    }
}

/// "4/5", for an item with active (non-dropped) steps.
pub(crate) fn step_progress(todo: &Todo) -> Option<String> {
    let active: Vec<_> = todo.steps.iter().filter(|s| !s.dropped).collect();
    if active.is_empty() {
        return None;
    }
    let done = active.iter().filter(|s| s.done).count();
    Some(format!("{done}/{}", active.len()))
}

fn effort_letter(effort: Effort) -> &'static str {
    match effort {
        Effort::S => "S",
        Effort::M => "M",
        Effort::L => "L",
    }
}

fn impact_word(impact: Impact) -> &'static str {
    match impact {
        Impact::Low => "low",
        Impact::Medium => "medium",
        Impact::High => "high",
    }
}

/// Human-readable label for any [`State`].
pub(crate) fn state_label(state: State) -> &'static str {
    match state {
        State::Open => "Open",
        State::InProgress => "In progress",
        State::Closed => "Closed",
        State::WontDo => "Won't do",
        State::Became => "Became",
        State::WatchingPr => "Watching PR",
    }
}

/// All five temporary sort-by options in menu order.
pub(crate) const SORT_OPTIONS: [SortBy; 5] = [
    SortBy::Priority,
    SortBy::DueSoonest,
    SortBy::MostNeeded,
    SortBy::SmallestEffort,
    SortBy::LargestImpact,
];

/// Toolbar label for the temporary sort-by switcher pill.
pub(crate) fn sort_pill_label(sort: SortBy) -> String {
    format!("Sort: {}", sort.label())
}

/// Quiet `"Hidden until <day>"` pill shown on search-matched snoozed rows.
pub(crate) fn hidden_until_pill(todo: &Todo, state: &TodoState) -> Option<String> {
    if state.view.filter.search.trim().is_empty() {
        return None;
    }
    let day = todo.hide_until?;
    (day > state.today).then(|| format!("Hidden until {}", day.short()))
}

/// Soft `"Idle Nd — resume or pause?"` inline nudge for an idle `InProgress` session.
pub(crate) fn idle_nudge(sig: &SessionSignal) -> Option<String> {
    if sig.active_turn || sig.idle_days == 0 {
        return None;
    }
    Some(format!("Idle {}d — resume or pause?", sig.idle_days))
}

/// What a keyboard triage keystroke does on the To-do page.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TriageCommand {
    Action(A),
    FocusSearch,
    FocusQuickAdd,
}

/// Maps `S` / `⌥S` on a milestone to starting a milestone session (with brief)
/// or a plain session (without brief). Returns `None` when a text input is
/// focused, the target is not a milestone, or no item is targeted.
pub(crate) fn milestone_start_shortcut(
    key: &str,
    alt: bool,
    input_focused: bool,
    target_key: Option<&str>,
    is_milestone: bool,
) -> Option<A> {
    if input_focused || !is_milestone {
        return None;
    }
    let target = target_key?;
    if !key.eq_ignore_ascii_case("s") {
        return None;
    }
    if alt {
        Some(A::StartPlainSession(target.to_owned()))
    } else {
        Some(A::StartSession(target.to_owned()))
    }
}

/// Quiet helper text shown beside the milestone primary split button.
pub(crate) fn milestone_helper_text(open_steps: usize) -> String {
    match open_steps {
        0 => "All steps done · S".to_owned(),
        1 => "Pre-fills 1 open step · S".to_owned(),
        n => format!("Pre-fills {n} open steps · S"),
    }
}

/// Maps a keystroke on the To-do page to a [`TriageCommand`], returning `None`
/// whenever a text input is focused so typing is never hijacked.
pub(crate) fn triage_command(
    key: &str,
    shift: bool,
    drawer_open: bool,
    input_focused: bool,
    focused_key: Option<&str>,
) -> Option<TriageCommand> {
    if input_focused {
        return None;
    }
    match key {
        "up" if shift => Some(TriageCommand::Action(A::ExtendSelection(-1))),
        "down" if shift => Some(TriageCommand::Action(A::ExtendSelection(1))),
        "up" if drawer_open => Some(TriageCommand::Action(A::StepDrawer(-1))),
        "down" if drawer_open => Some(TriageCommand::Action(A::StepDrawer(1))),
        "up" => Some(TriageCommand::Action(A::MoveFocus(-1))),
        "down" => Some(TriageCommand::Action(A::MoveFocus(1))),
        "enter" => focused_key.map(|k| TriageCommand::Action(A::Open(k.to_owned()))),
        "s" => Some(TriageCommand::Action(A::StartOrNextFocused)),
        "x" => Some(TriageCommand::Action(A::ToggleFocusDone)),
        "/" => Some(TriageCommand::FocusSearch),
        "n" | "N" => Some(TriageCommand::FocusQuickAdd),
        "escape" => Some(TriageCommand::Action(if drawer_open {
            A::Close
        } else {
            A::ClearSelection
        })),
        _ => None,
    }
}

/// What a click on a list row dispatches: multi-select when Shift/Cmd is held,
/// otherwise open/close the drawer.
pub(crate) fn row_click_action(key: &str, is_open: bool, shift: bool, toggle: bool) -> A {
    if shift || toggle {
        A::SelectRow {
            key: key.to_owned(),
            shift,
            toggle,
        }
    } else if is_open {
        A::Close
    } else {
        A::Open(key.to_owned())
    }
}

/// What clicking a linked session chip on a To-do row or drawer dispatches.
pub(crate) fn session_link_action(session_id: &str) -> A {
    A::OpenSession(session_id.to_owned())
}

/// What clicking a linked channel tag/name on a To-do row or drawer dispatches.
pub(crate) fn channel_link_action(channel_id: Option<&str>) -> Option<A> {
    channel_id.map(|id| A::OpenChannel(id.to_owned()))
}

/// Fields on an open item that are still unset, for subtle drawer highlights.
pub(crate) fn unset_fields(todo: &Todo) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if todo.need.is_none() {
        missing.push("Need");
    }
    if todo.due.is_none() && todo.fuzzy_due.is_none() {
        missing.push("Due");
    }
    if todo.effort.is_none() {
        missing.push("Effort");
    }
    if todo.impact.is_none() {
        missing.push("Impact");
    }
    missing
}

/// Folded `"Runs N · $X.XX"` ledger summary label for an item.
pub(crate) fn runs_ledger_label(todo: &Todo, loaded: Option<&TodoRunsResult>) -> Option<String> {
    let (count, cost) = match loaded {
        Some(res) if !res.runs.is_empty() => (res.runs.len() as u32, res.total_cost_micros),
        _ if todo.run_count > 0 => (todo.run_count, todo.total_cost_micros),
        _ => return None,
    };
    Some(match cost {
        Some(cost) => format!("Runs {count} · {}", format_cost_usd(cost)),
        None => format!("Runs {count}"),
    })
}

/// Compact close-readiness summary line if present on `todo`.
pub(crate) fn close_readiness_text(todo: &Todo) -> Option<String> {
    todo.close_readiness.as_ref().map(|cr| cr.summary_line())
}

/// The question an ask puts to the person.
pub(crate) fn ask_question(todo: &Todo, ask: &Ask) -> String {
    match &ask.kind {
        AskKind::Close => format!("Close {}?", todo.title),
        AskKind::Edit { field, .. } => format!("Change {field} on {}?", todo.title),
        AskKind::Add => format!("Add {}?", todo.title),
        AskKind::Question { question, .. } => question.clone(),
        AskKind::ActionRequest { action, .. } => action.clone(),
        AskKind::ReviewRequest { summary, round, .. } => {
            if *round > 1 {
                format!("Review (round {round}): {summary}")
            } else {
                format!("Review: {summary}")
            }
        }
    }
}

/// What the ask would change, as "before → after".
pub(crate) fn ask_change(todo: &Todo, ask: &Ask) -> Option<String> {
    match &ask.kind {
        AskKind::Close => Some(format!("State: {} → Closed", state_label(todo.state))),
        AskKind::Edit { field, before, after } => Some(format!("{field}: {before} → {after}")),
        AskKind::Add | AskKind::Question { .. } | AskKind::ActionRequest { .. } => None,
        AskKind::ReviewRequest { checkpoint: Some(cp), .. } => {
            Some(format!("Checkpoint: {}", cp.label()))
        }
        AskKind::ReviewRequest { checkpoint: None, .. } => None,
    }
}

/// The one line a folded group shows after its title.
pub(crate) fn fold_summary(state: &TodoState, section: &Section) -> String {
    let find = |key: &str| state.items.iter().map(|e| &e.todo).find(|t| t.key == key);
    match section.group {
        Group::Waiting => section
            .rows
            .iter()
            .filter_map(|r| {
                let t = find(&r.key)?;
                let on = t.depends_on.first()?;
                Some(format!("{} {} waits on {on}", t.key, t.title))
            })
            .collect::<Vec<_>>()
            .join(" · "),
        Group::Hidden => {
            let soonest = section
                .rows
                .iter()
                .filter_map(|r| find(&r.key))
                .filter_map(|t| t.hide_until.map(|d| (d, t)))
                .min_by_key(|(d, _)| *d);
            soonest.map_or_else(String::new, |(d, t)| {
                format!("Next to come back: \"{}\" on {}", t.title, d.short())
            })
        }
        Group::Attention => {
            let (mut close, mut due, mut add, mut other) = (0, 0, 0, 0);
            for t in section.rows.iter().filter_map(|r| find(&r.key)) {
                match t.ask.as_ref().map(|a| &a.kind) {
                    Some(AskKind::Close) => close += 1,
                    Some(AskKind::Edit { field, .. }) if field == "due" => due += 1,
                    Some(AskKind::Add) => add += 1,
                    _ => other += 1,
                }
            }
            [(close, "to close"), (due, "new due date"), (add, "suggested"), (other, "to edit")]
                .into_iter()
                .filter(|(n, _)| *n > 0)
                .map(|(n, w)| format!("{n} {w}"))
                .collect::<Vec<_>>()
                .join(" · ")
        }
        Group::InProgress => format!("{} on it", section.rows.len()),
        Group::WatchingPr => {
            let n = section.rows.len();
            if n == 1 { "1 PR watched".to_owned() } else { format!("{n} PRs watched") }
        }
        Group::Next | Group::Closed => String::new(),
    }
}

/// The due chip a row shows, if it has an exact or fuzzy date (fuzzy is never red).
pub(crate) fn due_chip(todo: &Todo, today: Day) -> Option<DueChip> {
    humata2_todo::chip_for(todo, today)
}

/// The due day in words beside the date field (frame 191): "tomorrow".
pub(crate) fn due_word(due: Day, today: Day) -> String {
    match today.days_until(due) {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        -1 => "yesterday".to_owned(),
        n if n > 1 => format!("in {n} days"),
        n => format!("{} days ago", -n),
    }
}

/// The day Hide until picks: next Monday.
pub(crate) fn hide_day(today: Day) -> Day {
    today.next_weekday(Weekday::Mon)
}

/// How long ago an ask was made: "20m ago".
pub(crate) fn asked_ago(minutes: u32) -> String {
    match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes}m ago"),
        60..=1439 => format!("{}h ago", minutes / 60),
        _ => format!("{}d ago", minutes / 1440),
    }
}

/// One line of the ask card: what it says, and whether it reads as a link.
pub(crate) struct AskRow {
    pub(crate) label: &'static str,
    pub(crate) text: String,
    pub(crate) link: bool,
}

/// What each evidence item on an ask card shows and opens when clicked.
pub(crate) fn evidence_links(todo: &Todo) -> Vec<(&str, Option<A>)> {
    todo.ask_detail
        .evidence
        .iter()
        .map(|ev| {
            (
                ev.text.as_str(),
                ev.link.as_ref().map(|target| A::OpenEvidence(target.clone())),
            )
        })
        .collect()
}

/// Whether an evidence link is a web page the system browser should open.
pub(crate) fn opens_in_browser(target: &str) -> bool {
    target.starts_with("https://") || target.starts_with("http://")
}

/// `send`, but a web evidence link also opens in the browser; the app still
/// hears the action.
fn opens_evidence(send: &Send) -> Send {
    let send = send.clone();
    Rc::new(move |action, cx| {
        if let A::OpenEvidence(target) = &action
            && opens_in_browser(target)
        {
            cx.open_url(target);
        }
        send(action, cx);
    })
}

/// The ask card's rows in the frames' order: Evidence, Change, Also, Asked by.
pub(crate) fn ask_rows(todo: &Todo, ask: &Ask) -> Vec<AskRow> {
    let d = &todo.ask_detail;
    let mut rows = Vec::new();
    if !d.evidence.is_empty() {
        let text = d.evidence.iter().map(|e| e.text.clone()).collect::<Vec<_>>().join(" · ");
        rows.push(AskRow { label: "Evidence", text, link: true });
    }
    let change = d
        .change
        .as_ref()
        .map(|(b, a)| format!("{b} → {a}"))
        .or_else(|| ask_change(todo, ask));
    if let Some(text) = change {
        rows.push(AskRow { label: "Change", text, link: false });
    }
    if !d.affects.is_empty() {
        rows.push(AskRow { label: "Also", text: d.affects.join(" · "), link: false });
    }
    rows.push(AskRow {
        label: "Asked by",
        text: format!("{} · {}", ask.by, asked_ago(d.asked_minutes_ago)),
        link: false,
    });
    rows
}

/// What the inline "Why?" toggle button says (NOTIFY2, Fable ruling 4).
pub(crate) fn ask_why_label(open: bool) -> &'static str {
    if open { "Hide why" } else { "Why?" }
}

/// The inline explanation shown when "Why?" is expanded on a To-do ask.
pub(crate) fn ask_why_body(ask: &Ask) -> &str {
    let reason = ask.reason.trim();
    if reason.is_empty() {
        "No reason was given."
    } else {
        reason
    }
}

/// "<agent> is waiting on this" when a session linked to `todo` is blocked
/// waiting on the ask (NOTIFY2).
pub(crate) fn ask_waiting_note(todo: &Todo, ask: &Ask) -> Option<String> {
    let blocked = todo
        .session_signals
        .iter()
        .any(|sig| matches!(sig.status.as_str(), "needs_you" | "waiting" | "blocked"));
    if !blocked {
        return None;
    }
    let agent = ask.by.split(" in ").next().unwrap_or(ask.by.as_str()).trim();
    let who = if agent.is_empty() { "Agent" } else { agent };
    Some(format!("{who} is waiting on this"))
}

/// Rows shown at first in a group other than Next; the rest wait behind
/// "Show all".
const GROUP_CAP: usize = 10;

/// How many rows of a group are drawn.
pub(crate) fn visible_rows(group: Group, total: usize, all: bool) -> usize {
    if all || matches!(group, Group::Next) {
        total
    } else {
        total.min(GROUP_CAP)
    }
}

/// The rows of the ranked list in view, 1-based and inclusive, from how far
/// the list is scrolled.
pub(crate) fn visible_range(scrolled: f32, viewport: f32, row_h: f32, total: usize) -> (usize, usize) {
    if total == 0 || row_h <= 0. {
        return (0, 0);
    }
    let first = ((scrolled.max(0.) / row_h).floor() as usize + 1).min(total);
    let last = (((scrolled.max(0.) + viewport) / row_h).ceil() as usize).clamp(first, total);
    (first, last)
}

/// The footer of a large list: "1–12 of 48 ranked".
pub(crate) fn ranked_footer(first: usize, last: usize, total: usize) -> String {
    format!("{first}–{last} of {total} ranked")
}

/// How the migration banner explains itself when "What changed" is open.
pub(crate) fn what_changed_lines(s: &humata2_todo::Summary) -> Vec<String> {
    vec![
        format!(
            "{} items from {} boards are here: {} open, {} in progress, {} closed.",
            s.items, s.boards, s.open, s.in_progress, s.closed
        ),
        "Backlog and To do became Open; In progress and In review became In progress; Done became Closed.".to_owned(),
        "Each item kept its key, history and sessions. VIP became Pin, and the order you left is the starting rank.".to_owned(),
    ]
}

/// "3 of 4 items in payments-rewrite", for the filters footer.
pub(crate) fn filter_count_text(state: &TodoState) -> String {
    let shown = state.shown().iter().filter(|t| t.state != State::Closed).count();
    let channel = state.view.filter.channel.clone();
    let of = state
        .items
        .iter()
        .map(|e| &e.todo)
        .filter(|t| t.state != State::Closed && !t.suggested)
        .filter(|t| match &channel {
            Some(ChannelPick::Channel(c)) => t.channel.as_deref() == Some(c.as_str()),
            Some(ChannelPick::NoChannel) => t.channel.is_none(),
            None => true,
        })
        .count();
    match channel {
        Some(ChannelPick::Channel(id)) => {
            format!("{shown} of {of} items in {}", state.channel_name(&id))
        }
        _ => format!("{shown} of {of} items"),
    }
}

fn entry<'a>(state: &'a TodoState, key: &str) -> Option<&'a Entry> {
    state.items.iter().find(|e| e.todo.key == key)
}

fn dark(look: Look) -> bool {
    look.content != 0xffffff
}

// ---- Local view state and inputs

/// What the page keeps between draws: the fold toggles, local popovers, and inputs.
pub struct Inputs {
    pub(crate) quick_add: Entity<InputState>,
    search: Entity<InputState>,
    pub(crate) edit: Entity<InputState>,
    step_context: Entity<TextareaState>,
    comment_input: Entity<InputState>,
    ext_block_input: Entity<InputState>,
    split_input: Entity<InputState>,
    ask_input: Entity<InputState>,
    page_focus: FocusHandle,
    toggles: Rc<Cell<u32>>,
    filters_open: Rc<Cell<bool>>,
    sort_open: Rc<Cell<bool>>,
    bulk_channel_open: Rc<Cell<bool>>,
    held_back_open: Rc<Cell<bool>>,
    runs_open: Rc<Cell<bool>>,
    history_open: Rc<Cell<bool>>,
    ask_details_open: Rc<Cell<bool>>,
    split_open: Rc<Cell<bool>>,
    split_keep_age: Rc<Cell<bool>>,
    fold_open: Rc<Cell<bool>>,
    ext_block_open: Rc<Cell<bool>>,
    milestone_start_menu_open: Rc<Cell<bool>>,
    editing: Cell<bool>,
    step_context_at: Cell<Option<usize>>,
    /// The store's text last put in quick add, search, edit and the step
    /// context, in that order. A field is written only when the store's text
    /// changes, because a field's change event arrives after the frame that
    /// follows the keystroke, and writing the older store text then would
    /// erase what was just typed.
    synced: RefCell<[String; 4]>,
    placeholder: RefCell<String>,
    scroll: ScrollHandle,
    _subscriptions: [Subscription; 4],
}

impl Inputs {
    /// Fields that send their changes through `send`.
    pub fn new<V: 'static>(send: &Send, window: &mut Window, cx: &mut Context<V>) -> Self {
        let quick_add = cx.new(|cx| InputState::new(window, cx).placeholder("Add an item…"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let edit = cx.new(|cx| InputState::new(window, cx));
        let step_context = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Context for this step. Markdown is fine.")
                .auto_grow(3, 8)
        });
        let comment_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add a comment…"));
        let ext_block_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Waiting on vendor / external reason…"));
        let split_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Titles separated by commas…"));
        let ask_input = cx.new(|cx| InputState::new(window, cx).placeholder("Write an answer or note…"));
        let (one, two, three, four) = (send.clone(), send.clone(), send.clone(), send.clone());
        let subs = [
            cx.subscribe(&quick_add, move |_, input, event: &InputEvent, cx| match event {
                InputEvent::Change => one(A::QuickAddChanged(input.read(cx).value().to_string()), cx),
                InputEvent::PressEnter { .. } => one(A::QuickAddSubmit, cx),
                _ => {}
            }),
            cx.subscribe(&search, move |_, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    two(A::SetSearch(input.read(cx).value().to_string()), cx);
                }
            }),
            cx.subscribe(&edit, move |_, input, event: &InputEvent, cx| match event {
                InputEvent::Change => three(A::EditChanged(input.read(cx).value().to_string()), cx),
                InputEvent::PressEnter { .. } => three(A::CommitEdit, cx),
                _ => {}
            }),
            // Return is a new line here; Cmd+Return saves.
            cx.subscribe(&step_context, move |_, input, event: &InputEvent, cx| match event {
                InputEvent::Change => four(A::EditChanged(input.read(cx).value().to_string()), cx),
                InputEvent::PressEnter { secondary: true, .. } => {
                    four(A::EditChanged(input.read(cx).value().to_string()), cx);
                    four(A::CommitEdit, cx);
                }
                _ => {}
            }),
        ];
        Self {
            quick_add,
            search,
            edit,
            step_context,
            comment_input,
            ext_block_input,
            split_input,
            ask_input,
            page_focus: cx.focus_handle(),
            toggles: Rc::new(Cell::new(0)),
            filters_open: Rc::new(Cell::new(false)),
            sort_open: Rc::new(Cell::new(false)),
            bulk_channel_open: Rc::new(Cell::new(false)),
            held_back_open: Rc::new(Cell::new(false)),
            runs_open: Rc::new(Cell::new(false)),
            history_open: Rc::new(Cell::new(false)),
            ask_details_open: Rc::new(Cell::new(false)),
            split_open: Rc::new(Cell::new(false)),
            split_keep_age: Rc::new(Cell::new(true)),
            fold_open: Rc::new(Cell::new(false)),
            ext_block_open: Rc::new(Cell::new(false)),
            milestone_start_menu_open: Rc::new(Cell::new(false)),
            editing: Cell::new(false),
            step_context_at: Cell::new(None),
            synced: RefCell::new(Default::default()),
            placeholder: RefCell::new(String::new()),
            scroll: ScrollHandle::new(),
            _subscriptions: subs,
        }
    }

    /// Puts the state's text into the fields that differ, and focuses the
    /// drawer field when typing into it starts.
    pub fn sync(&self, state: &TodoState, window: &mut Window, cx: &mut App) {
        let editing = state.editing.as_ref();
        let step_at = editing.and_then(|e| match e.field {
            EditField::StepContext(i) => Some(i),
            _ => None,
        });
        // The one-line edit field and the step-context box share the edit's
        // text; each shows it only while it is the one editing.
        let edit_text = editing.filter(|_| step_at.is_none()).map_or("", |e| e.text.as_str());
        let ctx_text = editing.filter(|_| step_at.is_some()).map_or("", |e| e.text.as_str());
        let mut synced = self.synced.borrow_mut();
        let one_line = [
            (&self.quick_add, state.quick_add.line.as_str()),
            (&self.search, state.view.filter.search.as_str()),
            (&self.edit, edit_text),
        ];
        for ((input, value), seen) in one_line.into_iter().zip(synced.iter_mut()) {
            if seen.as_str() != value {
                if input.read(cx).value().as_ref() != value {
                    input.update(cx, |i, cx| i.set_value(value.to_owned(), window, cx));
                }
                value.clone_into(seen);
            }
        }
        if synced[3] != ctx_text {
            if self.step_context.read(cx).value().as_ref() != ctx_text {
                let text = ctx_text.to_owned();
                self.step_context.update(cx, |i, cx| i.set_value(text, window, cx));
            }
            ctx_text.clone_into(&mut synced[3]);
        }
        drop(synced);
        if step_at.is_some() {
            // A box opens with its caret after the text, as any editor
            // does; a multi-line field's `set_value` leaves it at the start.
            if self.step_context_at.get() != step_at {
                self.step_context.update(cx, |i, cx| {
                    let end = i.text().len();
                    i.set_selected_range(end..end, cx);
                    i.focus(window, cx);
                });
            }
        } else if editing.is_some() && !self.editing.get() {
            self.edit.update(cx, |i, cx| i.focus(window, cx));
        }
        self.step_context_at.set(step_at);
        self.editing.set(editing.is_some());
        let want = add_placeholder(state);
        if *self.placeholder.borrow() != want {
            self.quick_add.update(cx, |i, cx| i.set_placeholder(want.clone(), window, cx));
            *self.placeholder.borrow_mut() = want;
        }
    }

    fn toggled(&self, group: Group) -> bool {
        self.toggles.get() & group.bit() != 0
    }
}

// ---- Tokens

fn tone(look: Look, light: u32, dark_: u32) -> Rgba {
    rgb(if dark(look) { dark_ } else { light })
}

fn soft(look: Look) -> Rgba {
    tone(look, 0x55555b, 0xc4c4ca)
}

fn amber(look: Look) -> Rgba {
    tone(look, 0xc76a00, 0xffb340)
}

fn amber_soft(look: Look) -> Rgba {
    tone(look, 0xfff4e0, 0x3b2b10)
}

fn teal(look: Look) -> Rgba {
    tone(look, 0x0f766e, 0x35c4b5)
}

fn hairline(look: Look) -> Rgba {
    tone(look, 0xf0f0f3, 0x2a2a2d)
}

fn selected_bg(look: Look) -> Rgba {
    tone(look, 0xeef4ff, 0x20304d)
}

fn muted(text: impl Into<SharedString>, look: Look) -> Div {
    div().text_color(rgb(look.secondary)).child(text.into())
}

fn small(text: impl Into<SharedString>, look: Look) -> Div {
    muted(text, look).text_size(px(12.))
}

fn pill(text: impl Into<SharedString>, bg: Rgba, fg: Rgba) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(7.))
        .py(px(1.))
        .rounded(px(5.))
        .text_size(px(12.))
        .bg(bg)
        .text_color(fg)
        .child(text.into())
}

fn tag(text: impl Into<SharedString>, look: Look) -> Div {
    pill(text, rgb(look.fill), soft(look))
}

fn outline(text: impl Into<SharedString>, color: u32) -> Div {
    div()
        .flex_shrink_0()
        .whitespace_nowrap()
        .px(px(6.))
        .rounded(px(5.))
        .text_size(px(11.5))
        .border_1()
        .border_color(rgb(color))
        .text_color(rgb(color))
        .child(text.into())
}

fn due_pill(chip: &DueChip, look: Look) -> Div {
    match chip.tone {
        Tone::Overdue => pill(
            chip.text.clone(),
            tone(look, 0xfde4e4, 0x4b1f1f),
            tone(look, 0xa51d1d, 0xff9088),
        )
        .font_weight(FontWeight::SEMIBOLD),
        Tone::Soon => pill(chip.text.clone(), amber_soft(look), tone(look, 0x8a4b00, 0xffcf85))
            .font_weight(FontWeight::MEDIUM),
        Tone::Plain => tag(chip.text.clone(), look),
    }
}

fn attention_pill(label: &str, look: Look) -> Div {
    pill(
        format!("Needs attention · {label}"),
        tone(look, 0xffe9c2, 0x5b3f0c),
        tone(look, 0x7a4300, 0xffd995),
    )
    .font_weight(FontWeight::SEMIBOLD)
}

/// A clickable element that sends `action`.
fn clickable(send: &Send, id: impl Into<ElementId>, label: &str, action: A, inner: Div) -> AnyElement {
    let send = send.clone();
    inner
        .id(id)
        .role(Role::Button)
        .aria_label(label.to_owned())
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            send(action.clone(), cx);
        })
        .test_support()
        .into_any_element()
}

fn quiet(send: &Send, id: impl Into<ElementId>, label: &str, action: A, look: Look) -> Button {
    let send = send.clone();
    push_quiet_shape(id, label.to_owned(), look).on_click(move |_, _, cx| {
        cx.stop_propagation();
        send(action.clone(), cx);
    })
}

fn primary(send: &Send, id: impl Into<ElementId>, label: &str, action: A, look: Look) -> Button {
    let send = send.clone();
    push_primary_shape(id, label.to_owned(), look).on_click(move |_, _, cx| {
        cx.stop_propagation();
        send(action.clone(), cx);
    })
}

fn link(send: &Send, id: impl Into<ElementId>, label: &str, action: A, look: Look) -> AnyElement {
    clickable(
        send,
        id,
        label,
        action,
        div().flex_shrink_0().whitespace_nowrap().text_color(rgb(look.accent)).child(label.to_owned()),
    )
}

// ---- The page

/// The whole page: header, toolbar, active filter chips, quick add, groups,
/// drawer, floating bulk action bar, and undo toast.
pub fn page(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    sessions: Sessions<'_>,
    look: Look,
) -> Div {
    let open = state.open.as_deref().and_then(|k| entry(state, k));
    let open_key = state.open.clone();
    let drawer_open = open.is_some();
    let focused_key = state.focused.clone();
    let target_is_milestone = open_key
        .as_deref()
        .or(focused_key.as_deref())
        .is_some_and(|k| state.is_milestone(k));
    let key_send = send.clone();
    let search_input = inputs.search.clone();
    let quick_add_input = inputs.quick_add.clone();
    let edit_input = inputs.edit.clone();
    let step_context_input = inputs.step_context.clone();
    let comment_input = inputs.comment_input.clone();
    let ext_block_input = inputs.ext_block_input.clone();
    let split_input = inputs.split_input.clone();
    let ask_input = inputs.ask_input.clone();
    col()
        .size_full()
        .relative()
        .track_focus(&inputs.page_focus)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            let input_focused = [
                &search_input,
                &quick_add_input,
                &edit_input,
                &comment_input,
                &ext_block_input,
                &split_input,
                &ask_input,
            ]
            .into_iter()
            .any(|inp| inp.focus_handle(cx).is_focused(window))
                || step_context_input.focus_handle(cx).is_focused(window);
            let key = event.keystroke.key.as_str();
            let shift = event.keystroke.modifiers.shift;
            let alt = event.keystroke.modifiers.alt;
            let target_key = open_key.as_deref().or(focused_key.as_deref());
            if let Some(action) =
                milestone_start_shortcut(key, alt, input_focused, target_key, target_is_milestone)
            {
                cx.stop_propagation();
                key_send(action, cx);
                return;
            }
            if let Some(cmd) =
                triage_command(key, shift, drawer_open, input_focused, focused_key.as_deref())
            {
                cx.stop_propagation();
                match cmd {
                    TriageCommand::Action(action) => key_send(action, cx),
                    TriageCommand::FocusSearch => {
                        search_input.update(cx, |i, cx| i.focus(window, cx));
                    }
                    TriageCommand::FocusQuickAdd => {
                        quick_add_input.update(cx, |i, cx| i.focus(window, cx));
                    }
                }
            }
        })
        .child(header(send, state, inputs, look))
        .child(channel_bar(send, state, look))
        .children(active_filter_chips_bar(send, state, sessions, look))
        .child(
            row()
                .flex_1()
                .min_h_0()
                .items_start()
                .child(list(send, state, inputs, sessions, look, drawer_open))
                .children(open.map(|e| {
                    col()
                        .id("todo-drawer")
                        .w(px(520.))
                        .flex_shrink_0()
                        .h_full()
                        .overflow_y_scroll()
                        .border_l_1()
                        .border_color(rgb(look.line))
                        .px(px(14.))
                        .py(px(12.))
                        .child(drawer(send, state, e, inputs, sessions, look, true))
                        .test_support()
                })),
        )
        .children((!state.selected.is_empty()).then(|| bulk_action_bar(send, state, inputs, look)))
        .children(state.undo.as_ref().map(|toast| undo_toast(send, toast, look)))
}

fn header(send: &Send, state: &TodoState, inputs: &Inputs, look: Look) -> Div {
    let (label, key) = do_next(state);
    let toggle = inputs.filters_open.clone();
    let sort_toggle = inputs.sort_open.clone();
    let filters_on = state.view.filter.due.len()
        + state.view.filter.tags.len()
        + usize::from(state.view.filter.session.is_some())
        + state.view.filter.effort.len()
        + state.view.filter.impact.len()
        + state.view.filter.need.len()
        + state.view.filter.only.len();
    row()
        .h(px(52.))
        .flex_shrink_0()
        .gap(px(10.))
        .px(px(20.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("To-do"))
        .child(small(header_text(state), look).flex_1().min_w_0().whitespace_nowrap())
        .child(
            div()
                .id("todo-search")
                .w(px(240.))
                .flex_shrink_0()
                .child(
                    Input::new(&inputs.search)
                        .small()
                        .min_h(px(26.))
                        .rounded(px(6.))
                        .bg(rgb(look.card))
                        .border_color(rgb(look.line))
                        .text_size(px(12.5)),
                )
                .test_support(),
        )
        .child(
            push_quiet_shape("todo-sort", sort_pill_label(state.sort), look).on_click(
                move |_, window, _| {
                    sort_toggle.set(!sort_toggle.get());
                    window.refresh();
                },
            ),
        )
        .child(
            push_quiet_shape(
                "todo-filters",
                if filters_on > 0 { format!("Filters · {filters_on}") } else { "Filters".to_owned() },
                look,
            )
            .on_click(move |_, window, _| {
                toggle.set(!toggle.get());
                window.refresh();
            }),
        )
        .child(match key {
            Some(_) => primary(send, "todo-do-next", &label, A::DoNext, look).into_any_element(),
            None => push_primary_shape("todo-do-next", label, look)
                .opacity(0.45)
                .into_any_element(),
        })
}

fn sort_popover(send: &Send, state: &TodoState, inputs: &Inputs, look: Look) -> impl IntoElement {
    let close = inputs.sort_open.clone();
    row()
        .id("todo-sort-popover")
        .flex_shrink_0()
        .flex_wrap()
        .gap(px(6.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
        .child(small("Sort by", look).font_weight(FontWeight::SEMIBOLD))
        .children(SORT_OPTIONS.into_iter().map(|opt| {
            let on = state.sort == opt;
            let send = send.clone();
            let close = close.clone();
            div()
                .id(SharedString::from(format!("todo-sort-{}", opt.label())))
                .role(Role::Button)
                .aria_label(opt.label())
                .cursor_pointer()
                .px(px(8.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(12.))
                .when(on, |c| c.bg(rgb(0x1c1c1e)).text_color(rgb(0xffffff)))
                .when(!on, |c| c.bg(rgb(look.fill)).text_color(soft(look)))
                .child(opt.label())
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    close.set(false);
                    send(A::SetSort(opt), cx);
                    window.refresh();
                })
                .test_support()
        }))
        .test_support()
}

fn active_filter_chips_bar(
    send: &Send,
    state: &TodoState,
    sessions: Sessions<'_>,
    look: Look,
) -> Option<impl IntoElement> {
    let chips = state.view.filter.active_chips();
    if chips.is_empty() {
        return None;
    }
    Some(
        row()
            .id("todo-active-filters")
            .flex_shrink_0()
            .flex_wrap()
            .gap(px(6.))
            .px(px(20.))
            .pb(px(6.))
            .children(chips.into_iter().enumerate().map(|(i, chip)| {
                let chip_text = chip.label_with(
                    |id| state.channel_name(id),
                    |id| sessions(id).map(|s| s.name),
                );
                let label = format!("{chip_text} ×");
                clickable(
                    send,
                    SharedString::from(format!("todo-filter-chip-{i}")),
                    &label,
                    A::RemoveFilterChip(chip),
                    outline(label.clone(), look.accent).py(px(1.)),
                )
            }))
            .child(link(send, "todo-clear-all-filters", "Clear all", A::ClearFilter, look))
            .test_support(),
    )
}

fn bulk_action_bar(send: &Send, state: &TodoState, inputs: &Inputs, look: Look) -> impl IntoElement {
    let count = state.selected.len();
    let ch_toggle = inputs.bulk_channel_open.clone();
    let ch_open = inputs.bulk_channel_open.get();
    let channels = state.all_channels();
    col()
        .id("todo-bulk-bar")
        .flex_shrink_0()
        .gap(px(6.))
        .px(px(20.))
        .py(px(8.))
        .border_t_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
        .child(
            row()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(format!("{count} selected")),
                )
                .child(quiet(send, "todo-bulk-done", "Done", A::BulkSetState(State::Closed), look))
                .child(quiet(
                    send,
                    "todo-bulk-wont-do",
                    "Won't do",
                    A::BulkSetState(State::WontDo),
                    look,
                ))
                .child(quiet(
                    send,
                    "todo-bulk-start",
                    "Start",
                    A::BulkSetState(State::InProgress),
                    look,
                ))
                .child(
                    push_quiet_shape("todo-bulk-move-channel", "Move channel", look).on_click(
                        move |_, window, _| {
                            ch_toggle.set(!ch_toggle.get());
                            window.refresh();
                        },
                    ),
                )
                .child(div().flex_1())
                .child(link(send, "todo-bulk-clear", "Clear", A::ClearSelection, look)),
        )
        .children(ch_open.then(|| {
            let close = inputs.bulk_channel_open.clone();
            row()
                .gap(px(6.))
                .flex_wrap()
                .child(small("Move to:", look))
                .children(channels.into_iter().map(|ch| {
                    let send = send.clone();
                    let close = close.clone();
                    let label = format!("# {}", ch.name);
                    let ch_id = ch.id;
                    div()
                        .id(SharedString::from(format!("todo-bulk-ch-{ch_id}")))
                        .role(Role::Button)
                        .aria_label(label.clone())
                        .cursor_pointer()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .bg(rgb(look.fill))
                        .text_size(px(12.))
                        .child(label)
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            close.set(false);
                            send(A::BulkMoveChannel(Some(ch_id.clone())), cx);
                            window.refresh();
                        })
                        .test_support()
                }))
                .child({
                    let send = send.clone();
                    let close = inputs.bulk_channel_open.clone();
                    div()
                        .id("todo-bulk-ch-none")
                        .role(Role::Button)
                        .aria_label("No channel")
                        .cursor_pointer()
                        .px(px(8.))
                        .py(px(2.))
                        .rounded(px(5.))
                        .bg(rgb(look.fill))
                        .text_size(px(12.))
                        .child("No channel")
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            close.set(false);
                            send(A::BulkMoveChannel(None), cx);
                            window.refresh();
                        })
                        .test_support()
                })
        }))
        .test_support()
}

fn undo_toast(send: &Send, toast: &UndoToast, look: Look) -> impl IntoElement {
    row()
        .id("todo-undo-toast")
        .flex_shrink_0()
        .gap(px(10.))
        .px(px(20.))
        .py(px(8.))
        .border_t_1()
        .border_color(rgb(look.line))
        .bg(tone(look, 0x1c1c1e, 0x2c2c2e))
        .text_color(rgb(0xffffff))
        .text_size(px(12.5))
        .child(div().flex_1().child(toast.message.clone()))
        .child(clickable(
            send,
            "todo-undo-btn",
            "Undo",
            A::Undo,
            div().font_weight(FontWeight::SEMIBOLD).text_color(rgb(0x6eb5ff)).child("Undo"),
        ))
        .children(toast.start_next.as_ref().map(|next| {
            let label = format!("Start next: {}", next.title);
            clickable(
                send,
                "todo-undo-start-next",
                &label,
                A::StartNextFromUndo,
                div()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .bg(rgb(0x3a3a3c))
                    .text_color(rgb(0xffffff))
                    .child(label.clone()),
            )
        }))
        .child(clickable(
            send,
            "todo-undo-dismiss",
            "Dismiss",
            A::DismissUndo,
            div().text_color(rgb(0xaeaeb2)).child("×"),
        ))
        .test_support()
}

fn channel_bar(send: &Send, state: &TodoState, look: Look) -> Div {
    let filter = &state.view.filter;
    let total: usize = state.channel_counts().iter().map(|(_, n)| n).sum();
    let chip = |id: String, text: String, on: bool, action: A| {
        let inner = div()
            .flex_shrink_0()
            .whitespace_nowrap()
            .px(px(9.))
            .py(px(3.))
            .rounded(px(6.))
            .text_size(px(12.5))
            .when(on, |c| c.bg(rgb(0x1c1c1e)).text_color(rgb(0xffffff)))
            .when(!on, |c| c.bg(rgb(look.fill)).text_color(soft(look)))
            .child(text);
        clickable(send, SharedString::from(id), "Filter by channel", action, inner)
    };
    row()
        .flex_shrink_0()
        .gap(px(6.))
        .px(px(20.))
        .py(px(10.))
        .child(chip(
            "todo-ch-all".into(),
            format!("All {total}"),
            filter.channel.is_none(),
            A::SetFilter(ViewFilter { channel: None, ..filter.clone() }),
        ))
        .children(state.channel_counts().into_iter().map(|(channel, n)| {
            let (chip_id, label, pick) = match &channel {
                Some(id) => (
                    format!("todo-ch-{id}"),
                    format!("{} {n}", state.channel_name(id)),
                    ChannelPick::Channel(id.clone()),
                ),
                None => (
                    "todo-ch-No channel".to_owned(),
                    format!("No channel {n}"),
                    ChannelPick::NoChannel,
                ),
            };
            let on = filter.channel.as_ref() == Some(&pick);
            chip(
                chip_id,
                label,
                on,
                A::SetFilter(ViewFilter { channel: Some(pick), ..filter.clone() }),
            )
        }))
        .children((!filter.due.is_empty()).then(|| {
            clickable(
                send,
                "todo-due-chip",
                "Clear the due filter",
                A::SetFilter(ViewFilter { due: Vec::new(), ..filter.clone() }),
                outline(due_filter_label(&filter.due), look.accent).py(px(2.)),
            )
        }))
        .child(div().flex_1())
        .child(clickable(
            send,
            "todo-show-closed",
            "Show closed",
            A::ShowClosed(!state.view.show_closed),
            if state.view.show_closed {
                pill("Closed shown ×", tone(look, 0xdff5f1, 0x153a36), teal(look))
            } else {
                div()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(rgb(look.line))
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child("Show closed")
            },
        ))
}

fn due_filter_label(due: &[DuePick]) -> String {
    let words: Vec<&str> = due
        .iter()
        .map(|d| match d {
            DuePick::Overdue => "overdue",
            DuePick::Next7Days => "this week",
            DuePick::AfterThisWeek => "after this week",
            DuePick::NoDate => "no date",
        })
        .collect();
    format!("Due: {} ×", words.join(" or "))
}

fn list(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    sessions: Sessions<'_>,
    look: Look,
    narrow: bool,
) -> impl IntoElement {
    let sections = sections(state);
    let sort_menu = inputs.sort_open.get().then(|| sort_popover(send, state, inputs, look));
    let filters = inputs.filters_open.get().then(|| filter_popover(send, state, inputs, sessions, look));
    let ranked = state.ranked().next.len();
    let footer = (state.shown().len() > LARGE && ranked > 0).then(|| {
        let offset = f32::from(inputs.scroll.offset().y);
        let height = f32::from(inputs.scroll.bounds().size.height);
        let (first, last) = visible_range(-offset, height, 53., ranked);
        small(ranked_footer(first, last, ranked), look).py(px(8.))
    });
    col()
        .id("todo-list")
        .track_scroll(&inputs.scroll)
        .flex_1()
        .min_w_0()
        .h_full()
        .overflow_y_scroll()
        .px(px(20.))
        .gap(px(8.))
        .children(sort_menu)
        .children(filters)
        .child(quick_add(state, inputs, look))
        .children(state.migration.as_ref().map(|n| migration_banner(send, n, state.what_changed, look)))
        .children(
            state
                .error
                .as_ref()
                .map(|e| div().text_color(rgb(crate::theme::RED)).text_size(px(12.5)).child(e.to_string())),
        )
        .when(sections.is_empty() && state.loaded, |l| l.child(empty(state, look)))
        .children(footer)
        .children(sections.into_iter().enumerate().map(|(index, section)| {
            let is_folded = folded(state, section.group, inputs.toggled(section.group));
            group(send, state, inputs, sessions, look, narrow, index == 0, section, is_folded)
        }))
}

fn quick_add(state: &TodoState, inputs: &Inputs, look: Look) -> Div {
    let preview = preview_line(state);
    col()
        .flex_shrink_0()
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(if preview.is_some() { look.accent } else { look.line }))
        .child(
            div()
                .id("todo-quick-add")
                .child(
                    Input::new(&inputs.quick_add)
                        .small()
                        .min_h(px(34.))
                        .bg(rgb(look.card))
                        .border_0()
                        .text_size(px(13.)),
                )
                .test_support(),
        )
        .children(state.quick_add.preview.as_ref().map(|p| {
            let d = &p.draft;
            row()
                .gap(px(8.))
                .px(px(12.))
                .pb(px(7.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(d.title.clone()))
                .children(d.due.map(|due| {
                    let chip = humata2_todo::chip(due, state.today);
                    due_pill(
                        &DueChip { text: format!("{} · {}", chip.text, due.short()), tone: chip.tone },
                        look,
                    )
                }))
                .children(d.tags.iter().map(|t| tag(t.clone(), look)))
                .children(d.channel.as_ref().map(|c| tag(format!("# {}", state.channel_name(c)), look)))
                .children(d.effort.map(|e| tag(effort_letter(e), look)))
                .children(d.impact.map(|i| tag(impact_word(i), look)))
                .child(div().flex_1())
                .children(preview.clone().map(|text| small(text, look)))
        }))
}

fn migration_banner(
    send: &Send,
    notice: &humata2_clients::todo::MigrationNotice,
    open: bool,
    look: Look,
) -> Div {
    let s = &notice.summary;
    let details = open.then(|| what_changed_lines(s));
    col().gap(px(6.)).child(banner_row(send, s, look)).children(details.map(|lines| {
        col().gap(px(3.)).px(px(14.)).pb(px(8.)).text_size(px(12.5)).children(
            lines.into_iter().map(|l| div().child(l)),
        )
    })).rounded(px(8.)).bg(tone(look, 0xeaf1ff, 0x1c2b46))
}

fn banner_row(send: &Send, s: &humata2_todo::Summary, look: Look) -> Div {
    row()
        .flex_shrink_0()
        .gap(px(12.))
        .px(px(14.))
        .py(px(10.))
        .child(
            div().flex_1().min_w_0().child(
                div()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Your boards became this list yesterday. "),
                    )
                    .child(format!(
                        "All {} items are here, in the order you left them: In progress, then To do, then Backlog. Nothing has a date, effort or impact yet; add them where they matter and the order starts to follow.",
                        s.items
                    )),
            ),
        )
        .child(link(send, "todo-what-changed", "What changed", A::ShowWhatChanged, look))
        .child(quiet(send, "todo-got-it", "Got it", A::DismissMigration, look))
}

fn empty(state: &TodoState, look: Look) -> Div {
    let bare = state.items.is_empty();
    col()
        .items_center()
        .gap(px(10.))
        .pt(px(60.))
        .child(
            div()
                .text_size(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(if bare { "Nothing to do yet" } else { "Nothing matches" }),
        )
        .child(
            muted(
                if bare {
                    "Type a title above and press Enter. A date, an effort and an impact are optional; the more an item has, the better the list can order itself."
                } else {
                    "Clear the filter to see everything again."
                },
                look,
            )
            .max_w(px(520.))
            .text_center(),
        )
        .children(bare.then(|| {
            muted(
                "Agents can suggest items and changes. They show here marked Needs attention, and nothing joins your list until you say yes. Pull requests waiting for your review show up by themselves.",
                look,
            )
            .max_w(px(520.))
            .text_center()
        }))
}

#[allow(clippy::too_many_arguments)]
fn group(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    sessions: Sessions<'_>,
    look: Look,
    narrow: bool,
    first: bool,
    section: Section,
    is_folded: bool,
) -> Div {
    let g = section.group;
    let count = section.rows.len();
    let toggles = inputs.toggles.clone();
    let all_bit = g.bit() << 8;
    let all = toggles.get() & all_bit != 0;
    let shown_n = visible_rows(g, count, all);
    let show_all = inputs.toggles.clone();
    let flip = move |_: &ClickEvent, window: &mut Window, _: &mut App| {
        toggles.set(toggles.get() ^ g.bit());
        window.refresh();
    };
    let folds = matches!(
        g,
        Group::Waiting | Group::Hidden | Group::Attention | Group::InProgress | Group::WatchingPr
    );
    let note = match g {
        Group::Attention => "an agent wants to change these; open one to say yes or no",
        Group::WatchingPr => "quietly watching linked PRs without holding an agent turn",
        Group::Next => "ranked for you; the order holds still while you look",
        Group::Waiting => "joins the list when what it waits on is closed",
        Group::Hidden => "each comes back on its date, without a notification",
        Group::Closed => "reopen within a week; older ones are under History",
        Group::InProgress => "",
    };
    let heading = row()
        .id(SharedString::from(format!("todo-group-{}", g.title())))
        .role(Role::Button)
        .aria_label(g.title())
        .gap(px(8.))
        .pt(px(8.))
        .pb(px(4.))
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .when(folds, |h| h.cursor_pointer().on_click(flip))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child({
                    let title = if is_folded && folds {
                        g.title().to_owned()
                    } else {
                        g.title().to_uppercase()
                    };
                    format!(
                        "{}{title} · {count}{}",
                        if !folds { "" } else if is_folded { "› " } else { "⌄ " },
                        if g == Group::Closed { " this week" } else { "" },
                    )
                }),
        )
        .child(div().child(if is_folded { fold_summary(state, &section) } else { note.to_owned() }))
        .children((!is_folded && shown_n < count).then(|| div().child(format!("{shown_n} of {count} shown"))))
        .child(div().flex_1())
        .when(first && !is_folded && !narrow, |h| {
            h.child(div().w(px(120.)).child("Effort · Impact"))
                .child(div().w(px(130.)).child("Channel"))
                .child(div().w(px(52.)))
        })
        .test_support();
    col().flex_shrink_0().child(heading).children((!is_folded).then(|| {
        col()
            .children(section.rows.iter().take(shown_n).filter_map(|r| {
                let e = entry(state, &r.key)?;
                Some(item_row(send, state, e, r, sessions, look, narrow).into_any_element())
            }))
            .children((shown_n < count).then(|| {
                div()
                    .id(SharedString::from(format!("todo-show-all {}", g.title())))
                    .role(Role::Button)
                    .aria_label("Show all")
                    .cursor_pointer()
                    .px(px(8.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(rgb(look.accent))
                    .child(format!("Show all {count}"))
                    .on_click(move |_, window, _| {
                        show_all.set(show_all.get() | all_bit);
                        window.refresh();
                    })
                    .test_support()
            }))
    }))
}

fn item_row(
    send: &Send,
    state: &TodoState,
    e: &Entry,
    r: &Row,
    sessions: Sessions<'_>,
    look: Look,
    narrow: bool,
) -> impl IntoElement {
    let t = &e.todo;
    let is_open = state.open.as_deref() == Some(t.key.as_str());
    let is_bulk_selected = state.selected.contains(&t.key);
    let is_focused = state.focused.as_deref() == Some(t.key.as_str());
    let selected = is_open || is_bulk_selected;
    let closed = t.state.is_terminal();
    let key = t.key.clone();
    let click_key = key.clone();
    let sender = send.clone();
    let reasons: Vec<String> = r.reasons.iter().map(humata2_todo::Reason::text).collect();
    let bulk_err = state.bulk_errors.get(&t.key).cloned();
    let hidden_pill = hidden_until_pill(t, state);
    let circle_action = A::SetState(key.clone(), if closed { State::Open } else { State::Closed });
    let circle = if closed {
        div()
            .size(px(22.))
            .rounded_full()
            .bg(teal(look))
            .text_color(rgb(0xffffff))
            .flex()
            .items_center()
            .justify_center()
            .child("✓")
    } else {
        div().size(px(22.)).rounded_full().border_1().border_color(tone(look, 0xc9c9cf, 0x55555a))
    };
    let circle = clickable(
        send,
        SharedString::from(format!("todo-check {key}")),
        if closed { "Reopen" } else { "Close" },
        circle_action,
        circle,
    );
    let title_line = row()
        .gap(px(8.))
        .flex_wrap()
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .when(closed, |d| d.text_color(rgb(look.secondary)))
                .child(t.title.clone()),
        )
        .children(t.ask.as_ref().filter(|_| !t.suggested).map(|a| attention_pill(a.label(), look)))
        .children(t.stuck_agent.as_ref().map(|_| attention_pill("stuck", look)))
        .children((!t.blocker_gone.is_empty()).then(|| attention_pill("blocker gone", look)))
        .children(t.lifecycle_prompts.first().map(|lp| attention_pill(&lp.title, look)))
        .children(t.suggested.then(|| {
            let label = t.ask.as_ref().map_or("add?", Ask::label);
            attention_pill(label, look)
        }))
        .children(matches!(t.state, State::WontDo | State::Became).then(|| {
            tag(state_label(t.state), look)
        }))
        .children(hidden_pill.map(|text| {
            pill(text, rgb(look.fill), rgb(look.secondary))
        }))
        .children(t.review.as_ref().map(|_| {
            pill("Pull request review", tone(look, 0xece9fd, 0x322c5c), tone(look, 0x4a2fa8, 0xcdc4ff))
                .font_weight(FontWeight::MEDIUM)
        }))
        .children(t.review.iter().flat_map(|rv| rv.repo_prs.iter()).map(|pr| tag(pr.clone(), look)))
        .children(t.review.as_ref().map(|rv| small(format!("by {}", rv.author), look)))
        .children(step_progress(t).map(|s| small(s, look)))
        .children(due_chip(t, state.today).map(|c| due_pill(&c, look)))
        .children(t.tags.iter().map(|g| {
            let active = state.view.filter.tags.iter().any(|x| x.eq_ignore_ascii_case(g));
            let chip_div = if active {
                pill(format!("#{g}"), rgb(look.accent), rgb(0xffffff))
            } else {
                tag(g.clone(), look)
            };
            clickable(
                send,
                SharedString::from(format!("todo-row-tag-{key}-{g}")),
                &format!("Filter by tag {g}"),
                A::ToggleTagChip(g.clone()),
                chip_div,
            )
        }));
    let signals_ui: Vec<AnyElement> = if t.state == State::InProgress {
        t.session_signals
            .iter()
            .enumerate()
            .map(|(idx, sig)| {
                let dot_color = if sig.active_turn {
                    teal(look)
                } else {
                    rgb(look.tertiary)
                };
                let nudge = idle_nudge(sig);
                let sig_send = send.clone();
                let sig_action = session_link_action(&sig.session);
                row()
                    .gap(px(6.))
                    .child(
                        row()
                            .id(SharedString::from(format!("todo-row-signal-{key}-{idx}")))
                            .role(Role::Link)
                            .aria_label(format!("Open session {}", sig.label))
                            .gap(px(5.))
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(5.))
                            .bg(rgb(look.fill))
                            .text_size(px(11.5))
                            .cursor_pointer()
                            .hover(move |x| x.bg(rgb(look.line)))
                            .child(div().size(px(6.)).rounded_full().bg(dot_color))
                            .child(sig.label.clone())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                sig_send(sig_action.clone(), cx);
                            })
                            .test_support(),
                    )
                    .children(nudge.map(|msg| {
                        row()
                            .gap(px(6.))
                            .child(small(msg, look))
                            .child(link(
                                send,
                                SharedString::from(format!("todo-idle-resume-{key}-{idx}")),
                                "Resume",
                                A::OpenSession(sig.session.clone()),
                                look,
                            ))
                            .child(link(
                                send,
                                SharedString::from(format!("todo-idle-pause-{key}-{idx}")),
                                "Pause",
                                A::SetState(key.clone(), State::Open),
                                look,
                            ))
                    }))
                    .into_any_element()
            })
            .collect()
    } else {
        Vec::new()
    };
    let detail = row()
        .gap(px(6.))
        .flex_wrap()
        .mt(px(3.))
        .text_size(px(12.))
        .text_color(rgb(look.secondary))
        .children(t.suggested.then(|| outline("Suggested", look.secondary).border_dashed()))
        .children(t.suggested.then(|| {
            let by = t.ask.as_ref().map_or(String::new(), |a| a.by.clone());
            small(format!("by {by} · not in your list until you say yes"), look)
        }))
        .children(
            t.sessions
                .iter()
                .filter(|_| !t.suggested)
                .filter_map(|s| sessions(s).map(|chip| (s, chip)))
                .map(|(s, chip)| {
                    session_chip(
                        send,
                        SharedString::from(format!("todo-row-session-{key}-{s}")),
                        s,
                        chip,
                        look,
                    )
                }),
        )
        .children(signals_ui)
        .children(t.watched_prs.iter().map(|pr| {
            let mut txt = pr.repo_pr.clone();
            if !pr.ci_state.is_empty() {
                txt.push_str(&format!(" · CI {}", pr.ci_state));
            }
            if pr.unresolved_comments > 0 {
                txt.push_str(&format!(" · {} comments", pr.unresolved_comments));
            }
            tag(txt, look)
        }))
        .children(reasons.into_iter().map(|text| {
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .px(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(rgb(look.line))
                .text_size(px(11.5))
                .text_color(soft(look))
                .child(text)
        }))
        .children(closed.then(|| small(closed_note(t), look)));
    let columns = (!narrow).then(|| {
        row()
            .flex_shrink_0()
            .child(
                div().w(px(120.)).text_size(px(12.)).text_color(soft(look)).child(
                    match (t.effort, t.impact) {
                        (None, None) => String::new(),
                        (e, i) => format!(
                            "{}  {}",
                            e.map_or("", effort_letter),
                            i.map_or("", impact_word)
                        ),
                    },
                ),
            )
            .child(match (t.channel.as_deref(), channel_link_action(t.channel.as_deref())) {
                (Some(channel_id), Some(action)) => {
                    let name = state.channel_name(channel_id).to_owned();
                    let ch_send = send.clone();
                    div()
                        .w(px(130.))
                        .child(
                            div()
                                .id(SharedString::from(format!("todo-row-channel-{key}")))
                                .role(Role::Link)
                                .aria_label(format!("Open channel {name}"))
                                .px(px(4.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .text_size(px(12.))
                                .text_color(rgb(look.secondary))
                                .cursor_pointer()
                                .hover(move |x| x.bg(rgb(look.fill)).text_color(rgb(look.text)))
                                .child(name)
                                .on_click(move |_, _, cx| {
                                    cx.stop_propagation();
                                    ch_send(action.clone(), cx);
                                })
                                .test_support(),
                        )
                        .into_any_element()
                }
                _ => small("No channel", look).w(px(130.)).into_any_element(),
            })
            .child(small(t.key.clone(), look).w(px(52.)).text_right())
    });
    let actions = if t.suggested {
        Some(
            row()
                .gap(px(6.))
                .child(quiet(send, SharedString::from(format!("todo-accept {key}")), "Accept", A::AcceptSuggestion(key.clone()), look))
                .child(quiet(send, SharedString::from(format!("todo-dismiss {key}")), "Dismiss", A::DismissSuggestion(key.clone()), look)),
        )
    } else if closed {
        Some(row().child(quiet(send, SharedString::from(format!("todo-reopen {key}")), "Reopen", A::SetState(key.clone(), State::Open), look)))
    } else {
        None
    };
    col()
        .child(
            row()
                .id(SharedString::from(format!("todo-row {key}")))
                .role(Role::ListItem)
                .aria_label(format!("{} {}", t.key, t.title))
                .test_support()
                .gap(px(11.))
                .px(px(8.))
                .py(px(7.))
                .min_h(px(50.))
                .rounded(px(9.))
                .cursor_pointer()
                .border_t_1()
                .border_color(hairline(look))
                .when(selected, |x| x.bg(selected_bg(look)))
                .when(is_focused && !selected, |x| x.border_1().border_color(rgb(look.accent)))
                .when(t.suggested, |x| x.border_1().border_dashed().border_color(rgb(look.line)))
                .on_click(move |event: &ClickEvent, _, cx| {
                    let mods = event.modifiers();
                    let action = row_click_action(
                        &click_key,
                        is_open,
                        mods.shift,
                        mods.platform || mods.control,
                    );
                    sender(action, cx);
                })
                .child(circle)
                .child(
                    div()
                        .w(px(14.))
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(rgb(look.tertiary))
                        .child(r.position.map(|p| p.to_string()).unwrap_or_default()),
                )
                .child(col().flex_1().min_w_0().child(title_line).child(detail))
                .children(actions)
                .children(columns),
        )
        .children(bulk_err.map(|reason| {
            row()
                .id(SharedString::from(format!("todo-bulk-err-{key}")))
                .mx(px(12.))
                .mb(px(4.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .bg(tone(look, 0xfde4e4, 0x4b1f1f))
                .text_color(tone(look, 0xa51d1d, 0xff9088))
                .text_size(px(11.5))
                .child(format!("Couldn't update {}: {reason}", t.key))
                .test_support()
        }))
}

fn session_chip(
    send: &Send,
    id: impl Into<ElementId>,
    session_id: &str,
    chip: SessionChip,
    look: Look,
) -> AnyElement {
    let send = send.clone();
    let action = session_link_action(session_id);
    let label = format!("Open session {}", chip.name);
    row()
        .id(id)
        .role(Role::Link)
        .aria_label(label)
        .gap(px(6.))
        .px(px(8.))
        .py(px(1.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .text_size(px(12.))
        .cursor_pointer()
        .hover(move |x| x.bg(rgb(look.line)))
        .child(div().size(px(8.)).rounded_full().bg(rgb(chip.dot)))
        .child(chip.name)
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            send(action.clone(), cx);
        })
        .test_support()
        .into_any_element()
}

fn closed_note(t: &Todo) -> String {
    t.closed.map_or_else(String::new, |d| format!("Closed {}", d.short()))
}

fn filter_popover(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    sessions: Sessions<'_>,
    look: Look,
) -> Div {
    let filter = &state.view.filter;
    let toggle = inputs.filters_open.clone();
    let mut tags: Vec<String> = state.items.iter().flat_map(|e| e.todo.tags.iter().cloned()).collect();
    tags.sort();
    tags.dedup();
    let option = |id: String, text: String, on: bool, action: A| {
        let inner = div()
            .px(px(8.))
            .py(px(2.))
            .rounded(px(5.))
            .text_size(px(12.))
            .when(on, |c| c.bg(rgb(0x1c1c1e)).text_color(rgb(0xffffff)))
            .when(!on, |c| c.bg(rgb(look.fill)).text_color(soft(look)))
            .child(text.clone());
        clickable(send, SharedString::from(id), &text, action, inner)
    };
    let due_options = [
        (DuePick::Overdue, "Overdue"),
        (DuePick::Next7Days, "This week"),
        (DuePick::AfterThisWeek, "After this week"),
        (DuePick::NoDate, "No date"),
    ];
    let mut session_ids: Vec<String> =
        state.items.iter().flat_map(|e| e.todo.sessions.iter().cloned()).collect();
    session_ids.sort();
    session_ids.dedup();
    col()
        .flex_shrink_0()
        .gap(px(8.))
        .p(px(14.))
        .rounded(px(10.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
        .child(small("DUE", look).font_weight(FontWeight::SEMIBOLD))
        .child(row().gap(px(6.)).children(due_options.into_iter().map(|(pick, name)| {
            let on = filter.due.contains(&pick);
            let mut due = filter.due.clone();
            if on {
                due.retain(|d| *d != pick);
            } else {
                due.push(pick);
            }
            option(format!("todo-due-{name}"), name.to_owned(), on, A::SetFilter(ViewFilter { due, ..filter.clone() }))
        })))
        .children((!tags.is_empty()).then(|| small("TAGS", look).font_weight(FontWeight::SEMIBOLD)))
        .child(row().flex_wrap().gap(px(6.)).children(tags.into_iter().map(|name| {
            let on = filter.tags.contains(&name);
            let mut picked = filter.tags.clone();
            if on {
                picked.retain(|t| *t != name);
            } else {
                picked.push(name.clone());
            }
            option(
                format!("todo-tag-{name}"),
                name,
                on,
                A::SetFilter(ViewFilter { tags: picked, ..filter.clone() }),
            )
        })))
        .child(small("MORE", look).font_weight(FontWeight::SEMIBOLD))
        .child(
            field_row("Session", look)
                .children(session_ids.into_iter().map(|id| {
                    let name = sessions(&id).map_or_else(|| id.clone(), |c| c.name);
                    let on = filter.session.as_ref() == Some(&SessionPick::Session(id.clone()));
                    option(
                        format!("todo-session-{id}"),
                        name,
                        on,
                        A::SetFilter(ViewFilter {
                            session: if on { None } else { Some(SessionPick::Session(id)) },
                            ..filter.clone()
                        }),
                    )
                }))
                .child({
                    let on = filter.session.as_ref() == Some(&SessionPick::NoneYet);
                    option(
                        "todo-session-none".to_owned(),
                        "None yet".to_owned(),
                        on,
                        A::SetFilter(ViewFilter {
                            session: if on { None } else { Some(SessionPick::NoneYet) },
                            ..filter.clone()
                        }),
                    )
                }),
        )
        .child(field_row("Effort", look).children([Effort::S, Effort::M, Effort::L].into_iter().map(|v| {
            let on = filter.effort.contains(&v);
            let mut picked = filter.effort.clone();
            if on {
                picked.retain(|x| *x != v);
            } else {
                picked.push(v);
            }
            option(
                format!("todo-f-effort-{}", effort_letter(v)),
                effort_letter(v).to_owned(),
                on,
                A::SetFilter(ViewFilter { effort: picked, ..filter.clone() }),
            )
        })))
        .child(field_row("Impact", look).children([Impact::Low, Impact::Medium, Impact::High].into_iter().map(|v| {
            let on = filter.impact.contains(&v);
            let mut picked = filter.impact.clone();
            if on {
                picked.retain(|x| *x != v);
            } else {
                picked.push(v);
            }
            let word = impact_word(v);
            option(
                format!("todo-f-impact-{word}"),
                format!("{}{}", word[..1].to_uppercase(), &word[1..]),
                on,
                A::SetFilter(ViewFilter { impact: picked, ..filter.clone() }),
            )
        })))
        .child(field_row("Only", look).children(
            [(OnlyPick::NeedsAttention, "Needs attention"), (OnlyPick::PullRequests, "Pull request reviews")]
                .into_iter()
                .map(|(pick, name)| {
                    let on = filter.only.contains(&pick);
                    let mut picked = filter.only.clone();
                    if on {
                        picked.retain(|x| *x != pick);
                    } else {
                        picked.push(pick);
                    }
                    option(
                        format!("todo-f-only-{name}"),
                        name.to_owned(),
                        on,
                        A::SetFilter(ViewFilter { only: picked, ..filter.clone() }),
                    )
                }),
        ))
        .child(
            row()
                .gap(px(8.))
                .pt(px(8.))
                .border_t_1()
                .border_color(rgb(look.line))
                .child(small(filter_count_text(state), look).flex_1())
                .child(link(send, "todo-filters-reset", "Reset", A::ClearFilter, look))
                .child(
                    push_quiet_shape("todo-filters-done", "Done", look).on_click(move |_, window, _| {
                        toggle.set(false);
                        window.refresh();
                    }),
                ),
        )
}

fn field_row(label: &'static str, look: Look) -> Div {
    row().flex_wrap().gap(px(6.)).child(small(label, look).w(px(70.)))
}

// ---- The drawer

/// An item's drawer: the ask on top, then its own fields. `close` adds the
/// close control (the page has it, the session panel has a back link).
pub(crate) fn drawer(
    send: &Send,
    state: &TodoState,
    e: &Entry,
    inputs: &Inputs,
    sessions: Sessions<'_>,
    look: Look,
    close: bool,
) -> Div {
    let t = &e.todo;
    let key = t.key.clone();
    // The session panel (no close control) draws frame 191: no key line,
    // channel, sessions, waits-on or description, Effort and Impact a row
    // each, and the footer pinned to the panel's bottom.
    let narrow = !close;
    let editing = |field: EditField| {
        state.editing.as_ref().is_some_and(|ed| ed.key == key && ed.field == field)
    };
    let edit_input = |placeholder: &'static str| {
        let _ = placeholder;
        row()
            .gap(px(8.))
            .child(
                div()
                    .id("todo-edit")
                    .flex_1()
                    .child(Input::new(&inputs.edit).small().rounded(px(6.)).bg(rgb(look.card)))
                    .test_support(),
            )
            .child(link(send, "todo-edit-cancel", "Cancel", A::CancelEdit, look))
    };
    let field = |label: &'static str, body: Div| {
        row()
            .gap(px(12.))
            .py(px(4.))
            .child(small(label, look).w(px(96.)).flex_shrink_0())
            .child(body.min_w_0())
    };
    let seg = |id: &'static str, options: Vec<(String, bool, A)>| {
        // Wraps inside a narrow panel instead of clipping its last choices.
        row().flex_wrap().min_w_0().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).children(options.into_iter().map(
            move |(name, on, action)| {
                let inner = div()
                    .px(px(if narrow { 7. } else { 10. }))
                    .py(px(2.))
                    .rounded(px(5.))
                    .text_size(px(12.))
                    .when(on, |d| d.bg(rgb(look.card)).font_weight(FontWeight::SEMIBOLD))
                    .child(name.clone());
                clickable(send, SharedString::from(format!("{id}-{name}")), &name, action, inner)
            },
        ))
    };
    let impact = || {
        seg(
            "todo-impact",
            [Impact::Low, Impact::Medium, Impact::High]
                .into_iter()
                .map(|v| {
                    let on = t.impact == Some(v);
                    let name = impact_word(v);
                    let name = format!("{}{}", name[..1].to_uppercase(), &name[1..]);
                    (name, on, A::SetImpact(key.clone(), (!on).then_some(v)))
                })
                .collect(),
        )
    };
    let due_label = t
        .due
        .map(Day::short)
        .or_else(|| t.fuzzy_due.map(|f| f.label().to_owned()))
        .unwrap_or_else(|| "No date".to_owned());
    let is_milestone = humata2_todo_feature::is_todo_milestone(t);
    let milestone_info = is_milestone.then(|| state.milestone(&t.key)).flatten();
    let made = small(format!("{} · made by you, {}", t.key, t.created.short()), look);
    let ranked_row = state
        .ranked()
        .next
        .into_iter()
        .find(|r| r.key == t.key);
    let held_back = ranked_row.as_ref().and_then(|r| r.held_back.clone());
    let rank_pos = ranked_row.as_ref().and_then(|r| r.position);
    let unset = unset_fields(t);
    let is_unset = |name: &str| t.state.is_active() && unset.contains(&name);
    let field_hint = |label: &'static str, body: Div| {
        let missing = is_unset(label);
        row()
            .gap(px(12.))
            .py(px(4.))
            .when(missing, |r| {
                r.px(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .border_dashed()
                    .border_color(tone(look, 0xe2e2e8, 0x35353a))
            })
            .child(small(label, look).w(px(96.)).flex_shrink_0())
            .child(body.min_w_0())
    };
    let held_toggle = inputs.held_back_open.clone();
    let held_open = inputs.held_back_open.get();
    let active_steps: Vec<(usize, &humata2_todo::Step)> =
        t.steps.iter().enumerate().filter(|(_, s)| !s.dropped).collect();
    let dropped_steps: Vec<(usize, &humata2_todo::Step)> =
        t.steps.iter().enumerate().filter(|(_, s)| s.dropped).collect();
    let steps_done = active_steps.iter().filter(|(_, s)| s.done).count();
    let steps_total = active_steps.len();
    let need_seg = seg(
        "todo-need",
        [Need::Idea, Need::NiceToHave, Need::Expected, Need::Critical]
            .into_iter()
            .map(|v| {
                let on = t.need == Some(v);
                (v.label().to_owned(), on, A::SetNeed(key.clone(), (!on).then_some(v)))
            })
            .collect(),
    );
    col()
        .gap(px(6.))
        .children((!narrow).then(|| {
            row()
                .gap(px(8.))
                .child(made)
                .children(is_milestone.then(|| {
                    pill(
                        "Milestone",
                        tone(look, 0xece9fd, 0x322c5c),
                        tone(look, 0x4a2fa8, 0xcdc4ff),
                    )
                    .font_weight(FontWeight::MEDIUM)
                }))
                .children(milestone_info.as_ref().map(|info| {
                    small(format!("{} of {} done", info.done_count(), info.total_count()), look)
                }))
                .child(div().flex_1())
                .children(rank_pos.map(|pos| {
                    let toggle = held_toggle.clone();
                    div()
                        .id("todo-rank-pill")
                        .role(Role::Button)
                        .aria_label("Why this rank")
                        .cursor_pointer()
                        .px(px(7.))
                        .py(px(1.))
                        .rounded(px(5.))
                        .bg(rgb(look.fill))
                        .text_size(px(11.5))
                        .text_color(rgb(look.accent))
                        .child(if held_back.is_some() {
                            format!("#{pos} · Held back?")
                        } else {
                            format!("#{pos}")
                        })
                        .on_click(move |_, window, _| {
                            toggle.set(!toggle.get());
                            window.refresh();
                        })
                        .test_support()
                }))
                .child(link(send, "todo-close", "×", A::Close, look))
        }))
        .children((held_open && held_back.is_some()).then(|| {
            held_back_popover(held_back.as_ref().expect("checked"), look)
        }))
        .child(if editing(EditField::Title) {
            edit_input("Title").into_any_element()
        } else {
            clickable(
                send,
                "todo-title",
                "Edit the title",
                A::StartEdit(key.clone(), EditField::Title),
                div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(t.title.clone()),
            )
            .into_any_element()
        })
        .children(close_readiness_text(t).map(|summary| {
            row()
                .id("todo-close-readiness")
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .bg(rgb(look.fill))
                .child(small(summary, look))
                .test_support()
        }))
        .children(t.owner.as_ref().map(|owner| owner_bar(send, state, &key, owner, look)))
        .children(
            e.ask_id
                .as_ref()
                .zip(t.ask.as_ref())
                .filter(|_| !t.suggested)
                .map(|(id, ask)| ask_card(send, state, inputs, t, id, ask, look)),
        )
        .children(t.stuck_agent.as_ref().map(|stuck| stuck_agent_card(send, &key, stuck, look)))
        .children(t.blocker_gone.iter().map(|bg| blocker_gone_card(send, &key, bg, look)))
        .children(t.lifecycle_prompts.iter().map(|lp| lifecycle_prompt_card(send, &key, lp, look)))
        .children(t.review.as_ref().filter(|_| t.state.is_active()).map(|_| {
            row().child(primary(send, "todo-start-review", "Start the review", A::StartReview(key.clone()), look))
        }))
        .child(field(
            "State",
            row()
                .gap(px(6.))
                .flex_wrap()
                .child(seg(
                    "todo-state",
                    [
                        (State::Open, "Open"),
                        (State::InProgress, "In progress"),
                        (State::Closed, "Closed"),
                        (State::WontDo, "Won't do"),
                    ]
                    .into_iter()
                    .map(|(s, n)| (n.to_owned(), t.state == s, A::SetState(key.clone(), s)))
                    .collect(),
                ))
                .children(matches!(t.state, State::Became | State::WatchingPr).then(|| {
                    pill(state_label(t.state), tone(look, 0xdff5f1, 0x153a36), teal(look))
                })),
        ))
        .children(t.wont_do_reason.as_ref().filter(|r| !r.is_empty()).map(|reason| {
            field("Reason", small(reason.clone(), look))
        }))
        .children((!narrow).then(|| checkpoint_bar(send, &key, t, look)))
        .children((!narrow).then(|| {
            field(
                "Channel",
                div().child(match (t.channel.as_deref(), channel_link_action(t.channel.as_deref())) {
                    (Some(channel_id), Some(action)) => {
                        let name = format!("# {}", state.channel_name(channel_id));
                        let ch_send = send.clone();
                        div()
                            .id(SharedString::from(format!("todo-drawer-channel-{key}")))
                            .role(Role::Link)
                            .aria_label(format!("Open channel {}", state.channel_name(channel_id)))
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(5.))
                            .cursor_pointer()
                            .hover(move |x| x.bg(rgb(look.fill)).text_color(rgb(look.text)))
                            .child(name)
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                ch_send(action.clone(), cx);
                            })
                            .test_support()
                            .into_any_element()
                    }
                    _ => div().child("No channel").into_any_element(),
                }),
            )
        }))
        .children((!narrow).then(|| field(
            "Sessions",
            row().gap(px(6.)).flex_wrap().children(t.sessions.iter().map(|s| {
                let id = SharedString::from(format!("todo-drawer-session-{key}-{s}"));
                match sessions(s) {
                    Some(chip) => session_chip(send, id, s, chip, look),
                    None => {
                        let sess_send = send.clone();
                        let action = session_link_action(s);
                        tag(s.clone(), look)
                            .id(id)
                            .role(Role::Link)
                            .aria_label(format!("Open session {s}"))
                            .cursor_pointer()
                            .hover(move |x| x.bg(rgb(look.line)))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                sess_send(action.clone(), cx);
                            })
                            .test_support()
                            .into_any_element()
                    }
                }
            })),
        )))
        .child(field_hint("Need", need_seg))
        .child(field_hint(
            "Due",
            col()
                .gap(px(4.))
                .child(if editing(EditField::Due) {
                    edit_input("fri, nov 28, or 2026-12-01").into_any_element()
                } else if narrow {
                    // One wrapping row, so an undated item leaves room for its steps.
                    row()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            row()
                                .gap(px(10.))
                                .child(clickable(
                                    send,
                                    "todo-due-edit",
                                    "Type a due date",
                                    A::StartEdit(key.clone(), EditField::Due),
                                    row()
                                        .w(px(120.))
                                        .flex_shrink(1.)
                                        .min_w_0()
                                        .px(px(8.))
                                        .py(px(4.))
                                        .rounded(px(6.))
                                        .border_1()
                                        .border_color(rgb(look.line))
                                        .bg(rgb(look.card))
                                        .child(div().flex_1().min_w_0().child(due_label.clone()))
                                        .children((t.due.is_some() || t.fuzzy_due.is_some()).then(|| {
                                            clickable(
                                                send,
                                                "todo-due-clear",
                                                "Clear",
                                                if t.due.is_some() {
                                                    A::ClearDue(key.clone())
                                                } else {
                                                    A::SetFuzzyDue(key.clone(), None)
                                                },
                                                div().text_color(rgb(look.secondary)).child("×"),
                                            )
                                        })),
                                ))
                                .children(t.due.map(|d| small(due_word(d, state.today), look))),
                        )
                        .child(
                            row()
                                .gap(px(8.))
                                .flex_none()
                                .child(link(send, "todo-due-today", "Today", A::SetDue(key.clone(), state.today), look))
                                .child(link(
                                    send,
                                    "todo-due-tomorrow",
                                    "Tomorrow",
                                    A::SetDue(key.clone(), state.today.add_days(1)),
                                    look,
                                )),
                        // No exact "Next week" here: the loose chip of that
                        // name sits just below in the narrow panel.
                        )
                        .into_any_element()
                } else {
                    row()
                        .gap(px(8.))
                        .flex_wrap()
                        .child(clickable(
                            send,
                            "todo-due-edit",
                            "Type a due date",
                            A::StartEdit(key.clone(), EditField::Due),
                            div().child(due_label),
                        ))
                        .child(link(
                            send,
                            "todo-due-today",
                            "Today",
                            A::SetDue(key.clone(), state.today),
                            look,
                        ))
                        .child(link(
                            send,
                            "todo-due-tomorrow",
                            "Tomorrow",
                            A::SetDue(key.clone(), state.today.add_days(1)),
                            look,
                        ))
                        // No exact "Next week": the loose chip of that name
                        // sits on the next line.
                        .children((t.due.is_some() || t.fuzzy_due.is_some()).then(|| {
                            link(
                                send,
                                "todo-due-clear",
                                "Clear",
                                if t.due.is_some() {
                                    A::ClearDue(key.clone())
                                } else {
                                    A::SetFuzzyDue(key.clone(), None)
                                },
                                look,
                            )
                        }))
                        .into_any_element()
                })
                .child(
                    row()
                        .gap(px(6.))
                        .flex_wrap()
                        .child(small("Loose:", look))
                        .children(
                            [FuzzyDue::InAFewDays, FuzzyDue::NextWeek, FuzzyDue::NextMonth]
                                .into_iter()
                                .map(|f| {
                                    let on = t.fuzzy_due == Some(f);
                                    let inner = div()
                                        .px(px(7.))
                                        .py(px(1.))
                                        .rounded(px(5.))
                                        .text_size(px(11.5))
                                        .when(on, |d| {
                                            d.bg(rgb(0x1c1c1e)).text_color(rgb(0xffffff))
                                        })
                                        .when(!on, |d| d.bg(rgb(look.fill)).text_color(soft(look)))
                                        .child(f.label());
                                    clickable(
                                        send,
                                        SharedString::from(format!("todo-fuzzy-{}", f.label())),
                                        f.label(),
                                        A::SetFuzzyDue(key.clone(), (!on).then_some(f)),
                                        inner,
                                    )
                                }),
                        ),
                )
                .children(state.due_error.as_ref().map(|err| {
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(crate::theme::RED))
                        .child(err.to_string())
                })),
        ))
        .child(field_hint(
            "Effort",
            row()
                .gap(px(14.))
                .flex_wrap()
                .child(seg(
                    "todo-effort",
                    [Effort::S, Effort::M, Effort::L]
                        .into_iter()
                        .map(|v| {
                            let on = t.effort == Some(v);
                            (effort_letter(v).to_owned(), on, A::SetEffort(key.clone(), (!on).then_some(v)))
                        })
                        .collect(),
                ))
                .children((!narrow).then(|| small("Impact", look)))
                .children((!narrow).then(&impact)),
        ))
        .children(narrow.then(|| field_hint("Impact", impact())))
        .child(field(
            "Tags",
            row()
                .flex_wrap()
                .gap(px(6.))
                .children(t.tags.iter().map(|g| {
                    clickable(
                        send,
                        SharedString::from(format!("todo-untag-{g}")),
                        "Remove the tag",
                        A::RemoveTag(key.clone(), g.clone()),
                        div().child(tag(g.clone(), look)),
                    )
                }))
                .child(if editing(EditField::Tag) {
                    edit_input("Tag").into_any_element()
                } else {
                    link(send, "todo-add-tag", "+ Add", A::StartEdit(key.clone(), EditField::Tag), look)
                        .into_any_element()
                }),
        ))
        .children((!narrow && !t.depends_on.is_empty()).then(|| {
            field(
                "Waits on",
                row().gap(px(6.)).flex_wrap().children(t.depends_on.iter().map(|k| {
                    clickable(
                        send,
                        SharedString::from(format!("todo-dep-{k}")),
                        &format!("Open {k}"),
                        A::Open(k.clone()),
                        pill(k.clone(), tone(look, 0xdff5f1, 0x153a36), teal(look)),
                    )
                })),
            )
        }))
        .children((!narrow).then(|| external_block_row(send, inputs, t, look)))
        .children((!narrow && !t.from_keys.is_empty()).then(|| {
            field(
                "From",
                row().gap(px(6.)).flex_wrap().children(t.from_keys.iter().map(|k| {
                    clickable(
                        send,
                        SharedString::from(format!("todo-from-{k}")),
                        &format!("Open {k}"),
                        A::Open(k.clone()),
                        tag(k.clone(), look),
                    )
                })),
            )
        }))
        .children((!narrow && !t.became_keys.is_empty()).then(|| {
            field(
                "Became",
                row().gap(px(6.)).flex_wrap().children(t.became_keys.iter().map(|k| {
                    clickable(
                        send,
                        SharedString::from(format!("todo-became-{k}")),
                        &format!("Open {k}"),
                        A::Open(k.clone()),
                        tag(k.clone(), look),
                    )
                })),
            )
        }))
        .children(t.source_anchor.as_ref().map(|anchor| {
            let label = anchor.display();
            field(
                "Source",
                row().child(link(
                    send,
                    "todo-source-anchor",
                    &label,
                    A::NavigateAnchor(anchor.clone()),
                    look,
                )),
            )
        }))
        .children(milestone_info.as_ref().map(|info| {
            col()
                .id("todo-milestone-steps")
                .mt(px(10.))
                .gap(px(4.))
                .child(
                    row()
                        .child(
                            small(
                                format!(
                                    "MILESTONE STEPS · {} OF {} DONE",
                                    info.done_count(),
                                    info.total_count()
                                ),
                                look,
                            )
                            .font_weight(FontWeight::SEMIBOLD),
                        )
                        .child(div().flex_1())
                        .child(small("in Waits-on order", look)),
                )
                .children(info.steps.iter().map(|step| {
                    let indicator = if step.done {
                        div()
                            .size(px(15.))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(teal(look))
                            .text_color(rgb(0xffffff))
                            .text_size(px(10.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("✓")
                    } else {
                        div()
                            .size(px(15.))
                            .flex_shrink_0()
                            .rounded_full()
                            .border_1()
                            .border_color(tone(look, 0xb9b9c0, 0x55555a))
                    };
                    let label = format!("{} · {}", step.id, step.title);
                    clickable(
                        send,
                        SharedString::from(format!("todo-milestone-step-{}", step.id)),
                        &label,
                        A::Open(step.id.clone()),
                        row()
                            .gap(px(8.))
                            .py(px(3.))
                            .px(px(6.))
                            .rounded(px(6.))
                            .hover(move |s| s.bg(rgb(look.fill)))
                            .child(indicator)
                            .child(
                                div()
                                    .text_size(px(12.5))
                                    .when(step.done, |d| d.text_color(rgb(look.secondary)))
                                    .child(label.clone()),
                            ),
                    )
                }))
                .test_support()
        }))
        .child(
            row()
                .mt(px(10.))
                .child(small(
                    format!("STEPS · {steps_done} OF {steps_total}"),
                    look,
                ).font_weight(FontWeight::SEMIBOLD))
                .child(div().flex_1())
                .children(ticked_by(t, narrow).map(|by| small(format!("ticked by {by}"), look))),
        )
        .children(active_steps.into_iter().map(|(i, s)| {
            let total_len = t.steps.len();
            let boxed = div()
                .size(px(15.))
                .flex_shrink_0()
                .rounded(px(4.))
                .border_1()
                .border_color(if s.done { teal(look) } else { tone(look, 0xb9b9c0, 0x55555a) })
                .when(s.done, |b| b.bg(teal(look)).text_color(rgb(0xffffff)).text_size(px(10.)).child("✓"));
            let row_group = SharedString::from(format!("todo-step-row-{i}"));
            let has_context = s.markdown_context.as_ref().is_some_and(|m| !m.is_empty());
            let editing_context = editing(EditField::StepContext(i));
            col()
                .py(px(3.))
                .gap(px(2.))
                .child(
                    row()
                        .group(row_group.clone())
                        .gap(px(8.))
                        .child(clickable(
                            send,
                            SharedString::from(format!("todo-step-{i}")),
                            &s.text,
                            A::ToggleStep(key.clone(), i),
                            row().gap(px(9.)).child(boxed).child(
                                div()
                                    .when(s.done, |d| d.text_color(rgb(look.secondary)))
                                    .child(s.text.clone()),
                            ),
                        ))
                        .child(div().flex_1())
                        .children((!narrow && !has_context && !s.dropped).then(|| {
                            div()
                                .id(SharedString::from(format!("todo-step-context-add-wrap-{i}")))
                                .opacity(0.)
                                .group_hover(row_group.clone(), |style| style.opacity(1.))
                                .child(link(
                                    send,
                                    SharedString::from(format!("todo-step-context-add-{i}")),
                                    "+ Context",
                                    A::StartEdit(key.clone(), EditField::StepContext(i)),
                                    look,
                                ))
                        }))
                        .children((!narrow && i > 0).then(|| {
                            link(
                                send,
                                SharedString::from(format!("todo-step-up-{i}")),
                                "↑",
                                A::MoveStep(key.clone(), i, i - 1),
                                look,
                            )
                        }))
                        .children((!narrow && i + 1 < total_len).then(|| {
                            link(
                                send,
                                SharedString::from(format!("todo-step-down-{i}")),
                                "↓",
                                A::MoveStep(key.clone(), i, i + 1),
                                look,
                            )
                        }))
                        .children((!narrow).then(|| {
                            link(
                                send,
                                SharedString::from(format!("todo-step-drop-{i}")),
                                "Drop",
                                A::DropStep(key.clone(), i),
                                look,
                            )
                        })),
                )
                .children(editing_context.then(|| step_context_editor(send, inputs, has_context, look)))
                .children(s.markdown_context.as_ref().filter(|m| !m.is_empty() && !editing_context).map(|md| {
                    let line = div()
                        .pl(px(24.))
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child(md.clone());
                    if narrow {
                        return line.into_any_element();
                    }
                    let send = send.clone();
                    let action = A::StartEdit(key.clone(), EditField::StepContext(i));
                    line.id(SharedString::from(format!("todo-step-context-{i}")))
                        .role(Role::Button)
                        .aria_label("Edit the step's context")
                        .cursor_pointer()
                        .hover(|style| style.text_color(soft(look)))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            send(action.clone(), cx);
                        })
                        .test_support()
                        .into_any_element()
                }))
                .children(s.source_anchor.as_ref().map(|anchor| {
                    let label = anchor.display();
                    div().pl(px(24.)).text_size(px(11.5)).child(link(
                        send,
                        SharedString::from(format!("todo-step-anchor-{i}")),
                        &label,
                        A::NavigateAnchor(anchor.clone()),
                        look,
                    ))
                }))
                .children(s.from_key.as_ref().map(|fk| {
                    div().pl(px(24.)).text_size(px(11.5)).child(link(
                        send,
                        SharedString::from(format!("todo-step-from-{i}")),
                        &format!("From {fk}"),
                        A::Open(fk.clone()),
                        look,
                    ))
                }))
        }))
        .children(dropped_steps.into_iter().map(|(i, s)| {
            row()
                .gap(px(8.))
                .py(px(2.))
                .pl(px(24.))
                .child(small(format!("Dropped: {}", s.text), look).flex_1())
                .child(link(
                    send,
                    SharedString::from(format!("todo-step-restore-{i}")),
                    "Restore",
                    A::RestoreStep(key.clone(), i),
                    look,
                ))
        }))
        .child(if editing(EditField::Step) {
            edit_input("Step").into_any_element()
        } else {
            clickable(
                send,
                "todo-add-step",
                "Add a step…",
                A::StartEdit(key.clone(), EditField::Step),
                row()
                    .gap(px(9.))
                    .py(px(3.))
                    .text_color(rgb(look.secondary))
                    .child(
                        div()
                            .size(px(15.))
                            .flex_shrink_0()
                            .rounded(px(4.))
                            .border_1()
                            .border_dashed()
                            .border_color(tone(look, 0xb9b9c0, 0x55555a)),
                    )
                    .child("Add a step…"),
            )
            .into_any_element()
        })
        .children((!narrow).then(|| {
            row().mt(px(10.)).child(
                small("DESCRIPTION", look).font_weight(FontWeight::SEMIBOLD),
            )
        }))
        .children((!narrow).then(|| {
            if editing(EditField::Description) {
                edit_input("Description").into_any_element()
            } else {
                clickable(
                    send,
                    "todo-description",
                    "Edit the description",
                    A::StartEdit(key.clone(), EditField::Description),
                    div().text_color(soft(look)).child(if t.description.is_empty() {
                        "Add a description…".to_owned()
                    } else {
                        t.description.clone()
                    }),
                )
                .into_any_element()
            }
        }))
        .children((!narrow).then(|| comments_section(send, inputs, t, look)))
        .children((!narrow).then(|| split_fold_section(send, state, inputs, t, look)))
        .children((!narrow).then(|| runs_ledger_section(send, state, inputs, t, look)).flatten())
        .children((!narrow).then(|| history_section(send, state, inputs, t, look)))
        .children((!narrow).then(|| {
            footer_inner(send, state, t, Some(inputs), look, true).mt(px(12.)).pt(px(10.))
        }))
}

/// The box that edits a step's Markdown context, under the step's row
/// (frames t2-1 and t2-2).
fn step_context_editor(send: &Send, inputs: &Inputs, had_context: bool, look: Look) -> Div {
    let hint = if had_context {
        "⌘↩ saves · Esc cancels · empty removes it"
    } else {
        "⌘↩ saves · Esc cancels"
    };
    let cancel = send.clone();
    col()
        // Escape is the app's Dismiss, which a key binding sends before any
        // key listener runs, and the Shell's Dismiss closes the drawer. On
        // its way up from the box it cancels only the edit.
        .on_action(move |_: &humata2_ui_core::keys::Dismiss, _, cx| cancel(A::CancelEdit, cx))
        .pl(px(24.))
        .mt(px(4.))
        .mb(px(4.))
        .child(
            div()
                .id("todo-step-context-edit")
                .text_size(px(12.))
                .child(
                    Textarea::new(&inputs.step_context)
                        .small()
                        .rounded(px(6.))
                        .bg(rgb(look.card))
                        .text_size(px(12.)),
                )
                .test_support(),
        )
        .child(
            row()
                .mt(px(6.))
                .gap(px(10.))
                .child(primary(send, "todo-step-context-save", "Save", A::CommitEdit, look).small())
                .child(link(send, "todo-step-context-cancel", "Cancel", A::CancelEdit, look))
                .child(div().flex_1())
                .child(div().text_size(px(11.5)).text_color(rgb(look.secondary)).child(hint)),
        )
}

/// What the drawer's start-session button says (frames b1-2 and b4-2).
pub(crate) fn start_session_label(t: &Todo) -> &'static str {
    if t.sessions.is_empty() || t.ask_detail.session_ended {
        "Start a session on this"
    } else {
        "Start another session on this"
    }
}

/// The drawer's footer. On the page (`start_session == true`, frames b1-2 and
/// b4-2), a button on the left starts a new session for the item (except on
/// suggested and pull-request review items) and History · Pin · Hide until…
/// sits on the right; in the session panel (`start_session == false`, frame
/// 191), only History · Pin · Hide until… is drawn.
pub(crate) fn footer(
    send: &Send,
    state: &TodoState,
    t: &Todo,
    look: Look,
    start_session: bool,
) -> Div {
    footer_inner(send, state, t, None, look, start_session)
}

fn footer_inner(
    send: &Send,
    state: &TodoState,
    t: &Todo,
    inputs: Option<&Inputs>,
    look: Look,
    start_session: bool,
) -> Div {
    let key = &t.key;
    let is_milestone = humata2_todo_feature::is_todo_milestone(t);
    let open_steps = if is_milestone {
        state.milestone(key).map_or(0, |m| m.open_steps().len())
    } else {
        0
    };
    let menu_cell = inputs.map(|i| i.milestone_start_menu_open.clone());
    let menu_open = menu_cell.as_ref().is_some_and(|c| c.get());
    let start: Option<AnyElement> = (start_session && !t.suggested && t.review.is_none()).then(|| {
        if is_milestone {
            let toggle_cell = menu_cell.clone();
            let plain_cell = menu_cell.clone();
            let send_plain = send.clone();
            let plain_key = key.clone();
            col()
                .gap(px(4.))
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            row()
                                .gap(px(1.))
                                .child(
                                    primary(
                                        send,
                                        "todo-start-milestone",
                                        "Start milestone session",
                                        A::StartSession(key.clone()),
                                        look,
                                    )
                                    .rounded_r(px(0.)),
                                )
                                .child(
                                    push_primary_shape("todo-start-milestone-menu", "⌄", look)
                                        .rounded_l(px(0.))
                                        .on_click(move |_, window, cx| {
                                            cx.stop_propagation();
                                            if let Some(cell) = &toggle_cell {
                                                cell.set(!cell.get());
                                                window.refresh();
                                            }
                                        }),
                                ),
                        )
                        .child(small(milestone_helper_text(open_steps), look)),
                )
                .children(menu_open.then(|| {
                    div()
                        .id("todo-start-milestone-plain")
                        .role(Role::Button)
                        .aria_label("Plain start · ⌥S")
                        .cursor_pointer()
                        .px(px(10.))
                        .py(px(5.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .bg(rgb(look.card))
                        .text_size(px(12.))
                        .hover(move |s| s.bg(rgb(look.fill)))
                        .child("Plain start · ⌥S")
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(cell) = &plain_cell {
                                cell.set(false);
                            }
                            send_plain(A::StartPlainSession(plain_key.clone()), cx);
                            window.refresh();
                        })
                        .test_support()
                }))
                .into_any_element()
        } else {
            quiet(
                send,
                "todo-start-session",
                start_session_label(t),
                A::StartSession(key.clone()),
                look,
            )
            .into_any_element()
        }
    });
    let history = link(send, "todo-history", "History", A::ShowHistory(key.clone()), look);
    let pin = if t.pin.is_some() {
        link(send, "todo-pin", "Unpin", A::Unpin(key.clone()), look)
    } else {
        link(send, "todo-pin", "Pin", A::Pin(key.clone()), look)
    };
    let hide = if t.hide_until.is_some() {
        link(send, "todo-hide", "Stop hiding", A::HideUntil(key.clone(), None), look)
    } else {
        link(
            send,
            "todo-hide",
            "Hide until…",
            A::HideUntil(key.clone(), Some(hide_day(state.today))),
            look,
        )
    };
    let base = row()
        .gap(px(8.))
        .border_t_1()
        .border_color(rgb(look.line));
    match start {
        Some(button) => base
            .child(button)
            .child(div().flex_1())
            .child(history)
            .child(pin)
            .child(hide),
        None => base
            .child(history)
            .child(div().flex_1())
            .child(pin)
            .child(hide),
    }
}

/// Who ticked the steps done: the agent that asks about the item, "claude"
/// rather than "claude in this session" in the session's own panel.
fn ticked_by(t: &Todo, narrow: bool) -> Option<String> {
    let ask = t.ask.as_ref()?;
    if !t.steps.iter().any(|s| s.done) {
        return None;
    }
    let by = ask.by.as_str();
    Some(if narrow { by.split(" in ").next().unwrap_or(by) } else { by }.to_owned())
}

fn held_back_popover(hb: &HeldBackBy, look: Look) -> impl IntoElement {
    col()
        .id("todo-held-back-popover")
        .gap(px(4.))
        .p(px(10.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
        .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child("Held back by"))
        .children((!hb.summary.is_empty()).then(|| small(hb.summary.clone(), look)))
        .children(hb.factors.iter().map(|f| small(format!("• {f}"), look)))
        .test_support()
}

fn owner_bar(
    send: &Send,
    state: &TodoState,
    key: &str,
    owner: &humata2_todo::TodoOwner,
    look: Look,
) -> Div {
    let confirming = state.confirm_take_back.as_deref() == Some(key);
    let is_agent = owner.kind == OwnerKind::Agent;
    col()
        .gap(px(6.))
        .child(
            row()
                .id("todo-owner-bar")
                .gap(px(8.))
                .px(px(8.))
                .py(px(4.))
                .rounded(px(6.))
                .bg(rgb(look.fill))
                .child(small(format!("Owner: {}", owner.label), look).flex_1())
                .children((is_agent && !confirming).then(|| {
                    quiet(
                        send,
                        "todo-take-back",
                        "Take back",
                        A::RequestTakeBack(key.to_owned()),
                        look,
                    )
                }))
                .test_support(),
        )
        .children(confirming.then(|| {
            row()
                .id("todo-take-back-confirm")
                .gap(px(8.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(amber_soft(look))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .child(format!("Take ownership back from {}?", owner.label)),
                )
                .child(primary(
                    send,
                    "todo-take-back-yes",
                    "Take back",
                    A::ConfirmTakeBack(key.to_owned()),
                    look,
                ))
                .child(quiet(send, "todo-take-back-cancel", "Cancel", A::CancelTakeBack, look))
                .test_support()
        }))
}

fn checkpoint_bar(send: &Send, key: &str, t: &Todo, look: Look) -> impl IntoElement {
    let stages = [
        CheckpointState::WithContext,
        CheckpointState::Defined,
        CheckpointState::Done,
    ];
    let review_ask_id = match (&t.ask, state_review_flag(t)) {
        (Some(ask), true) => matches!(ask.kind, AskKind::ReviewRequest { .. }),
        _ => false,
    };
    row()
        .id("todo-checkpoint-bar")
        .gap(px(6.))
        .py(px(4.))
        .flex_wrap()
        .child(small("Checkpoint", look).w(px(96.)).flex_shrink_0())
        .children(stages.into_iter().enumerate().map(|(i, stage)| {
            let on = t.checkpoint == Some(stage);
            let reached = t.checkpoint.is_some_and(|cur| cur >= stage);
            let inner = div()
                .px(px(8.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(11.5))
                .when(on, |d| d.bg(teal(look)).text_color(rgb(0xffffff)).font_weight(FontWeight::SEMIBOLD))
                .when(!on && reached, |d| {
                    d.bg(tone(look, 0xdff5f1, 0x153a36)).text_color(teal(look))
                })
                .when(!reached, |d| d.bg(rgb(look.fill)).text_color(soft(look)))
                .child(stage.label());
            row()
                .gap(px(4.))
                .children((i > 0).then(|| small("→", look)))
                .child(clickable(
                    send,
                    SharedString::from(format!("todo-checkpoint-{i}")),
                    stage.label(),
                    A::SetCheckpoint(key.to_owned(), (!on).then_some(stage)),
                    inner,
                ))
        }))
        .children((review_ask_id || t.checkpoint.is_some()).then(|| {
            let bounce_to = match t.checkpoint {
                Some(CheckpointState::Done) => Some(CheckpointState::Defined),
                Some(CheckpointState::Defined) => Some(CheckpointState::WithContext),
                _ => None,
            };
            link(
                send,
                "todo-checkpoint-changes-asked",
                "Changes asked…",
                A::SetCheckpoint(key.to_owned(), bounce_to),
                look,
            )
        }))
        .test_support()
}

fn state_review_flag(t: &Todo) -> bool {
    t.ask.as_ref().is_some_and(|a| matches!(a.kind, AskKind::ReviewRequest { .. }))
}

fn external_block_row(send: &Send, inputs: &Inputs, t: &Todo, look: Look) -> Div {
    let key = t.key.clone();
    let open_toggle = inputs.ext_block_open.clone();
    let is_open = inputs.ext_block_open.get();
    row()
        .gap(px(12.))
        .py(px(4.))
        .child(small("External", look).w(px(96.)).flex_shrink_0())
        .child(if let Some(eb) = &t.external_block {
            row()
                .gap(px(8.))
                .flex_1()
                .min_w_0()
                .child(small(format!("Waiting: {}", eb.reason), look).flex_1())
                .child(link(
                    send,
                    "todo-ext-block-clear",
                    "Clear",
                    A::RemoveExternalBlock(key),
                    look,
                ))
                .into_any_element()
        } else if is_open {
            let send = send.clone();
            let input = inputs.ext_block_input.clone();
            let close = inputs.ext_block_open.clone();
            row()
                .gap(px(6.))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .id("todo-ext-block-input")
                        .flex_1()
                        .child(Input::new(&inputs.ext_block_input).small().rounded(px(6.)).bg(rgb(look.card)))
                        .test_support(),
                )
                .child(
                    push_quiet_shape("todo-ext-block-save", "Set", look).on_click(
                        move |_, window, cx| {
                            let reason = input.read(cx).value().trim().to_owned();
                            if !reason.is_empty() {
                                send(A::AddExternalBlock(key.clone(), reason), cx);
                                input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
                                close.set(false);
                                window.refresh();
                            }
                        },
                    ),
                )
                .into_any_element()
        } else {
            div()
                .id("todo-ext-block-add")
                .role(Role::Button)
                .aria_label("Add external block")
                .cursor_pointer()
                .text_size(px(12.))
                .text_color(rgb(look.accent))
                .child("+ Add external block")
                .on_click(move |_, window, _| {
                    open_toggle.set(true);
                    window.refresh();
                })
                .test_support()
                .into_any_element()
        })
}

fn comments_section(send: &Send, inputs: &Inputs, t: &Todo, look: Look) -> Div {
    let key = t.key.clone();
    let send_c = send.clone();
    let input = inputs.comment_input.clone();
    col()
        .mt(px(10.))
        .gap(px(6.))
        .child(
            small(format!("COMMENTS · {}", t.comments.len()), look)
                .font_weight(FontWeight::SEMIBOLD),
        )
        .children(t.comments.iter().map(|c| {
            col()
                .p(px(8.))
                .rounded(px(6.))
                .bg(rgb(look.fill))
                .gap(px(2.))
                .child(
                    row()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(11.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(c.author.clone()),
                        )
                        .children(c.created.map(|d| small(d.short(), look)))
                        .children(c.step_id.as_ref().map(|sid| tag(format!("step {sid}"), look))),
                )
                .child(div().text_size(px(12.)).child(c.body.clone()))
        }))
        .child(
            row()
                .gap(px(6.))
                .child(
                    div()
                        .id("todo-comment-input")
                        .flex_1()
                        .child(Input::new(&inputs.comment_input).small().rounded(px(6.)).bg(rgb(look.card)))
                        .test_support(),
                )
                .child(
                    push_quiet_shape("todo-comment-submit", "Comment", look).on_click(
                        move |_, window, cx| {
                            let body = input.read(cx).value().trim().to_owned();
                            if !body.is_empty() {
                                send_c(
                                    A::AddComment {
                                        key: key.clone(),
                                        body,
                                        step_id: None,
                                    },
                                    cx,
                                );
                                input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
                                window.refresh();
                            }
                        },
                    ),
                ),
        )
}

fn split_fold_section(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    t: &Todo,
    look: Look,
) -> Div {
    let key = t.key.clone();
    let split_toggle = inputs.split_open.clone();
    let fold_toggle = inputs.fold_open.clone();
    let split_open = inputs.split_open.get();
    let fold_open = inputs.fold_open.get();
    let keep_age = inputs.split_keep_age.get();
    let keep_age_cell = inputs.split_keep_age.clone();
    col()
        .mt(px(8.))
        .gap(px(6.))
        .child(
            row()
                .gap(px(12.))
                .child(
                    div()
                        .id("todo-split-toggle")
                        .role(Role::Button)
                        .aria_label("Split into tasks")
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(look.accent))
                        .child("Split…")
                        .on_click(move |_, window, _| {
                            split_toggle.set(!split_toggle.get());
                            window.refresh();
                        })
                        .test_support(),
                )
                .child(
                    div()
                        .id("todo-fold-toggle")
                        .role(Role::Button)
                        .aria_label("Fold into step")
                        .cursor_pointer()
                        .text_size(px(12.))
                        .text_color(rgb(look.accent))
                        .child("Fold into step…")
                        .on_click(move |_, window, _| {
                            fold_toggle.set(!fold_toggle.get());
                            window.refresh();
                        })
                        .test_support(),
                ),
        )
        .children(split_open.then(|| {
            let send = send.clone();
            let input = inputs.split_input.clone();
            let close = inputs.split_open.clone();
            let k = key.clone();
            col()
                .gap(px(6.))
                .p(px(8.))
                .rounded(px(8.))
                .bg(rgb(look.fill))
                .child(
                    div()
                        .id("todo-split-input")
                        .child(Input::new(&inputs.split_input).small().rounded(px(6.)).bg(rgb(look.card)))
                        .test_support(),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            div()
                                .id("todo-split-keep-age")
                                .role(Role::Button)
                                .aria_label("Keep age")
                                .cursor_pointer()
                                .text_size(px(11.5))
                                .text_color(rgb(look.secondary))
                                .child(if keep_age { "☑ Keep age" } else { "☐ Fresh age" })
                                .on_click(move |_, window, _| {
                                    keep_age_cell.set(!keep_age_cell.get());
                                    window.refresh();
                                })
                                .test_support(),
                        )
                        .child(div().flex_1())
                        .child(
                            push_primary_shape("todo-split-submit", "Split", look).on_click(
                                move |_, window, cx| {
                                    let titles: Vec<String> = input
                                        .read(cx)
                                        .value()
                                        .split(',')
                                        .map(|s| s.trim().to_owned())
                                        .filter(|s| !s.is_empty())
                                        .collect();
                                    if !titles.is_empty() {
                                        send(
                                            A::SplitItem {
                                                key: k.clone(),
                                                titles,
                                                keep_age,
                                            },
                                            cx,
                                        );
                                        input.update(cx, |i, cx| {
                                            i.set_value(String::new(), window, cx)
                                        });
                                        close.set(false);
                                        window.refresh();
                                    }
                                },
                            ),
                        ),
                )
        }))
        .children(fold_open.then(|| {
            let targets: Vec<(String, String)> = state
                .items
                .iter()
                .map(|e| &e.todo)
                .filter(|other| other.key != key && other.state.is_active() && !other.key.is_empty())
                .take(8)
                .map(|other| (other.key.clone(), other.title.clone()))
                .collect();
            let close = inputs.fold_open.clone();
            col()
                .gap(px(4.))
                .p(px(8.))
                .rounded(px(8.))
                .bg(rgb(look.fill))
                .child(small("Fold this item as a step into:", look))
                .children(targets.into_iter().map(|(tk, tt)| {
                    let send = send.clone();
                    let close = close.clone();
                    let src = key.clone();
                    let label = format!("{tk} · {tt}");
                    div()
                        .id(SharedString::from(format!("todo-fold-target-{tk}")))
                        .role(Role::Button)
                        .aria_label(label.clone())
                        .cursor_pointer()
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(5.))
                        .bg(rgb(look.card))
                        .text_size(px(12.))
                        .child(label)
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            send(
                                A::FoldIntoStep {
                                    source_key: src.clone(),
                                    target_key: tk.clone(),
                                },
                                cx,
                            );
                            close.set(false);
                            window.refresh();
                        })
                        .test_support()
                }))
        }))
}

fn runs_ledger_section(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    t: &Todo,
    look: Look,
) -> Option<impl IntoElement> {
    let loaded = state.runs.get(&t.key);
    let label = runs_ledger_label(t, loaded)?;
    let open = inputs.runs_open.get();
    let toggle = inputs.runs_open.clone();
    let send_load = send.clone();
    let key = t.key.clone();
    Some(
        col()
            .id("todo-runs-ledger")
            .mt(px(8.))
            .gap(px(4.))
            .child(
                row()
                    .id("todo-runs-toggle")
                    .role(Role::Button)
                    .aria_label(label.clone())
                    .cursor_pointer()
                    .gap(px(6.))
                    .child(small(
                        format!("{} {label}", if open { "⌄" } else { "›" }),
                        look,
                    ).font_weight(FontWeight::SEMIBOLD))
                    .on_click(move |_, window, cx| {
                        let next = !toggle.get();
                        toggle.set(next);
                        if next {
                            send_load(A::LoadRuns(key.clone()), cx);
                        }
                        window.refresh();
                    })
                    .test_support(),
            )
            .children(open.then(|| {
                let runs = loaded.map(|r| r.runs.as_slice()).unwrap_or_default();
                col().gap(px(4.)).children(runs.iter().enumerate().map(|(i, run)| {
                    row()
                        .gap(px(8.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .bg(rgb(look.fill))
                        .text_size(px(11.5))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(run.tool.clone()))
                        .child(small(run.model.clone(), look))
                        .child(small(run.outcome.clone(), look).flex_1())
                        .children(run.links.iter().enumerate().map(|(li, rl)| {
                            link(
                                &opens_evidence(send),
                                SharedString::from(format!("todo-run-link-{i}-{li}")),
                                &rl.label,
                                A::OpenEvidence(rl.target.clone()),
                                look,
                            )
                        }))
                        .children(run.cost_micros.map(|c| small(format_cost_usd(c), look)))
                }))
            }))
            .test_support(),
    )
}

fn history_section(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    t: &Todo,
    look: Look,
) -> impl IntoElement {
    let open = inputs.history_open.get();
    let toggle = inputs.history_open.clone();
    let send_load = send.clone();
    let key = t.key.clone();
    let entries = state.history.get(&t.key).map(Vec::as_slice).unwrap_or_default();
    col()
        .id("todo-history-inline")
        .mt(px(8.))
        .gap(px(4.))
        .child(
            row()
                .id("todo-history-toggle")
                .role(Role::Button)
                .aria_label("Toggle history")
                .cursor_pointer()
                .gap(px(6.))
                .child(
                    small(format!("{} HISTORY", if open { "⌄" } else { "›" }), look)
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .on_click(move |_, window, cx| {
                    let next = !toggle.get();
                    toggle.set(next);
                    if next {
                        send_load(A::LoadHistory(key.clone()), cx);
                    }
                    window.refresh();
                })
                .test_support(),
        )
        .children(open.then(|| {
            if entries.is_empty() {
                col().child(small("No history entries yet.", look))
            } else {
                col().gap(px(4.)).children(entries.iter().map(|entry| history_entry_row(entry, look)))
            }
        }))
        .test_support()
}

fn history_entry_row(entry: &TodoHistoryEntry, look: Look) -> Div {
    col()
        .p(px(7.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .gap(px(2.))
        .child(
            row()
                .gap(px(6.))
                .child(div().text_size(px(11.5)).font_weight(FontWeight::SEMIBOLD).child(entry.actor.clone()))
                .child(small(entry.field.clone(), look))
                .children(entry.at_day.map(|d| small(d.short(), look))),
        )
        .child(if !entry.word_diff.is_empty() {
            row()
                .flex_wrap()
                .gap(px(4.))
                .text_size(px(11.5))
                .children(entry.word_diff.iter().map(|span| match span.kind {
                    WordDiffKind::Equal => div().text_color(soft(look)).child(span.text.clone()),
                    WordDiffKind::Added => pill(
                        format!("+{}", span.text),
                        tone(look, 0xdff5f1, 0x153a36),
                        teal(look),
                    ),
                    WordDiffKind::Removed => pill(
                        format!("-{}", span.text),
                        tone(look, 0xfde4e4, 0x4b1f1f),
                        tone(look, 0xa51d1d, 0xff9088),
                    ),
                }))
                .into_any_element()
        } else {
            small(entry.change.clone(), look).into_any_element()
        })
}

fn ask_card(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    t: &Todo,
    ask_id: &str,
    ask: &Ask,
    look: Look,
) -> AnyElement {
    let session = t.sessions.first().cloned();
    let details_open = inputs.ask_details_open.get();
    let details_toggle = inputs.ask_details_open.clone();
    match &ask.kind {
        AskKind::Question {
            question,
            choices,
            multi_select,
            step_id,
            details,
            editable: _,
        } => {
            let draft = state.question_drafts.get(ask_id).cloned().unwrap_or_default();
            let ask_id_s = ask_id.to_owned();
            let input = inputs.ask_input.clone();
            let send_submit = send.clone();
            let selected_for_submit = draft.selected_choices.clone();
            col()
                .id("todo-ask-question")
                .mt(px(6.))
                .p(px(12.))
                .gap(px(8.))
                .rounded(px(10.))
                .bg(amber_soft(look))
                .child(
                    row()
                        .gap(px(6.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).flex_1().child(question.clone()))
                        .children(step_id.as_ref().map(|sid| tag(format!("step {sid}"), look))),
                )
                .children((!ask.reason.is_empty()).then(|| {
                    div().text_color(tone(look, 0x5c4a2a, 0xe0cfa8)).child(ask.reason.clone())
                }))
                .children(details.as_ref().filter(|d| !d.is_empty()).map(|d| {
                    col()
                        .gap(px(4.))
                        .child(
                            div()
                                .id("todo-ask-details-toggle")
                                .role(Role::Button)
                                .aria_label("Toggle details")
                                .cursor_pointer()
                                .text_size(px(11.5))
                                .text_color(rgb(look.accent))
                                .child(if details_open { "Hide details" } else { "Show details" })
                                .on_click(move |_, window, _| {
                                    details_toggle.set(!details_toggle.get());
                                    window.refresh();
                                })
                                .test_support(),
                        )
                        .children(details_open.then(|| small(d.clone(), look)))
                }))
                .children((!choices.is_empty()).then(|| {
                    row().flex_wrap().gap(px(6.)).children(choices.iter().enumerate().map(|(i, c)| {
                        let on = draft.selected_choices.contains(c);
                        let inner = div()
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(6.))
                            .text_size(px(12.))
                            .when(on, |d| d.bg(rgb(0x1c1c1e)).text_color(rgb(0xffffff)))
                            .when(!on, |d| d.bg(rgb(look.card)).text_color(soft(look)))
                            .child(c.clone());
                        clickable(
                            send,
                            SharedString::from(format!("todo-question-choice-{i}")),
                            c,
                            A::ToggleQuestionChoice {
                                ask_id: ask_id_s.clone(),
                                choice: c.clone(),
                                multi_select: *multi_select,
                            },
                            inner,
                        )
                    }))
                }))
                .child(
                    div()
                        .id("todo-question-input")
                        .child(Input::new(&inputs.ask_input).small().rounded(px(6.)).bg(rgb(look.card)))
                        .test_support(),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            push_primary_shape("todo-question-submit", "Send answer", look)
                                .on_click(move |_, window, cx| {
                                    let text = input.read(cx).value().trim().to_owned();
                                    send_submit(
                                        A::AnswerQuestion {
                                            ask_id: ask_id_s.clone(),
                                            text,
                                            selected_choices: selected_for_submit.clone(),
                                        },
                                        cx,
                                    );
                                    input.update(cx, |i, cx| {
                                        i.set_value(String::new(), window, cx)
                                    });
                                    window.refresh();
                                }),
                        )
                        .child(div().flex_1())
                        .children(session.map(|s| {
                            quiet(send, "todo-open-session", "Open session", A::OpenSession(s), look)
                        })),
                )
                .test_support()
                .into_any_element()
        }
        AskKind::ActionRequest { action, step_id, details } => {
            let ask_id_done = ask_id.to_owned();
            let ask_id_cant = ask_id.to_owned();
            let input_done = inputs.ask_input.clone();
            let input_cant = inputs.ask_input.clone();
            let send_done = send.clone();
            let send_cant = send.clone();
            col()
                .id("todo-ask-action")
                .mt(px(6.))
                .p(px(12.))
                .gap(px(8.))
                .rounded(px(10.))
                .bg(amber_soft(look))
                .child(
                    row()
                        .gap(px(6.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).flex_1().child(action.clone()))
                        .children(step_id.as_ref().map(|sid| tag(format!("step {sid}"), look))),
                )
                .children((!ask.reason.is_empty()).then(|| {
                    div().text_color(tone(look, 0x5c4a2a, 0xe0cfa8)).child(ask.reason.clone())
                }))
                .children(details.as_ref().filter(|d| !d.is_empty()).map(|d| small(d.clone(), look)))
                .child(
                    div()
                        .id("todo-action-note")
                        .child(Input::new(&inputs.ask_input).small().rounded(px(6.)).bg(rgb(look.card)))
                        .test_support(),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            push_primary_shape("todo-action-done", "Done", look).on_click(
                                move |_, window, cx| {
                                    let note = input_done.read(cx).value().trim().to_owned();
                                    send_done(
                                        A::AnswerAction {
                                            ask_id: ask_id_done.clone(),
                                            done: true,
                                            note,
                                        },
                                        cx,
                                    );
                                    input_done.update(cx, |i, cx| {
                                        i.set_value(String::new(), window, cx)
                                    });
                                    window.refresh();
                                },
                            ),
                        )
                        .child(
                            push_quiet_shape("todo-action-cant", "Can't be done", look).on_click(
                                move |_, window, cx| {
                                    let note = input_cant.read(cx).value().trim().to_owned();
                                    send_cant(
                                        A::AnswerAction {
                                            ask_id: ask_id_cant.clone(),
                                            done: false,
                                            note,
                                        },
                                        cx,
                                    );
                                    input_cant.update(cx, |i, cx| {
                                        i.set_value(String::new(), window, cx)
                                    });
                                    window.refresh();
                                },
                            ),
                        ),
                )
                .test_support()
                .into_any_element()
        }
        AskKind::ReviewRequest { summary, round, step_id, details, checkpoint } => {
            let ask_id_ok = ask_id.to_owned();
            let ask_id_chg = ask_id.to_owned();
            let input_ok = inputs.ask_input.clone();
            let input_chg = inputs.ask_input.clone();
            let send_ok = send.clone();
            let send_chg = send.clone();
            col()
                .id("todo-ask-review")
                .mt(px(6.))
                .p(px(12.))
                .gap(px(8.))
                .rounded(px(10.))
                .bg(amber_soft(look))
                .child(
                    row()
                        .gap(px(6.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).flex_1().child(summary.clone()))
                        .children((*round > 0).then(|| tag(format!("Round {round}"), look)))
                        .children(checkpoint.map(|cp| tag(cp.label(), look)))
                        .children(step_id.as_ref().map(|sid| tag(format!("step {sid}"), look))),
                )
                .children((!ask.reason.is_empty()).then(|| {
                    div().text_color(tone(look, 0x5c4a2a, 0xe0cfa8)).child(ask.reason.clone())
                }))
                .children(details.as_ref().filter(|d| !d.is_empty()).map(|d| small(d.clone(), look)))
                .child(
                    div()
                        .id("todo-review-note")
                        .child(Input::new(&inputs.ask_input).small().rounded(px(6.)).bg(rgb(look.card)))
                        .test_support(),
                )
                .child(
                    row()
                        .gap(px(8.))
                        .child(
                            push_primary_shape("todo-review-approve", "Approved", look).on_click(
                                move |_, window, cx| {
                                    let note = input_ok.read(cx).value().trim().to_owned();
                                    send_ok(
                                        A::AnswerReview {
                                            ask_id: ask_id_ok.clone(),
                                            approved: true,
                                            note,
                                        },
                                        cx,
                                    );
                                    input_ok.update(cx, |i, cx| {
                                        i.set_value(String::new(), window, cx)
                                    });
                                    window.refresh();
                                },
                            ),
                        )
                        .child(
                            push_quiet_shape("todo-review-changes", "Changes asked", look)
                                .on_click(move |_, window, cx| {
                                    let note = input_chg.read(cx).value().trim().to_owned();
                                    send_chg(
                                        A::AnswerReview {
                                            ask_id: ask_id_chg.clone(),
                                            approved: false,
                                            note,
                                        },
                                        cx,
                                    );
                                    input_chg.update(cx, |i, cx| {
                                        i.set_value(String::new(), window, cx)
                                    });
                                    window.refresh();
                                }),
                        ),
                )
                .test_support()
                .into_any_element()
        }
        AskKind::Close | AskKind::Edit { .. } | AskKind::Add => {
            let why_open = state.expanded_why.contains(ask_id);
            let waiting = ask_waiting_note(t, ask);
            col()
                .mt(px(6.))
                .p(px(12.))
                .gap(px(8.))
                .rounded(px(10.))
                .bg(amber_soft(look))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(ask_question(t, ask)))
                .children(waiting.map(|note| {
                    div()
                        .id("todo-ask-waiting")
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(tone(look, 0x7a4f00, 0xf2c97d))
                        .child(note)
                }))
                .children(why_open.then(|| {
                    div()
                        .id("todo-ask-why-body")
                        .p(px(8.))
                        .rounded(px(8.))
                        .bg(rgb(look.card))
                        .text_color(tone(look, 0x5c4a2a, 0xe0cfa8))
                        .child(ask_why_body(ask).to_owned())
                        .test_support()
                }))
                .children(
                    matches!(ask.kind, AskKind::Close)
                        .then(|| close_readiness_text(t))
                        .flatten()
                        .map(|summary| small(summary, look)),
                )
                .child(col().p(px(8.)).rounded(px(8.)).bg(rgb(look.card)).children(
                    ask_rows(t, ask).into_iter().map(|r| {
                        let body = if r.label == "Evidence" {
                            let mut line = row().flex_wrap().gap(px(4.));
                            for (i, (text, action)) in evidence_links(t).into_iter().enumerate() {
                                if i > 0 {
                                    line = line.child(muted("·", look));
                                }
                                line = match action {
                                    Some(act) => line.child(link(
                                        &opens_evidence(send),
                                        SharedString::from(format!("todo-evidence-{i}")),
                                        text,
                                        act,
                                        look,
                                    )),
                                    None => line.child(div().child(text.to_owned())),
                                };
                            }
                            line.into_any_element()
                        } else {
                            div()
                                .when(r.link, |d| d.text_color(rgb(look.accent)))
                                .child(r.text)
                                .into_any_element()
                        };
                        row()
                            .items_start()
                            .child(small(r.label, look).w(px(74.)).flex_shrink_0())
                            .child(div().flex_1().min_w_0().child(body))
                    }),
                ))
                .child(
                    row()
                        .gap(px(8.))
                        .child(primary(send, "todo-yes", "Yes", A::Answer(ask_id.to_owned(), true), look))
                        .child(quiet(send, "todo-no", "No", A::Answer(ask_id.to_owned(), false), look))
                        .child(div().flex_1())
                        .child(quiet(
                            send,
                            "todo-ask-why",
                            ask_why_label(why_open),
                            A::AskWhy(ask_id.to_owned()),
                            look,
                        ))
                        .children(session.map(|s| {
                            quiet(send, "todo-open-session", "Open session", A::OpenSession(s), look)
                        })),
                )
                .into_any_element()
        }
    }
}

fn stuck_agent_card(send: &Send, key: &str, stuck: &StuckAgent, look: Look) -> impl IntoElement {
    col()
        .id("todo-stuck-agent")
        .mt(px(6.))
        .p(px(12.))
        .gap(px(6.))
        .rounded(px(10.))
        .bg(amber_soft(look))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(format!("{} looks stuck ({})", stuck.agent_label, asked_ago(stuck.since_minutes))),
        )
        .child(small(stuck.reason.clone(), look))
        .child(
            row()
                .gap(px(8.))
                .child(primary(send, "todo-stuck-retry", "Retry", A::RetryStuck(key.to_owned()), look))
                .child(quiet(
                    send,
                    "todo-stuck-take-back",
                    "Take back",
                    A::RequestTakeBack(key.to_owned()),
                    look,
                )),
        )
        .test_support()
}

fn blocker_gone_card(send: &Send, key: &str, bg: &BlockerGone, look: Look) -> impl IntoElement {
    col()
        .id(SharedString::from(format!("todo-blocker-gone-{}", bg.blocker_key)))
        .mt(px(6.))
        .p(px(12.))
        .gap(px(6.))
        .rounded(px(10.))
        .bg(amber_soft(look))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
            "Blocker {} is {}",
            bg.blocker_key,
            state_label(bg.blocker_state)
        )))
        .child(
            row()
                .gap(px(8.))
                .flex_wrap()
                .child(primary(
                    send,
                    SharedString::from(format!("todo-bg-unblock-{}", bg.blocker_key)),
                    "Unblock",
                    A::ResolveBlockerGone {
                        key: key.to_owned(),
                        blocker_key: bg.blocker_key.clone(),
                        resolution: BlockerGoneResolution::Unblock,
                    },
                    look,
                ))
                .child(quiet(
                    send,
                    SharedString::from(format!("todo-bg-wontdo-{}", bg.blocker_key)),
                    "Won't do",
                    A::ResolveBlockerGone {
                        key: key.to_owned(),
                        blocker_key: bg.blocker_key.clone(),
                        resolution: BlockerGoneResolution::CloseWontDo,
                    },
                    look,
                ))
                .children((!bg.replacement_keys.is_empty()).then(|| {
                    let label = format!("Rewire to {}", bg.replacement_keys.join(", "));
                    quiet(
                        send,
                        SharedString::from(format!("todo-bg-rewire-{}", bg.blocker_key)),
                        &label,
                        A::ResolveBlockerGone {
                            key: key.to_owned(),
                            blocker_key: bg.blocker_key.clone(),
                            resolution: BlockerGoneResolution::Rewire(bg.replacement_keys.clone()),
                        },
                        look,
                    )
                })),
        )
        .test_support()
}

fn lifecycle_prompt_card(send: &Send, key: &str, lp: &LifecyclePrompt, look: Look) -> impl IntoElement {
    col()
        .id(SharedString::from(format!("todo-lifecycle-{}", lp.id)))
        .mt(px(6.))
        .p(px(12.))
        .gap(px(6.))
        .rounded(px(10.))
        .bg(rgb(look.fill))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(lp.title.clone()))
        .children((!lp.detail.is_empty()).then(|| small(lp.detail.clone(), look)))
        .child(row().gap(px(8.)).flex_wrap().children(lp.choices.iter().enumerate().map(|(i, choice)| {
            let act = A::ResolveLifecyclePrompt {
                key: key.to_owned(),
                prompt_id: lp.id.clone(),
                choice: choice.clone(),
            };
            let id = SharedString::from(format!("todo-lp-{}-{i}", lp.id));
            if i == 0 {
                primary(send, id, choice, act, look)
            } else {
                quiet(send, id, choice, act, look)
            }
        })))
        .test_support()
}

#[cfg(test)]
mod tests {
    use super::*;
    // gpui_kit's glob brings in its own `test` attribute; use std's.
    use core::prelude::v1::test;
    use humata2_clients_fakes::todo::FakeTodo;
    use humata2_todo_feature::View;

    fn state() -> TodoState {
        let fake = FakeTodo::frame_b1_1();
        let mut s = TodoState::new(View::default(), fake.today());
        s.items = fake.entries();
        s.loaded = true;
        s
    }

    #[test]
    fn header_counts_open_items_and_channels() {
        let s = state();
        let (open, channels) = s.header();
        assert_eq!(header_text(&s), format!("{open} open across {channels} channels"));
    }

    #[test]
    fn header_names_the_picked_channel() {
        let mut s = state();
        s.view.filter.channel = Some(ChannelPick::Channel("infra".to_owned()));
        assert!(header_text(&s).ends_with("in infra"));
    }

    #[test]
    fn empty_list_says_so_and_offers_no_do_next() {
        let s = TodoState::new(View::default(), state().today);
        assert_eq!(header_text(&s), "no items");
        assert_eq!(do_next(&s).1, None);
    }

    #[test]
    fn do_next_names_the_top_next_item() {
        let s = state();
        let (label, key) = do_next(&s);
        let key = key.expect("the list has a next item");
        assert!(label.contains(&key));
        assert_eq!(Some(key), s.ranked().next.first().map(|r| r.key.clone()));
    }

    #[test]
    fn sections_come_in_drawing_order_and_skip_empty_groups() {
        let s = state();
        let groups: Vec<_> = sections(&s).iter().map(|x| x.group).collect();
        assert_eq!(&groups[..3], [Group::Attention, Group::InProgress, Group::Next]);
        assert!(!groups.contains(&Group::Closed));
    }

    #[test]
    fn waiting_and_hidden_fold_until_a_channel_is_picked() {
        let mut s = state();
        assert!(folded(&s, Group::Waiting, false));
        assert!(!folded(&s, Group::Waiting, true));
        assert!(!folded(&s, Group::Next, false));
        s.view.filter.channel = Some(ChannelPick::Channel("web".to_owned()));
        assert!(!folded(&s, Group::Hidden, false));
        assert!(folded(&s, Group::Hidden, true));
    }

    #[test]
    fn a_large_list_folds_the_groups_above_next() {
        let mut s = state();
        let template = s.items[0].clone();
        for n in 0..LARGE {
            let mut e = template.clone();
            e.todo.key = format!("BIG-{n}");
            e.todo.ask = None;
            e.ask_id = None;
            e.todo.state = State::Open;
            s.items.push(e);
        }
        assert!(folded(&s, Group::Attention, false));
        assert!(!folded(&s, Group::Next, false));
    }

    #[test]
    fn a_typed_line_previews_where_it_lands() {
        let mut s = state();
        assert_eq!(preview_line(&s), None);
        let line = "fix login redirect loop fri #bug";
        s.quick_add.preview = Some(humata2_todo_feature::Preview {
            draft: humata2_todo::parse(line, s.today, &s.channels()).expect("a title"),
            position: Some(2),
        });
        assert!(preview_line(&s).is_some());
    }

    #[test]
    fn step_progress_counts_done_steps() {
        let s = state();
        let t = s.items.iter().map(|e| &e.todo).find(|t| !t.steps.is_empty()).expect("a stepped item");
        let done = t.steps.iter().filter(|x| x.done).count();
        assert_eq!(step_progress(t), Some(format!("{done}/{}", t.steps.len())));
        assert_eq!(step_progress(&Todo::new("X-1", "x", s.today)), None);
    }

    #[test]
    fn an_edit_ask_shows_before_and_after() {
        let s = state();
        let t = s
            .items
            .iter()
            .map(|e| &e.todo)
            .find(|t| matches!(t.ask.as_ref().map(|a| &a.kind), Some(AskKind::Edit { .. })))
            .expect("an edit ask");
        let ask = t.ask.as_ref().expect("ask");
        let AskKind::Edit { before, after, .. } = &ask.kind else { unreachable!() };
        let change = ask_change(t, ask).expect("a change");
        assert!(change.contains(before.as_str()) && change.contains(after.as_str()));
    }

    #[test]
    fn hiding_picks_a_later_day() {
        let s = state();
        assert!(hide_day(s.today) > s.today);
    }

    #[test]
    fn asked_ago_reads_in_the_largest_unit() {
        assert_eq!(asked_ago(0), "just now");
        assert_eq!(asked_ago(20), "20m ago");
        assert_eq!(asked_ago(120), "2h ago");
        assert_eq!(asked_ago(3000), "2d ago");
    }

    #[test]
    fn the_ask_card_lists_evidence_change_also_and_asker_in_order() {
        let s = state();
        let t = s.items.iter().map(|e| &e.todo).find(|t| !t.ask_detail.evidence.is_empty()).expect("detailed ask");
        let ask = t.ask.as_ref().expect("ask");
        let labels: Vec<_> = ask_rows(t, ask).iter().map(|r| r.label).collect();
        assert_eq!(labels, ["Evidence", "Change", "Also", "Asked by"]);
    }

    #[test]
    fn ask_why_toggles_inline_and_waiting_note_names_blocked_agent() {
        assert_eq!(ask_why_label(false), "Why?");
        assert_eq!(ask_why_label(true), "Hide why");

        let ask = Ask {
            kind: AskKind::Close,
            by: "claude in Ledger entries".to_owned(),
            reason: "All 3 commits merged and tests pass.".to_owned(),
        };
        assert_eq!(ask_why_body(&ask), "All 3 commits merged and tests pass.");
        let empty = Ask { reason: "   ".to_owned(), ..ask.clone() };
        assert_eq!(ask_why_body(&empty), "No reason was given.");

        let mut t = Todo::new("PAY-9", "Close me", Day::from_ymd(2026, 11, 19).unwrap());
        assert_eq!(ask_waiting_note(&t, &ask), None);
        t.session_signals.push(SessionSignal {
            session: "sess-1".to_owned(),
            label: "Ledger entries".to_owned(),
            status: "needs_you".to_owned(),
            idle_days: 0,
            active_turn: false,
        });
        assert_eq!(
            ask_waiting_note(&t, &ask).as_deref(),
            Some("claude is waiting on this")
        );
    }

    #[test]
    fn groups_other_than_next_show_a_few_rows_until_all_are_asked_for() {
        assert_eq!(visible_rows(Group::InProgress, 25, false), GROUP_CAP);
        assert_eq!(visible_rows(Group::InProgress, 25, true), 25);
        assert_eq!(visible_rows(Group::InProgress, 3, false), 3);
        assert_eq!(visible_rows(Group::Next, 48, false), 48);
    }

    #[test]
    fn the_visible_range_follows_the_scroll_and_stays_inside_the_list() {
        assert_eq!(visible_range(0., 530., 53., 48), (1, 10));
        assert_eq!(visible_range(530., 530., 53., 48), (11, 20));
        assert_eq!(visible_range(10_000., 530., 53., 48), (48, 48));
        assert_eq!(visible_range(0., 530., 53., 0), (0, 0));
    }

    #[test]
    fn the_filter_footer_counts_what_is_shown_of_the_channel() {
        let mut s = state();
        s.view.filter.channel = Some(ChannelPick::Channel("payments-rewrite".to_owned()));
        s.view.filter.due = vec![DuePick::Overdue];
        let text = filter_count_text(&s);
        assert!(text.ends_with("in payments-rewrite"));
        let shown = s.shown().iter().filter(|t| t.state != State::Closed).count();
        assert!(text.starts_with(&shown.to_string()));
    }

    #[test]
    fn only_web_evidence_opens_in_the_browser() {
        assert!(opens_in_browser("https://github.com/o/r/pull/7"));
        assert!(opens_in_browser("http://localhost:8080/report"));
        assert!(!opens_in_browser("commits"));
        assert!(!opens_in_browser("file:///etc/passwd"));
    }

    #[test]
    fn evidence_links_produce_open_evidence_actions() {
        let s = state();
        let mut t = s
            .items
            .iter()
            .map(|e| e.todo.clone())
            .find(|t| !t.ask_detail.evidence.is_empty())
            .expect("detailed ask");
        t.ask_detail.evidence.push(humata2_todo::Evidence {
            text: "plain note".to_owned(),
            link: None,
        });
        let links = evidence_links(&t);
        assert_eq!(links.len(), t.ask_detail.evidence.len());
        assert_eq!(links[0].1, Some(A::OpenEvidence("commits".to_owned())));
        assert_eq!(links.last().and_then(|(_, a)| a.clone()), None);
    }

    #[test]
    fn start_session_label_tracks_running_sessions() {
        let s = state();
        let fresh = s
            .items
            .iter()
            .map(|e| &e.todo)
            .find(|t| t.sessions.is_empty() && t.review.is_none() && !t.suggested)
            .expect("an item with no session");
        assert_eq!(start_session_label(fresh), "Start a session on this");
        let running = s
            .items
            .iter()
            .map(|e| &e.todo)
            .find(|t| !t.sessions.is_empty() && !t.ask_detail.session_ended)
            .expect("an item with a running session");
        assert_eq!(start_session_label(running), "Start another session on this");
        let ended = s
            .items
            .iter()
            .map(|e| &e.todo)
            .find(|t| t.ask_detail.session_ended)
            .expect("an item whose session ended");
        assert_eq!(start_session_label(ended), "Start a session on this");
    }

    #[test]
    fn sort_switcher_lists_all_five_modes() {
        assert_eq!(SORT_OPTIONS.len(), 5);
        assert_eq!(sort_pill_label(SortBy::Priority), "Sort: Priority");
        assert_eq!(sort_pill_label(SortBy::DueSoonest), "Sort: Due soonest");
        assert_eq!(sort_pill_label(SortBy::MostNeeded), "Sort: Most needed");
        assert_eq!(sort_pill_label(SortBy::SmallestEffort), "Sort: Smallest effort");
        assert_eq!(sort_pill_label(SortBy::LargestImpact), "Sort: Largest impact");
    }

    #[test]
    fn hidden_until_pill_appears_only_when_searching_snoozed_item() {
        let mut s = state();
        let mut item = Todo::new("SNOOZE-1", "Renew cert", s.today);
        item.hide_until = Some(s.today.add_days(5));
        assert_eq!(hidden_until_pill(&item, &s), None);
        s.view.filter.search = "cert".to_owned();
        let pill = hidden_until_pill(&item, &s).expect("hidden pill when searching");
        assert!(pill.starts_with("Hidden until "));
    }

    #[test]
    fn idle_nudge_triggers_only_when_inactive_and_idle() {
        let active = SessionSignal {
            session: "s1".into(),
            label: "claude".into(),
            status: "working".into(),
            idle_days: 3,
            active_turn: true,
        };
        assert_eq!(idle_nudge(&active), None);
        let idle = SessionSignal { active_turn: false, ..active };
        assert_eq!(idle_nudge(&idle).as_deref(), Some("Idle 3d — resume or pause?"));
    }

    #[test]
    fn watching_pr_appears_in_sections_between_in_progress_and_next() {
        let mut s = state();
        let mut pr = Todo::new("PR-9", "Watch PR #9", s.today);
        pr.state = State::WatchingPr;
        s.items.push(Entry { todo: pr, revision: 1, ask_id: None });
        let groups: Vec<_> = sections(&s).iter().map(|sec| sec.group).collect();
        let idx_ip = groups.iter().position(|g| *g == Group::InProgress).unwrap();
        let idx_pr = groups.iter().position(|g| *g == Group::WatchingPr).unwrap();
        let idx_next = groups.iter().position(|g| *g == Group::Next).unwrap();
        assert!(idx_ip < idx_pr && idx_pr < idx_next);
    }

    #[test]
    fn keyboard_triage_maps_shortcuts_and_never_hijacks_inputs() {
        // When an input is focused, all shortcuts are ignored.
        assert_eq!(triage_command("down", false, false, true, Some("A-1")), None);
        assert_eq!(triage_command("x", false, false, true, Some("A-1")), None);
        assert_eq!(triage_command("/", false, false, true, Some("A-1")), None);

        // List navigation & triage shortcuts.
        assert_eq!(
            triage_command("down", false, false, false, Some("A-1")),
            Some(TriageCommand::Action(A::MoveFocus(1)))
        );
        assert_eq!(
            triage_command("up", false, true, false, Some("A-1")),
            Some(TriageCommand::Action(A::StepDrawer(-1)))
        );
        assert_eq!(
            triage_command("down", true, false, false, Some("A-1")),
            Some(TriageCommand::Action(A::ExtendSelection(1)))
        );
        assert_eq!(
            triage_command("enter", false, false, false, Some("A-1")),
            Some(TriageCommand::Action(A::Open("A-1".into())))
        );
        assert_eq!(
            triage_command("s", false, false, false, Some("A-1")),
            Some(TriageCommand::Action(A::StartOrNextFocused))
        );
        assert_eq!(
            triage_command("x", false, false, false, Some("A-1")),
            Some(TriageCommand::Action(A::ToggleFocusDone))
        );
        assert_eq!(
            triage_command("/", false, false, false, None),
            Some(TriageCommand::FocusSearch)
        );
        assert_eq!(
            triage_command("N", true, false, false, None),
            Some(TriageCommand::FocusQuickAdd)
        );
    }

    #[test]
    fn shift_click_dispatches_multi_select_and_plain_click_opens_drawer() {
        assert_eq!(
            row_click_action("PAY-1", false, true, false),
            A::SelectRow { key: "PAY-1".into(), shift: true, toggle: false }
        );
        assert_eq!(
            row_click_action("PAY-1", false, false, true),
            A::SelectRow { key: "PAY-1".into(), shift: false, toggle: true }
        );
        assert_eq!(row_click_action("PAY-1", false, false, false), A::Open("PAY-1".into()));
        assert_eq!(row_click_action("PAY-1", true, false, false), A::Close);
    }

    #[test]
    fn fuzzy_due_is_never_red_and_dropped_steps_are_excluded_from_progress() {
        let s = state();
        let mut t = Todo::new("FUZ-1", "Fuzzy task", s.today);
        t.fuzzy_due = Some(FuzzyDue::InAFewDays);
        let chip = due_chip(&t, s.today).expect("fuzzy due chip");
        assert_eq!(chip.tone, Tone::Plain);
        assert_eq!(chip.text, "In a few days");

        t.steps = vec![
            humata2_todo::Step { text: "one".into(), done: true, ..Default::default() },
            humata2_todo::Step { text: "two".into(), done: false, dropped: true, ..Default::default() },
            humata2_todo::Step { text: "three".into(), done: false, ..Default::default() },
        ];
        assert_eq!(step_progress(&t), Some("1/2".to_owned()));
        assert_eq!(unset_fields(&t), vec!["Need", "Effort", "Impact"]);
    }

    #[test]
    fn runs_ledger_has_no_amount_when_cost_is_unknown() {
        let s = state();
        let mut t = Todo::new("RUN-2", "Ship API", s.today);
        t.run_count = 2;
        t.total_cost_micros = None;
        let label = runs_ledger_label(&t, None).expect("label");
        assert!(label.contains('2'));
        assert!(!label.contains('$'));
    }

    #[test]
    fn runs_ledger_and_structured_asks_format_cleanly() {
        let s = state();
        let mut t = Todo::new("RUN-1", "Ship API", s.today);
        t.run_count = 3;
        t.total_cost_micros = Some(1_450_000);
        assert_eq!(runs_ledger_label(&t, None), Some("Runs 3 · $1.45".to_owned()));

        t.close_readiness = Some(humata2_todo::CloseReadiness {
            steps_done: 2,
            steps_total: 3,
            unresolved_comments: 1,
            ci_status: "passing".into(),
            commits: 4,
        });
        assert_eq!(
            close_readiness_text(&t).as_deref(),
            Some("Steps 2/3 · 1 unresolved comment · CI passing · 4 commits")
        );

        let q = Ask {
            kind: AskKind::Question {
                question: "Use OAuth or PAT?".into(),
                choices: vec!["OAuth".into(), "PAT".into()],
                multi_select: false,
                step_id: Some("s1".into()),
                details: None,
                editable: true,
            },
            by: "claude".into(),
            reason: "Need auth choice".into(),
        };
        assert_eq!(ask_question(&t, &q), "Use OAuth or PAT?");

        let r = Ask {
            kind: AskKind::ReviewRequest {
                summary: "Check migration script".into(),
                round: 2,
                step_id: None,
                details: None,
                checkpoint: Some(CheckpointState::Defined),
            },
            by: "claude".into(),
            reason: "Ready for round 2".into(),
        };
        assert_eq!(ask_question(&t, &r), "Review (round 2): Check migration script");
        assert_eq!(ask_change(&t, &r).as_deref(), Some("Checkpoint: Defined"));
    }

    #[core::prelude::v1::test]
    fn channel_display_uses_human_name_instead_of_generated_id() {
        let mut s = state();
        let ch_id = "ch_9f8a7b6c5d4e";
        s.set_channels(vec![humata2_todo_feature::ChannelRef {
            id: ch_id.to_owned(),
            name: "Payments".to_owned(),
        }]);
        let mut item = Todo::new("PAY-99", "Fix checkout", s.today);
        item.channel = Some(ch_id.to_owned());
        s.items.push(Entry { todo: item, revision: 1, ask_id: None });
        s.view.filter.channel = Some(ChannelPick::Channel(ch_id.to_owned()));

        let header = header_text(&s);
        let placeholder = add_placeholder(&s);
        let filter_count = filter_count_text(&s);

        assert!(header.contains("Payments"), "header should show channel name: {header}");
        assert!(!header.contains(ch_id), "header should not leak channel id: {header}");

        assert!(placeholder.contains("Payments"), "placeholder should show channel name: {placeholder}");
        assert!(!placeholder.contains(ch_id), "placeholder should not leak channel id: {placeholder}");

        assert!(filter_count.contains("Payments"), "filter count should show channel name: {filter_count}");
        assert!(!filter_count.contains(ch_id), "filter count should not leak channel id: {filter_count}");
    }

    #[core::prelude::v1::test]
    fn linked_session_and_channel_clicks_dispatch_navigation_actions() {
        assert_eq!(session_link_action("se_42"), A::OpenSession("se_42".to_owned()));
        assert_eq!(
            channel_link_action(Some("ch_payments")),
            Some(A::OpenChannel("ch_payments".to_owned()))
        );
        assert_eq!(channel_link_action(None), None);
    }

    #[test]
    fn milestone_start_shortcut_dispatches_brief_or_plain_and_ignores_inputs_and_tasks() {
        assert_eq!(
            milestone_start_shortcut("s", false, false, Some("M-1"), true),
            Some(A::StartSession("M-1".to_owned()))
        );
        assert_eq!(
            milestone_start_shortcut("S", false, false, Some("M-1"), true),
            Some(A::StartSession("M-1".to_owned()))
        );
        assert_eq!(
            milestone_start_shortcut("s", true, false, Some("M-1"), true),
            Some(A::StartPlainSession("M-1".to_owned()))
        );
        assert_eq!(milestone_start_shortcut("s", false, false, Some("T-1"), false), None);
        assert_eq!(milestone_start_shortcut("s", true, false, Some("T-1"), false), None);
        assert_eq!(milestone_start_shortcut("s", false, true, Some("M-1"), true), None);
        assert_eq!(milestone_start_shortcut("s", true, true, Some("M-1"), true), None);
        assert_eq!(milestone_start_shortcut("s", false, false, None, true), None);
        assert_eq!(milestone_start_shortcut("x", false, false, Some("M-1"), true), None);
    }

    #[test]
    fn milestone_helper_text_formats_zero_one_and_many_open_steps() {
        assert_eq!(milestone_helper_text(0), "All steps done · S");
        assert_eq!(milestone_helper_text(1), "Pre-fills 1 open step · S");
        assert_eq!(milestone_helper_text(3), "Pre-fills 3 open steps · S");
    }
}
