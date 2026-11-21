//! The delete sheet for many sessions (MASSDELETE, D875; frames m1-2, m1-3,
//! m1-4, m2-5): the plan, what each session ends and loses, the count to
//! type, and one Delete. It draws `MassDeleteState.sheet` and nothing else.

use super::Overlays;
use super::fork::{card, heading};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::mass_delete::{
    DeleteScope, EndingState, SessionToDelete, TerminalEnding,
};
use humata2_clients::model::{ChannelId, MachineId, SessionId};
use humata2_mass_delete::{DeleteSheet, FooterHint, MassDeleteAction};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::chrome::{self, runs};
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

const ORANGE: u32 = 0xff9f0a;
const RED: u32 = 0xff453a;
const RED_FILL: u32 = 0xe5392d;
const SHELLS: [&str; 7] = ["zsh", "bash", "fish", "sh", "nu", "pwsh", "shell"];

/// The count field, and which channels' folded lines the person opened.
pub(super) struct Inputs {
    count: Entity<InputState>,
    /// Whether the count field has taken initial focus for the current open sheet.
    count_focused: Cell<bool>,
    opened: RefCell<BTreeSet<String>>,
    /// The plan token `opened` belongs to; a new plan or closing clears `opened`.
    plan_token: RefCell<Option<String>>,
}

impl Inputs {
    /// Makes the count field: typing sends `Typed`, Return deletes when the
    /// sheet allows it.
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<Overlays>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let count = cx.new(|cx| InputState::new(window, cx).placeholder(""));
        subscriptions.push(cx.subscribe(
            &count,
            |this: &mut Overlays, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    this.ui.send(AppAction::MassDelete(MassDeleteAction::Typed(text)), cx);
                }
                InputEvent::PressEnter { .. } => {
                    this.ui.send(AppAction::MassDelete(MassDeleteAction::Confirm), cx);
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        ));
        Self {
            count,
            count_focused: Cell::new(false),
            opened: RefCell::new(BTreeSet::new()),
            plan_token: RefCell::new(None),
        }
    }

    /// Clears the `Show`-expanded channels when the sheet closes or reopens.
    fn reset_opened(&self) {
        self.count_focused.set(false);
        self.opened.borrow_mut().clear();
        *self.plan_token.borrow_mut() = None;
    }

    /// Clears the `Show`-expanded channels whenever the plan resets or changes.
    fn sync_plan(&self, token: Option<&str>) {
        let mut current = self.plan_token.borrow_mut();
        if current.as_deref() != token {
            self.opened.borrow_mut().clear();
            if token.is_none() {
                self.count_focused.set(false);
            }
            *current = token.map(str::to_owned);
        }
    }
}

/// The sheet with the names of what it lists, read from the app's state.
#[derive(Clone)]
pub(super) struct View {
    sheet: DeleteSheet,
    machines: Vec<(MachineId, String)>,
    channels: Vec<(ChannelId, String)>,
    /// Every session the workspace knows: id, title, channel, parent, agent.
    known: Vec<Known>,
}

#[derive(Clone)]
struct Known {
    id: SessionId,
    title: String,
    channel: ChannelId,
    parent: Option<SessionId>,
    agent: &'static str,
    desk: bool,
}

impl View {
    /// The open sheet, if there is one.
    pub(super) fn read(state: &AppState) -> Option<Self> {
        let sheet = state.mass_delete.sheet.clone()?;
        let workspace = &state.workspace;
        Some(Self {
            sheet,
            machines: workspace
                .machines
                .iter()
                .map(|machine| (machine.id.clone(), machine.name.clone()))
                .collect(),
            channels: workspace
                .channels
                .iter()
                .map(|channel| (channel.id.clone(), channel.name.clone()))
                .collect(),
            known: workspace
                .sessions
                .iter()
                .map(|session| Known {
                    id: session.id.clone(),
                    title: session.title.clone(),
                    channel: session.channel.clone(),
                    parent: session.parent.clone(),
                    agent: runs(session.agent),
                    desk: workspace.desk.sessions.contains_key(&session.id),
                })
                .collect(),
        })
    }

    /// Whether the count field is what opens focused.
    fn counts(&self) -> bool {
        matches!(self.sheet.scope, DeleteScope::All)
    }

    fn machine(&self, id: &MachineId) -> String {
        self.machines
            .iter()
            .find(|(machine, _)| machine == id)
            .map_or_else(|| id.to_string(), |(_, name)| name.clone())
    }

    fn channel(&self, id: &ChannelId) -> String {
        self.channels
            .iter()
            .find(|(channel, _)| channel == id)
            .map_or_else(|| id.to_string(), |(_, name)| name.clone())
    }

    fn known(&self, id: &SessionId) -> Option<&Known> {
        self.known.iter().find(|session| &session.id == id)
    }

    fn title(&self, id: &SessionId) -> String {
        self.sheet
            .plan
            .iter()
            .flat_map(|plan| plan.sessions.iter())
            .find(|session| &session.session == id)
            .map(|session| session.title.clone())
            .or_else(|| self.known(id).map(|session| session.title.clone()))
            .unwrap_or_else(|| id.to_string())
    }

    /// How many sessions a channel holds, for "k of n".
    fn in_channel(&self, channel: &ChannelId) -> usize {
        self.known
            .iter()
            .filter(|session| &session.channel == channel && !session.desk)
            .count()
    }
}

impl Overlays {
    /// Puts focus where the sheet starts: the count field for delete-all,
    /// the sheet itself otherwise (Return deletes from there).
    pub(super) fn delete_sessions_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.delete_sessions.reset_opened();
        let counts = self
            .ui
            .read(|state| state.mass_delete.sheet.as_ref().is_some_and(|sheet| matches!(sheet.scope, DeleteScope::All)));
        if counts {
            self.delete_sessions.count.update(cx, |input, cx| input.focus(window, cx));
        } else {
            window.focus(&self.sheet, cx);
        }
    }

    /// The delete sheet.
    pub(super) fn delete_sessions_dialog(
        &self,
        view: &View,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sheet = &view.sheet;
        self.delete_sessions
            .sync_plan(sheet.plan.as_ref().map(|plan| plan.token.as_str()));
        super::sync(&self.delete_sessions.count, &sheet.typed, window, cx);
        let send = |action: MassDeleteAction| AppAction::MassDelete(action);
        let all = view.counts();
        let planned = sheet.planned_count();
        let mut body = card("delete-sessions-sheet", "Delete sessions", 700., look)
            .child(heading(title(sheet, planned)));
        if let Some(stale) = &sheet.stale {
            body = body.child(stale_notice(look, &stale.changes));
        }
        if sheet.loading {
            body = body.child(quiet(look, "Planning what this would delete\u{2026}"));
        }
        if let Some(plan) = &sheet.plan {
            body = body
                .child(quiet(look, scope_line(view, plan, all, planned)).text_size(px(12.5)))
                .child(totals(view, plan, look));
        }
        if let Some(error) = &sheet.error {
            body = body.child(widgets::error_banner_in(
                look,
                ui,
                "delete-sessions-error",
                error.clone(),
                send(MassDeleteAction::Cancel),
            ));
        }
        if sheet.plan.is_some() {
            body = body.child(self.list(view, look, ui, cx));
            if all {
                body = body.child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.5))
                        .text_color(rgb(look.tertiary))
                        .child("Desk sessions are not part of this. The Desk has its own Clean up."),
                );
            }
            body = body.child(trash_box(view, look));
            if all {
                if !self.delete_sessions.count_focused.replace(true) {
                    self.delete_sessions
                        .count
                        .update(cx, |input, cx| input.focus(window, cx));
                }
                body = body.child(self.count_field(sheet, look, window, cx));
            }
        }
        body = body.child(footer(view, look, ui));
        let (keys, dismiss, focus) = (ui.clone(), ui.clone(), self.sheet.clone());
        let body = body
            .track_focus(&self.sheet)
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "enter" && focus.is_focused(window) {
                    cx.stop_propagation();
                    keys.send(AppAction::MassDelete(MassDeleteAction::Confirm), cx);
                }
            })
            .on_action(move |_: &humata2_ui_core::keys::Dismiss, _, cx| {
                cx.stop_propagation();
                dismiss.send(AppAction::MassDelete(MassDeleteAction::Cancel), cx);
            })
            .test_support()
            .focus_trap("delete-sessions-trap", &self.sheet);
        div()
            .id("delete-sessions-dialog")
            .role(Role::Group)
            .aria_label("Delete sessions")
            .test_support()
            .child(body)
            .into_any_element()
    }

    fn count_field(
        &self,
        sheet: &DeleteSheet,
        look: Look,
        window: &Window,
        cx: &Context<Self>,
    ) -> Div {
        let focused = self
            .delete_sessions
            .count
            .read(cx)
            .focus_handle(cx)
            .is_focused(window);
        row()
            .flex_shrink_0()
            .gap(px(12.))
            .child(div().child(format!(
                "Type the number of sessions, {}, to confirm",
                sheet.planned_count()
            )))
            .child(
                row()
                    .id("delete-sessions-count")
                    .role(Role::Group)
                    .aria_label("Number of sessions")
                    .test_support()
                    .w(px(108.))
                    .h(px(28.))
                    .rounded(px(6.))
                    .bg(rgb(look.content))
                    .border_1()
                    .border_color(rgb(if focused { look.accent } else { look.line }))
                    .text_color(rgb(look.text))
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.delete_sessions.count)
                                .small()
                                .appearance(false)
                                .text_size(px(12.5))
                                .aria_label("Number of sessions"),
                        ),
                    ),
            )
    }

    /// The scrolling list, grouped by channel, and the unreachable group.
    fn list(&self, view: &View, look: Look, ui: &Rc<Ui>, cx: &mut Context<Self>) -> AnyElement {
        let sheet = &view.sheet;
        let Some(plan) = sheet.plan.as_ref() else {
            return div().into_any_element();
        };
        let all = view.counts();
        let scrollable = plan.sessions.len() > 3;
        let mut channels: Vec<ChannelId> = Vec::new();
        for session in &plan.sessions {
            if !channels.contains(&session.channel) {
                channels.push(session.channel.clone());
            }
        }
        let mut list = col()
            .id("delete-sessions-list")
            .role(Role::List)
            .aria_label("Sessions to delete")
            .test_support()
            .min_h_0()
            .overflow_y_scroll()
            .gap(px(12.))
            .py(px(10.))
            .border_t_1()
            .when(scrollable, |x| x.border_b_1())
            .border_color(rgb(look.line))
            .when(all, |x| x.max_h(px(296.)));
        for channel in channels {
            let members: Vec<&SessionToDelete> = plan
                .sessions
                .iter()
                .filter(|session| session.channel == channel)
                .collect();
            let opened = self.delete_sessions.opened.borrow().contains(channel.as_str());
            let (shown, folded): (Vec<&SessionToDelete>, Vec<&SessionToDelete>) = if all && !opened {
                members.iter().copied().partition(|session| !foldable(session))
            } else {
                (members.clone(), Vec::new())
            };
            let count = if all {
                let n = members.len();
                format!("{n} {}", if n == 1 { "session" } else { "sessions" })
            } else {
                let total = view.in_channel(&channel);
                format!(
                    "{} of {total} {}",
                    members.len(),
                    if total == 1 { "session" } else { "sessions" }
                )
            };
            let mut group = col()
                .gap(px(4.))
                .child(
                    row()
                        .gap(px(8.))
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("# {}", view.channel(&channel))))
                        .child(quiet(look, count)),
                );
            for line in ordered(&shown) {
                group = group.child(session_block(view, line, plan, look, ui));
            }
            if !folded.is_empty() {
                group = group.child(fold_line(&folded, !shown.is_empty(), &channel, look, cx));
            }
            // Forks that stay, whose parent is not in this delete: one line at the end.
            let orphans: Vec<String> = plan
                .forks_staying
                .iter()
                .filter(|work| {
                    let known = view.known(&work.session);
                    known.is_some_and(|k| k.channel == channel)
                        && !known
                            .and_then(|k| k.parent.as_ref())
                            .is_some_and(|parent| members.iter().any(|m| &m.session == parent))
                })
                .map(|work| work.title.clone())
                .collect();
            if !orphans.is_empty() {
                group = group.child(quiet(look, stay_line(&orphans)).pl(px(21.)));
            }
            list = list.child(group);
        }
        if all {
            for left in &plan.left_alone {
                list = list.child(unreachable_group(view, left, look));
            }
        }
        list.into_any_element()
    }
}

/// A session with nothing to lose: no terminals, no work, no fork riding along.
fn foldable(session: &SessionToDelete) -> bool {
    terminal_count(session) == 0 && session.unpushed.is_empty() && session.goes_with.is_none()
}

fn terminal_count(session: &SessionToDelete) -> usize {
    if session.terminals.is_empty() {
        usize::try_from(session.running_terminals).unwrap_or(usize::MAX)
    } else {
        session.terminals.len()
    }
}

/// Parents first, each followed by the forks that go with it.
fn ordered<'a>(sessions: &[&'a SessionToDelete]) -> Vec<&'a SessionToDelete> {
    let mut out: Vec<&SessionToDelete> = Vec::new();
    for session in sessions {
        let rides = session
            .goes_with
            .as_ref()
            .is_some_and(|parent| sessions.iter().any(|other| &other.session == parent));
        if rides {
            continue;
        }
        out.push(*session);
        out.extend(
            sessions
                .iter()
                .copied()
                .filter(|other| other.goes_with.as_ref() == Some(&session.session)),
        );
    }
    out
}

fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}

fn join(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

fn quiet(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}

fn title(sheet: &DeleteSheet, planned: u32) -> String {
    if sheet.plan.is_none() {
        return if matches!(sheet.scope, DeleteScope::All) {
            "Delete all sessions?".to_owned()
        } else {
            "Delete sessions?".to_owned()
        };
    }
    match (&sheet.scope, planned) {
        (DeleteScope::All, n) => format!("Delete all {n} sessions?"),
        (_, 1) => "Delete 1 session?".to_owned(),
        (_, n) => format!("Delete {n} sessions?"),
    }
}

fn scope_line(
    view: &View,
    plan: &humata2_clients::mass_delete::DeleteSessionsPlan,
    all: bool,
    planned: u32,
) -> String {
    let mut channels: Vec<&ChannelId> = Vec::new();
    let mut machines: Vec<String> = Vec::new();
    for session in &plan.sessions {
        if !channels.contains(&&session.channel) {
            channels.push(&session.channel);
        }
        let name = view.machine(&session.machine);
        if !machines.contains(&name) {
            machines.push(name);
        }
    }
    let channels = plural(channels.len(), "channel", "channels");
    let machines = join(&machines);
    if all {
        format!("Every session, live and archived: {planned} in {channels}, on {machines}.")
    } else {
        format!("In {channels}, on {machines}.")
    }
}

/// An icon beside a line of text.
fn lined(icon: IconName, color: u32, size: f32, text: impl Into<SharedString>, text_color: u32) -> Div {
    row()
        .items_start()
        .gap(px(7.))
        .child(div().mt(px(1.)).flex_shrink_0().child(chrome::icon(icon, color, size)))
        .child(div().flex_1().min_w_0().text_color(rgb(text_color)).child(text.into()))
}

/// The totals block, which stays outside the scroll area.
fn totals(
    view: &View,
    plan: &humata2_clients::mass_delete::DeleteSessionsPlan,
    look: Look,
) -> impl IntoElement {
    let ending: usize = plan.sessions.iter().map(terminal_count).sum();
    let with_terminals = plan.sessions.iter().filter(|s| terminal_count(s) > 0).count();
    let with_work = plan.sessions.iter().filter(|s| !s.unpushed.is_empty()).count();
    let along = plan.sessions.iter().filter(|s| s.goes_with.is_some()).count();
    let staying = plan.forks_staying.len();
    let mut block = col().id("delete-sessions-totals").test_support().flex_shrink_0().gap(px(6.));
    if ending > 0 {
        let head = if ending == 1 {
            "1 running terminal ends".to_owned()
        } else {
            format!("{ending} running terminals end")
        };
        block = block.child(lined(
            IconName::SquareTerminal,
            ORANGE,
            13.,
            format!("{head}, in {}.", plural(with_terminals, "session", "sessions")),
            ORANGE,
        ));
    }
    if with_work > 0 {
        let noun = if with_work == 1 { "session holds" } else { "sessions hold" };
        block = block.child(lined(
            IconName::GitBranch,
            ORANGE,
            13.,
            format!("{with_work} {noun} work that was never pushed."),
            ORANGE,
        ));
    }
    if along > 0 || staying > 0 {
        let mut text = Vec::new();
        if along == 1 {
            text.push("1 fork goes with its parent.".to_owned());
        } else if along > 1 {
            text.push(format!("{along} forks go with their parent."));
        }
        if staying == 1 {
            text.push("1 fork stays.".to_owned());
        } else if staying > 1 {
            text.push(format!("{staying} forks stay."));
        }
        block = block.child(lined(IconName::GitFork, look.text, 13., text.join(" "), look.text));
    }
    for left in &plan.left_alone {
        let machine = view.machine(&left.machine);
        let its = if left.sessions.len() == 1 {
            "Its 1 session is left alone.".to_owned()
        } else {
            format!("Its {} sessions are left alone.", left.sessions.len())
        };
        block = block.child(
            row()
                .gap(px(8.))
                .child(div().ml(px(3.)).size(px(8.)).rounded_full().flex_shrink_0().bg(rgb(look.tertiary)))
                .child(
                    div()
                        .text_color(rgb(look.secondary))
                        .child(format!("{machine} can\u{2019}t be reached. {its}")),
                ),
        );
    }
    block
}

fn stale_notice(look: Look, changes: &[String]) -> impl IntoElement {
    col()
        .id("delete-sessions-stale")
        .role(Role::Alert)
        .aria_label("Something changed since this plan")
        .test_support()
        .flex_shrink_0()
        .gap(px(5.))
        .p(px(14.))
        .rounded(px(8.))
        .bg(rgb(look.attention_bg))
        .text_color(rgb(ORANGE))
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Something changed since this plan. Nothing was deleted."),
        )
        .children(changes.iter().map(|line| div().child(line.clone())))
        .child(div().child("This is the new plan. Check it and delete again."))
}

/// What the stale notice says is new: terminals by name, and commits by branch.
struct Fresh {
    terminals: Vec<String>,
    commits: Vec<(String, String)>,
}

fn fresh(view: &View, session: &SessionToDelete) -> Fresh {
    let mut found = Fresh { terminals: Vec::new(), commits: Vec::new() };
    let Some(stale) = &view.sheet.stale else {
        return found;
    };
    let prefix = format!("{}: ", session.title);
    for line in &stale.changes {
        let Some(rest) = line.strip_prefix(&prefix) else {
            continue;
        };
        if let Some(after) = rest.strip_prefix("a new terminal started (") {
            if let Some((name, _)) = after.split_once(')') {
                found.terminals.push(name.to_owned());
            }
        } else if let Some((count, tail)) = rest.split_once(" new commit")
            && let Some((_, branch)) = tail.split_once(" on ")
        {
            found.commits.push((branch.trim_end_matches('.').to_owned(), count.to_owned()));
        }
    }
    found
}

fn terminal_line(terminals: &[TerminalEnding], running: u32, fresh: &Fresh) -> String {
    if terminals.is_empty() {
        let n = usize::try_from(running).unwrap_or(0);
        return format!("Ends {}.", plural(n, "running terminal", "running terminals"));
    }
    let mut new = fresh.terminals.clone();
    let mut items: Vec<String> = terminals
        .iter()
        .rev()
        .map(|terminal| {
            let state = match terminal.state {
                EndingState::Working => " (working)",
                EndingState::WaitingForYou => " (waiting for you)",
                EndingState::Idle if SHELLS.contains(&terminal.name.as_str()) => "",
                EndingState::Idle => " (idle)",
            };
            let marker = new
                .iter()
                .position(|name| name == &terminal.name)
                .map(|at| new.remove(at))
                .map_or("", |_| " (new)");
            format!("{}{state}{marker}", terminal.name)
        })
        .collect();
    items.reverse();
    format!(
        "Ends {}: {}.",
        plural(terminals.len(), "running terminal", "running terminals"),
        items.join(", ")
    )
}

/// A row with the lines under it.
fn session_block(
    view: &View,
    session: &SessionToDelete,
    plan: &humata2_clients::mass_delete::DeleteSessionsPlan,
    look: Look,
    ui: &Rc<Ui>,
) -> impl IntoElement {
    let rides = session.goes_with.is_some();
    let indent = if rides { 24. } else { 0. };
    let agent = view.known(&session.session).map_or("", |known| known.agent);
    let machine = view.machine(&session.machine);
    let ran = if agent.is_empty() { machine } else { format!("{agent} \u{b7} {machine}") };
    let fresh = fresh(view, session);
    let mut block = col()
        .id(SharedString::from(format!("delete-session-{}", session.session.as_str())))
        .role(Role::ListItem)
        .aria_label(session.title.clone())
        .test_support()
        .gap(px(3.))
        .pl(px(indent))
        .child(
            row()
                .gap(px(8.))
                .child(div().flex_shrink_0().child(chrome::icon(
                    if rides { IconName::GitFork } else { IconName::SquareTerminal },
                    look.secondary,
                    13.,
                )))
                .child(
                    row()
                        .flex_1()
                        .min_w_0()
                        .gap(px(8.))
                        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().child(session.title.clone()))
                        .when(view.sheet.is_changed(&session.session), |x| {
                            x.child(
                                div()
                                    .flex_shrink_0()
                                    .px(px(6.))
                                    .rounded(px(4.))
                                    .border_1()
                                    .border_color(rgb(ORANGE))
                                    .text_size(px(11.))
                                    .text_color(rgb(ORANGE))
                                    .child("changed"),
                            )
                        }),
                )
                .child(quiet(look, ran).w(px(170.)).flex_shrink_0().whitespace_nowrap())
                .child(
                    quiet(look, if session.archived { "archived" } else { "live" })
                        .w(px(56.))
                        .flex_shrink_0()
                        .flex()
                        .justify_end(),
                ),
        );
    let mut lines = col().gap(px(3.)).pl(px(21.));
    let mut any = false;
    if let Some(parent) = &session.goes_with {
        any = true;
        lines = lines.child(quiet(
            look,
            format!("Goes with {}: this fork works in the same folder.", view.title(parent)),
        ));
    }
    if terminal_count(session) > 0 {
        any = true;
        lines = lines.child(lined(
            IconName::SquareTerminal,
            ORANGE,
            11.,
            terminal_line(&session.terminals, session.running_terminals, &fresh),
            ORANGE,
        ).text_size(px(12.)));
    }
    for work in &session.unpushed {
        any = true;
        let new = fresh
            .commits
            .iter()
            .find(|(branch, _)| work.what.contains(branch.as_str()))
            .map(|(_, count)| format!(" ({count} new)"))
            .unwrap_or_default();
        lines = lines.child(lined(
            IconName::GitBranch,
            ORANGE,
            11.,
            format!("{}: {}{new}.", work.repo, work.what.trim_end_matches('.')),
            ORANGE,
        ).text_size(px(12.)));
    }
    if !session.unpushed.is_empty() {
        lines = lines.child(work_box(
            ui,
            look,
            &session.session,
            view.sheet.work_ticks.contains(&session.session),
            !view.sheet.deleting,
        ));
    }
    if !any {
        lines = lines.child(quiet(look, "Nothing running. Nothing unpushed."));
    }
    block = block.child(lines);
    // The forks that stay: one quiet line after the parent they came from.
    let stay: Vec<String> = plan
        .forks_staying
        .iter()
        .filter(|work| {
            view.known(&work.session)
                .and_then(|known| known.parent.as_ref())
                .is_some_and(|parent| parent == &session.session)
        })
        .map(|work| work.title.clone())
        .collect();
    if !stay.is_empty() && !rides {
        block = block.child(quiet(look, stay_line(&stay)).pl(px(21.)));
    }
    block
}

fn stay_line(names: &[String]) -> String {
    if names.len() == 1 {
        format!("{} stays. This fork has its own folder; it moves up in the tree.", names[0])
    } else {
        format!(
            "{} stay. These forks have their own folders; they move up in the tree.",
            join(names)
        )
    }
}

/// "Delete that work too".
fn work_box(ui: &Rc<Ui>, look: Look, session: &SessionId, on: bool, enabled: bool) -> impl IntoElement {
    let ui = ui.clone();
    let action = AppAction::MassDelete(MassDeleteAction::TickWork(session.clone()));
    row()
        .id(SharedString::from(format!("delete-work-{}", session.as_str())))
        .role(Role::CheckBox)
        .aria_label("Delete that work too")
        .aria_selected(on)
        .test_support()
        .gap(px(8.))
        .py(px(2.))
        .child(
            row()
                .size(px(16.))
                .flex_shrink_0()
                .rounded(px(4.))
                .justify_center()
                .when(on, |x| x.bg(rgb(look.accent)).child(chrome::icon(IconName::Check, 0xffffff, 11.)))
                .when(!on, |x| x.border_1().border_color(rgb(look.tertiary)).bg(rgb(look.card))),
        )
        .child(div().text_size(px(13.)).child("Delete that work too"))
        .when(enabled, |x| {
            x.cursor_pointer().on_click(move |_, _, cx| ui.send(action.clone(), cx))
        })
}

/// One quiet line for the sessions with nothing to lose, with Show.
fn fold_line(
    folded: &[&SessionToDelete],
    more: bool,
    channel: &ChannelId,
    look: Look,
    cx: &mut Context<Overlays>,
) -> Div {
    let k = folded.len();
    let archived = folded.iter().filter(|session| session.archived).count();
    let live: Vec<String> = folded
        .iter()
        .filter(|session| !session.archived)
        .map(|session| session.title.clone())
        .collect();
    let all_archived = archived == k;
    let names = if k <= 3 {
        Some(join(&folded.iter().map(|s| s.title.clone()).collect::<Vec<_>>()))
    } else if !all_archived && live.len() <= 3 {
        Some(format!("{} and {archived} archived", join(&live)))
    } else {
        None
    };
    let head = if all_archived {
        format!("{k} archived, nothing unpushed")
    } else {
        let lead = if more {
            format!("{k} more")
        } else {
            format!("{k} {}", if k == 1 { "session" } else { "sessions" })
        };
        format!("{lead}, nothing running and nothing unpushed")
    };
    let text = names.map_or_else(|| head.clone(), |names| format!("{head}: {names}"));
    let key = channel.as_str().to_owned();
    let mut line = row().gap(px(4.)).child(quiet(look, text).text_size(px(12.)));
    if k > 3 {
        line = line
            .child(quiet(look, "\u{b7}"))
            .child(
                div()
                    .id(SharedString::from(format!("delete-show-{key}")))
                    .role(Role::Button)
                    .aria_label("Show")
                    .test_support()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(rgb(look.accent))
                    .child("Show")
                    .on_click(cx.listener(move |this: &mut Overlays, _, _, cx| {
                        this.delete_sessions.opened.borrow_mut().insert(key.clone());
                        cx.notify();
                    })),
            );
    }
    line
}

/// A machine that can't be reached, and the sessions a delete leaves on it.
fn unreachable_group(view: &View, left: &humata2_clients::mass_delete::LeftAlone, look: Look) -> Div {
    let machine = view.machine(&left.machine);
    let names: Vec<String> = left
        .sessions
        .iter()
        .map(|work| match view.known(&work.session) {
            Some(known) => format!("{} (# {})", work.title, view.channel(&known.channel)),
            None => work.title.clone(),
        })
        .collect();
    col()
        .gap(px(4.))
        .child(
            row()
                .gap(px(8.))
                .child(div().size(px(8.)).rounded_full().flex_shrink_0().bg(rgb(look.tertiary)))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(machine.clone()))
                .child(quiet(
                    look,
                    format!(
                        "can\u{2019}t reach, left alone \u{b7} {}",
                        plural(left.sessions.len(), "session", "sessions")
                    ),
                )),
        )
        .child(
            quiet(
                look,
                format!(
                    "Not deleted: {}. Delete them when {machine} is back.",
                    names.join(", ")
                ),
            )
            .pl(px(16.)),
        )
}

/// The grey box that says what a delete does for good.
fn trash_box(view: &View, look: Look) -> impl IntoElement {
    let mut machines: Vec<String> = Vec::new();
    for session in view.sheet.plan.iter().flat_map(|plan| plan.sessions.iter()) {
        let name = view.machine(&session.machine);
        if !machines.contains(&name) {
            machines.push(name);
        }
    }
    div()
        .id("delete-sessions-trash")
        .test_support()
        .flex_shrink_0()
        .p(px(12.))
        .rounded(px(8.))
        .bg(rgb(look.fill))
        .text_size(px(13.))
        .text_color(rgb(look.secondary))
        .child(format!(
            "The terminals end for good. The sessions go to the trash on {} for 30 days: restore one \
             from the machine\u{2019}s Storage > Trash and it comes back archived, with its work. There is no Undo.",
            join(&machines)
        ))
}

fn hint(view: &View, hint: Option<FooterHint>) -> String {
    let sheet = &view.sheet;
    if sheet.deleting {
        return "Deleting\u{2026}".to_owned();
    }
    match hint {
        None => String::new(),
        Some(FooterHint::TickOne(id)) => {
            format!("Tick \u{201c}Delete that work too\u{201d} for {} to continue", view.title(&id))
        }
        Some(FooterHint::TickMany(k)) => format!(
            "Tick \u{201c}Delete that work too\u{201d} for {k} sessions to continue"
        ),
        Some(FooterHint::TickAndType { sessions, count }) => format!(
            "Tick \u{201c}Delete that work too\u{201d} for {sessions} sessions and type {count} to continue"
        ),
        Some(FooterHint::Type(count)) => format!("Type {count} to continue"),
        Some(FooterHint::Ready) => "Return deletes".to_owned(),
        Some(FooterHint::NothingReachable) => {
            "No session can be reached, so nothing can be deleted yet".to_owned()
        }
    }
}

/// The hint, Cancel and Delete.
fn footer(view: &View, look: Look, ui: &Rc<Ui>) -> Div {
    let sheet = &view.sheet;
    let live = sheet.can_confirm();
    let all = view.counts();
    let planned = sheet.planned_count();
    let label = if all {
        format!("Delete all {planned} sessions")
    } else if planned == 1 {
        "Delete 1 session".to_owned()
    } else if planned == 0 {
        "Delete sessions".to_owned()
    } else {
        format!("Delete {planned} sessions")
    };
    let confirm = widgets::push_quiet_shape("delete-sessions-confirm", label, look);
    let confirm = if !live {
        confirm.bg(rgb(look.fill)).text_color(rgb(look.tertiary)).disabled(true)
    } else if all {
        confirm.bg(rgb(RED_FILL)).border_color(rgb(RED_FILL)).text_color(rgb(0xffffff))
    } else {
        confirm.bg(rgb(look.card)).border_color(rgb(RED)).text_color(rgb(RED))
    };
    let confirm = if live {
        let ui = ui.clone();
        confirm.on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(AppAction::MassDelete(MassDeleteAction::Confirm), cx);
        })
    } else {
        confirm
    };
    row()
        .flex_shrink_0()
        .gap(px(8.))
        .pt(px(4.))
        .child(
            div()
                .id("delete-sessions-hint")
                .test_support()
                .flex_1()
                .min_w_0()
                .text_size(px(11.5))
                .text_color(rgb(look.tertiary))
                .child(hint(view, sheet.hint())),
        )
        .child(
            widgets::push_quiet(
                ui,
                "delete-sessions-cancel",
                "Cancel",
                AppAction::MassDelete(MassDeleteAction::Cancel),
                look,
            )
            .disabled(sheet.deleting),
        )
        .child(confirm)
}
