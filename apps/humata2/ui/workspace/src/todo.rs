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
    input::{Input, InputEvent, InputState},
};
use gpui_kit::{prelude::*, *};
use humata2_clients::todo::Entry;
use humata2_todo::{Ask, AskKind, Day, DueChip, Effort, Impact, Row, State, Todo, Tone, Weekday};
use humata2_todo_feature::{
    ChannelPick, DuePick, EditField, OnlyPick, SessionPick, TodoAction as A, TodoState, ViewFilter,
};
use std::cell::Cell;
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
        Group::Attention | Group::InProgress => state.shown().len() > LARGE,
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
        Some(ChannelPick::Channel(name)) => format!("{open} open in {name}"),
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
        Some(ChannelPick::Channel(name)) => {
            format!("Add an item to {name}… try “fix login redirect fri #bug”")
        }
        _ => "Add an item… try “fix login redirect fri #bug”".to_owned(),
    }
}

/// "4/5", for an item with steps.
pub(crate) fn step_progress(todo: &Todo) -> Option<String> {
    if todo.steps.is_empty() {
        return None;
    }
    let done = todo.steps.iter().filter(|s| s.done).count();
    Some(format!("{done}/{}", todo.steps.len()))
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

/// The question an ask puts to the person.
pub(crate) fn ask_question(todo: &Todo, ask: &Ask) -> String {
    match &ask.kind {
        AskKind::Close => format!("Close {}?", todo.title),
        AskKind::Edit { field, .. } => format!("Change {field} on {}?", todo.title),
        AskKind::Add => format!("Add {}?", todo.title),
    }
}

/// What the ask would change, as "before → after".
pub(crate) fn ask_change(todo: &Todo, ask: &Ask) -> Option<String> {
    match &ask.kind {
        AskKind::Close => {
            let before = match todo.state {
                State::Open => "Open",
                State::InProgress => "In progress",
                State::Closed => "Closed",
            };
            Some(format!("State: {before} → Closed"))
        }
        AskKind::Edit { field, before, after } => Some(format!("{field}: {before} → {after}")),
        AskKind::Add => None,
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
        Group::Next | Group::Closed => String::new(),
    }
}

/// The due chip a row shows, if it has a date.
pub(crate) fn due_chip(todo: &Todo, today: Day) -> Option<DueChip> {
    todo.due.map(|d| humata2_todo::chip(d, today))
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

/// What the "Ask why" button says: an ended session can't be asked.
pub(crate) fn ask_why_label(todo: &Todo) -> &'static str {
    if todo.ask_detail.session_ended {
        "Resume the session to ask why"
    } else {
        "Ask why"
    }
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
        Some(ChannelPick::Channel(c)) => format!("{shown} of {of} items in {c}"),
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

/// What the page keeps between draws: the fold toggles and the three fields.
pub struct Inputs {
    pub(crate) quick_add: Entity<InputState>,
    search: Entity<InputState>,
    pub(crate) edit: Entity<InputState>,
    toggles: Rc<Cell<u32>>,
    filters_open: Rc<Cell<bool>>,
    editing: Cell<bool>,
    placeholder: std::cell::RefCell<String>,
    scroll: ScrollHandle,
    _subscriptions: [Subscription; 3],
}

impl Inputs {
    /// Fields that send their changes through `send`.
    pub fn new<V: 'static>(send: &Send, window: &mut Window, cx: &mut Context<V>) -> Self {
        let quick_add = cx.new(|cx| InputState::new(window, cx).placeholder("Add an item…"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let edit = cx.new(|cx| InputState::new(window, cx));
        let (one, two, three) = (send.clone(), send.clone(), send.clone());
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
        ];
        Self {
            quick_add,
            search,
            edit,
            toggles: Rc::new(Cell::new(0)),
            filters_open: Rc::new(Cell::new(false)),
            editing: Cell::new(false),
            placeholder: std::cell::RefCell::new(String::new()),
            scroll: ScrollHandle::new(),
            _subscriptions: subs,
        }
    }

    /// Puts the state's text into the fields that differ, and focuses the
    /// drawer field when typing into it starts.
    pub fn sync(&self, state: &TodoState, window: &mut Window, cx: &mut App) {
        let editing = state.editing.as_ref();
        for (input, value) in [
            (&self.quick_add, state.quick_add.line.as_str()),
            (&self.search, state.view.filter.search.as_str()),
            (&self.edit, editing.map_or("", |e| e.text.as_str())),
        ] {
            if input.read(cx).value().as_ref() != value {
                input.update(cx, |i, cx| i.set_value(value.to_owned(), window, cx));
            }
        }
        if editing.is_some() && !self.editing.get() {
            self.edit.update(cx, |i, cx| i.focus(window, cx));
        }
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

/// The whole page: header, toolbar, quick add, the groups and the drawer.
pub fn page(
    send: &Send,
    state: &TodoState,
    inputs: &Inputs,
    sessions: Sessions<'_>,
    look: Look,
) -> Div {
    let open = state.open.as_deref().and_then(|k| entry(state, k));
    col()
        .size_full()
        .child(header(send, state, inputs, look))
        .child(channel_bar(send, state, look))
        .child(
            row()
                .flex_1()
                .min_h_0()
                .items_start()
                .child(list(send, state, inputs, sessions, look, open.is_some()))
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
                })),
        )
}

fn header(send: &Send, state: &TodoState, inputs: &Inputs, look: Look) -> Div {
    let (label, key) = do_next(state);
    let toggle = inputs.filters_open.clone();
    let filters_on = state.view.filter.due.len()
        + state.view.filter.tags.len()
        + usize::from(state.view.filter.session.is_some())
        + state.view.filter.effort.len()
        + state.view.filter.impact.len()
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
        .child(small("Sort: Ranked", look).px(px(10.)).whitespace_nowrap())
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
            let (name, pick) = match &channel {
                Some(c) => (c.clone(), ChannelPick::Channel(c.clone())),
                None => ("No channel".to_owned(), ChannelPick::NoChannel),
            };
            let on = filter.channel.as_ref() == Some(&pick);
            chip(
                format!("todo-ch-{name}"),
                format!("{name} {n}"),
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
                .children(d.channel.as_ref().map(|c| tag(format!("# {c}"), look)))
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
    let folds = matches!(g, Group::Waiting | Group::Hidden | Group::Attention | Group::InProgress);
    let note = match g {
        Group::Attention => "an agent wants to change these; open one to say yes or no",
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
    let selected = state.open.as_deref() == Some(t.key.as_str());
    let closed = t.state == State::Closed;
    let key = t.key.clone();
    let open = if selected { A::Close } else { A::Open(key.clone()) };
    let sender = send.clone();
    let reasons: Vec<String> = r.reasons.iter().map(humata2_todo::Reason::text).collect();
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
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .when(closed, |d| d.text_color(rgb(look.secondary)))
                .child(t.title.clone()),
        )
        .children(t.ask.as_ref().filter(|_| !t.suggested).map(|a| attention_pill(a.label(), look)))
        .children(t.suggested.then(|| {
            let label = t.ask.as_ref().map_or("add?", Ask::label);
            attention_pill(label, look)
        }))
        .children(t.review.as_ref().map(|_| {
            pill("Pull request review", tone(look, 0xece9fd, 0x322c5c), tone(look, 0x4a2fa8, 0xcdc4ff))
                .font_weight(FontWeight::MEDIUM)
        }))
        .children(t.review.iter().flat_map(|rv| rv.repo_prs.iter()).map(|pr| tag(pr.clone(), look)))
        .children(t.review.as_ref().map(|rv| small(format!("by {}", rv.author), look)))
        .children(step_progress(t).map(|s| small(s, look)))
        .children(due_chip(t, state.today).map(|c| due_pill(&c, look)))
        .children(t.tags.iter().map(|g| tag(g.clone(), look)));
    let detail = row()
        .gap(px(6.))
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
                .filter_map(|s| sessions(s))
                .map(|chip| session_chip(chip, look)),
        )
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
            .child(small(t.channel.clone().unwrap_or_else(|| "No channel".to_owned()), look).w(px(130.)))
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
        .when(t.suggested, |x| x.border_1().border_dashed().border_color(rgb(look.line)))
        .on_click(move |_, _, cx| sender(open.clone(), cx))
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
        .children(columns)
}

fn session_chip(chip: SessionChip, look: Look) -> Div {
    row()
        .gap(px(6.))
        .px(px(8.))
        .py(px(1.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .text_size(px(12.))
        .child(div().size(px(8.)).rounded_full().bg(rgb(chip.dot)))
        .child(chip.name)
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
        row().gap(px(12.)).py(px(4.)).child(small(label, look).w(px(96.)).flex_shrink_0()).child(body)
    };
    let seg = |id: &'static str, options: Vec<(String, bool, A)>| {
        row().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).children(options.into_iter().map(
            move |(name, on, action)| {
                let inner = div()
                    .px(px(10.))
                    .py(px(2.))
                    .rounded(px(5.))
                    .text_size(px(12.))
                    .when(on, |d| d.bg(rgb(look.card)).font_weight(FontWeight::SEMIBOLD))
                    .child(name.clone());
                clickable(send, SharedString::from(format!("{id}-{name}")), &name, action, inner)
            },
        ))
    };
    let due_label = t.due.map_or("No date".to_owned(), Day::short);
    col()
        .gap(px(6.))
        .child(
            row()
                .gap(px(8.))
                .child(
                    small(
                        format!("{} · made by you, {}", t.key, t.created.short()),
                        look,
                    )
                    .flex_1(),
                )
                .children(close.then(|| link(send, "todo-close", "×", A::Close, look))),
        )
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
        .children(
            e.ask_id
                .as_ref()
                .zip(t.ask.as_ref())
                .filter(|_| !t.suggested)
                .map(|(id, ask)| ask_card(send, t, id, ask, look)),
        )
        .child(field(
            "State",
            seg(
                "todo-state",
                [(State::Open, "Open"), (State::InProgress, "In progress"), (State::Closed, "Closed")]
                    .into_iter()
                    .map(|(s, n)| (n.to_owned(), t.state == s, A::SetState(key.clone(), s)))
                    .collect(),
            ),
        ))
        .child(field("Channel", div().child(t.channel.as_ref().map_or("No channel".to_owned(), |c| format!("# {c}")))))
        .child(field(
            "Sessions",
            row().gap(px(6.)).children(t.sessions.iter().map(|s| match sessions(s) {
                Some(chip) => row()
                    .gap(px(6.))
                    .px(px(8.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .bg(rgb(look.fill))
                    .text_size(px(12.))
                    .child(div().size(px(8.)).rounded_full().bg(rgb(chip.dot)))
                    .child(chip.name),
                None => tag(s.clone(), look),
            })),
        ))
        .child(field(
            "Due",
            col()
                .gap(px(4.))
                .child(if editing(EditField::Due) {
                    edit_input("fri, nov 28, or 2026-12-01").into_any_element()
                } else {
                    row()
                        .gap(px(8.))
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
                        .child(link(
                            send,
                            "todo-due-week",
                            "Next week",
                            A::SetDue(key.clone(), state.today.next_weekday(Weekday::Mon)),
                            look,
                        ))
                        .children(t.due.map(|_| {
                            link(send, "todo-due-clear", "Clear", A::ClearDue(key.clone()), look)
                        }))
                        .into_any_element()
                })
                .children(state.due_error.as_ref().map(|err| {
                    div()
                        .text_size(px(12.))
                        .text_color(rgb(crate::theme::RED))
                        .child(err.to_string())
                })),
        ))
        .child(field(
            "Effort",
            row()
                .gap(px(14.))
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
                .child(small("Impact", look))
                .child(seg(
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
                )),
        ))
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
        .children((!t.depends_on.is_empty()).then(|| {
            field(
                "Waits on",
                row().gap(px(6.)).children(
                    t.depends_on
                        .iter()
                        .map(|k| pill(k.clone(), tone(look, 0xdff5f1, 0x153a36), teal(look))),
                ),
            )
        }))
        .child(
            row()
                .mt(px(10.))
                .child(small(
                    format!("STEPS · {} OF {}", t.steps.iter().filter(|s| s.done).count(), t.steps.len()),
                    look,
                ).font_weight(FontWeight::SEMIBOLD)),
        )
        .children(t.steps.iter().enumerate().map(|(i, s)| {
            let boxed = div()
                .size(px(15.))
                .rounded(px(4.))
                .border_1()
                .border_color(if s.done { teal(look) } else { tone(look, 0xb9b9c0, 0x55555a) })
                .when(s.done, |b| b.bg(teal(look)).text_color(rgb(0xffffff)).text_size(px(10.)).child("✓"));
            clickable(
                send,
                SharedString::from(format!("todo-step-{i}")),
                &s.text,
                A::ToggleStep(key.clone(), i),
                row().gap(px(9.)).py(px(3.)).child(boxed).child(
                    div().when(s.done, |d| d.text_color(rgb(look.secondary))).child(s.text.clone()),
                ),
            )
        }))
        .child(
            row().mt(px(10.)).child(
                small("DESCRIPTION", look).font_weight(FontWeight::SEMIBOLD),
            ),
        )
        .child(if editing(EditField::Description) {
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
        })
        .child(if editing(EditField::Step) {
            edit_input("Step").into_any_element()
        } else {
            link(send, "todo-add-step", "Add a step…", A::StartEdit(key.clone(), EditField::Step), look)
                .into_any_element()
        })
        .child(
            row()
                .gap(px(8.))
                .mt(px(12.))
                .pt(px(10.))
                .border_t_1()
                .border_color(rgb(look.line))
                .child(link(send, "todo-history", "History", A::ShowHistory(key.clone()), look))
                .child(div().flex_1())
                .child(if t.pin.is_some() {
                    link(send, "todo-pin", "Unpin", A::Unpin(key.clone()), look)
                } else {
                    link(send, "todo-pin", "Pin", A::Pin(key.clone()), look)
                })
                .child(if t.hide_until.is_some() {
                    link(send, "todo-hide", "Stop hiding", A::HideUntil(key.clone(), None), look)
                } else {
                    link(
                        send,
                        "todo-hide",
                        "Hide until…",
                        A::HideUntil(key.clone(), Some(hide_day(state.today))),
                        look,
                    )
                }),
        )
}

fn ask_card(send: &Send, t: &Todo, ask_id: &str, ask: &Ask, look: Look) -> Div {
    let session = t.sessions.first().cloned();
    col()
        .mt(px(6.))
        .p(px(12.))
        .gap(px(8.))
        .rounded(px(10.))
        .bg(amber_soft(look))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(ask_question(t, ask)))
        .child(div().text_color(tone(look, 0x5c4a2a, 0xe0cfa8)).child(ask.reason.clone()))
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
                                send,
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
                    .gap(px(12.))
                    .child(small(r.label, look).w(px(70.)))
                    .child(body)
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
                    ask_why_label(t),
                    if t.ask_detail.session_ended {
                        A::OpenSession(session.clone().unwrap_or_default())
                    } else {
                        A::AskWhy(ask_id.to_owned())
                    },
                    look,
                ))
                .children(session.map(|s| quiet(send, "todo-open-session", "Open session", A::OpenSession(s), look))),
        )
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
    fn an_ended_session_cannot_be_asked_why() {
        let s = state();
        let ended = s.items.iter().map(|e| &e.todo).find(|t| t.ask_detail.session_ended).expect("ended session");
        let live = s.items.iter().map(|e| &e.todo).find(|t| t.ask.is_some() && !t.ask_detail.session_ended).expect("live");
        assert_ne!(ask_why_label(ended), ask_why_label(live));
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
}
