//! The real session's terminal/file row and its only split: two tab groups.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, ObservedElement, TestTextExt as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::model::{Agent, MoveProgress, MoveStepState, Session, Status, Turn};
use humata2_inbox::InboxAction;
use humata2_session::{
    GroupWidth, SessionAction, SessionRefused, Tab, TabGroup, group_widths, is_markdown,
};

use crate::{
    bridge::Ui,
    chrome,
    code_editor,
    theme::{GREEN, Look, ORANGE, RED_STATUS},
    widgets::{self, col, row},
};

fn send(ui: &Rc<Ui>, id: String, label: &str, action: SessionAction, look: Look) -> Stateful<Div> {
    let ui = ui.clone();
    div()
        .id(SharedString::from(id))
        .role(Role::Button)
        .aria_label(label.to_owned())
        .cursor_pointer()
        .text_color(rgb(look.secondary))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            ui.send(AppAction::Session(action.clone()), cx);
        })
}

/// A tab being dragged to the other group (SPEC 26.20): where it is now.
#[derive(Clone)]
struct TabDrag {
    group: usize,
    index: usize,
    label: String,
}

impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px(px(10.))
            .py(px(5.))
            .rounded(px(6.))
            .bg(rgb(0x323235))
            .text_size(px(12.))
            .text_color(rgb(0xffffff))
            .child(self.label.clone())
    }
}

fn tab_row(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    group: usize,
    tabs: &TabGroup,
    look: Look,
    terminals: &crate::terminals::Terminals,
) -> Stateful<Div> {
    let split = state.session.view().is_some_and(|v| v.groups.len() == 2);
    // In a split, only the focused group's tab reads lit.
    let focused = !split || state.session.view().is_some_and(|v| v.focused_group == group);
    row()
        .id(SharedString::from(format!("session-tabs-{group}")))
        .h(px(32.))
        .flex_shrink_0()
        .overflow_x_scroll()
        .bg(rgb(look.panel))
        .border_b_1()
        .border_color(rgb(look.line))
        .children(tabs.tabs.iter().enumerate().map(|(index, tab)| {
            let (name, glyph, id) = match tab {
                Tab::Terminal(tab) => (
                    if *tab == 0 {
                        session.agent.command().to_owned()
                    } else {
                        session
                            .shells
                            .get(tab - 1)
                            .cloned()
                            .unwrap_or_else(|| "terminal".into())
                    },
                    if *tab == 0 || session.terminal_agents.contains_key(tab) {
                        chrome::status(if *tab == 0 {
                            session.status
                        } else {
                            Status::Working
                        })
                        .into_any_element()
                    } else {
                        chrome::icon(IconName::SquareTerminal, look.secondary, 12.)
                            .into_any_element()
                    },
                    if group == 0 {
                        format!("terminal-tab {tab}")
                    } else {
                        format!("terminal-tab-{group}-{tab}")
                    },
                ),
                Tab::File { path, .. } => (
                    path.rsplit('/').next().unwrap_or(path).to_owned(),
                    chrome::icon(
                        if is_markdown(path) {
                            IconName::FileText
                        } else {
                            IconName::FileCode
                        },
                        look.secondary,
                        12.,
                    )
                    .into_any_element(),
                    format!("file-tab-{group}-{index}"),
                ),
                Tab::View(id) => (
                    crate::view_tab::tab_label(state, id),
                    chrome::icon(IconName::Sparkles, 0xbf5af2, 12.).into_any_element(),
                    format!("view-tab-{group}-{index}"),
                ),
                Tab::Conversation => (
                    "Conversation".to_owned(),
                    chrome::icon(IconName::FileText, look.secondary, 12.).into_any_element(),
                    format!("conversation-tab-{group}-{index}"),
                ),
            };
            let ui2 = ui.clone();
            // Each tab's X is named for its tab (D784).
            let close_label = format!("Close {name}");
            let drag = TabDrag {
                group,
                index,
                label: name.clone(),
            };
            row()
                .id(SharedString::from(id))
                .role(Role::Tab)
                .aria_label(name.clone())
                .aria_selected(index == tabs.selected)
                .h_full()
                .flex_shrink_0()
                .px(px(12.))
                .gap(px(6.))
                .border_r_1()
                .border_color(rgb(look.line))
                .when(index == tabs.selected, |t| t.bg(rgb(look.term)))
                .text_size(px(12.))
                .text_color(rgb(if index == tabs.selected && focused {
                    look.text
                } else {
                    look.secondary
                }))
                .child(glyph)
                .child(name)
                .child({
                    // D784: a terminal's close asks Keep running, Stop or
                    // End for good; any other tab just closes.
                    let close = if matches!(tab, Tab::Terminal(_)) {
                        SessionAction::Close(humata2_session::CloseAction::Ask { group, index })
                    } else {
                        SessionAction::CloseTab { group, index }
                    };
                    send(ui, format!("close-tab-{group}-{index}"), &close_label, close, look)
                        .child(chrome::icon(IconName::X, look.tertiary, 12.))
                        .test_support()
                })
                .on_click(move |_, _, cx| {
                    ui2.send(
                        AppAction::Session(SessionAction::SelectGroupTab { group, index }),
                        cx,
                    )
                })
                .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
                .test_support()
        }))
        .child(
            send(
                ui,
                format!("add-tab-{group}"),
                "New terminal",
                SessionAction::TerminalMenu(if state.session.terminal_menu == Some(group) {
                    None
                } else {
                    Some(group)
                }),
                look,
            )
            .px(px(10.))
            .h_full()
            .flex()
            .items_center()
            .child(chrome::icon(IconName::Plus, look.secondary, 12.))
            .test_support(),
        )
        .child(div().flex_1())
        .children(crate::document_tab::mode_switch(ui, state, look, group, tabs))
        .when(session.status != Status::Offline, |bar| {
            let peers = tabs
                .tabs
                .get(tabs.selected)
                .and_then(|tab| match tab {
                    // The link's count once it has told one; the
                    // workspace's (a fixture's) before.
                    Tab::Terminal(tab) => terminals
                        .other_writers(&session.id, *tab)
                        .map(|others| usize::try_from(others).unwrap_or(usize::MAX))
                        .or_else(|| {
                            state
                                .workspace
                                .terminal_peers
                                .get(&(session.id.clone(), *tab))
                                .copied()
                        }),
                    _ => None,
                })
                .unwrap_or(0);
            bar.when(
                session.status == Status::Working && peers > 0 && !split,
                |bar| {
                    let label = format!(
                        "{peers} other{} can type here",
                        if peers == 1 { "" } else { "s" }
                    );
                    bar.child(
                        div()
                            .id(SharedString::from(format!("terminal-peers-{group}")))
                            .mr(px(12.))
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(label.clone())
                            .test_text(label)
                            .test_support(),
                    )
                },
            )
        })
        .when(!split && session.status == Status::Offline, |bar| {
            bar.child(
                row()
                    .mr(px(12.))
                    .gap(px(5.))
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(chrome::icon(IconName::WifiOff, look.secondary, 12.))
                    .child("disconnected"),
            )
        })
        .drag_over::<TabDrag>(move |style, _, _, _| style.bg(rgba(look.accent << 8 | 0x26)))
        .on_drop({
            let ui = ui.clone();
            move |drag: &TabDrag, _, cx| {
                // Dropped on its own row, a tab stays where it is.
                if drag.group != group {
                    ui.send(
                        AppAction::Session(SessionAction::MoveTab {
                            group: drag.group,
                            index: drag.index,
                        }),
                        cx,
                    );
                }
            }
        })
}

/// One row of a moving session's step card.
#[derive(Debug, PartialEq)]
struct StepRow {
    mark: &'static str,
    state: MoveStepState,
    title: String,
    /// The running step's bar; none on every other row.
    bar: Option<StepBar>,
}

/// The bar under the running step.
#[derive(Debug, PartialEq)]
enum StepBar {
    /// The copy's share so far, with its text under the bar.
    Measured(CopyProgress),
    /// The move has not reported units yet, so the bar sweeps.
    Sweep,
}

/// A thin bar's fill and its text under the running step.
#[derive(Debug, PartialEq)]
struct CopyProgress {
    /// 0..=1 of the card's width.
    fraction: f32,
    label: String,
}

/// `done` of `total` bytes; none when the total is unknown (0).
fn copy_progress(done: u64, total: u64) -> Option<CopyProgress> {
    const MB: u64 = 1_000_000;
    if total == 0 {
        return None;
    }
    let done = done.min(total);
    Some(CopyProgress {
        fraction: (done as f64 / total as f64) as f32,
        label: format!("{} of {} MB", done / MB, total / MB),
    })
}

/// The bar under a step: only the running step has one, measured when the
/// move reports its units and swept until then.
fn step_bar(state: MoveStepState, done: u64, total: u64) -> Option<StepBar> {
    if state != MoveStepState::Running {
        return None;
    }
    Some(match copy_progress(done, total) {
        Some(progress) => StepBar::Measured(progress),
        None => StepBar::Sweep,
    })
}

/// The card's rows, one per step in order (frame 64: done ✓, running ●,
/// waiting ○).
fn step_rows(moving: &MoveProgress) -> Vec<StepRow> {
    moving
        .steps
        .iter()
        .map(|step| StepRow {
            mark: match step.state {
                MoveStepState::Done => "✓",
                MoveStepState::Running => "●",
                MoveStepState::Failed => "✕",
                MoveStepState::Pending | MoveStepState::Skipped | MoveStepState::Undone => "○",
            },
            state: step.state,
            title: step.title.clone(),
            bar: step_bar(step.state, moving.done_units, moving.total_units),
        })
        .collect()
}

/// The thin bar's size under a step: 180 px wide, 3 px tall.
fn step_bar_metrics(look: Look) -> widgets::BarMetrics {
    widgets::BarMetrics {
        width: Some(px(180.)),
        height: px(3.),
        radius: px(2.),
        track: look.line,
        fill: look.accent,
    }
}

/// Frame 64's card: "Moving to <machine>" and every step with its state.
fn steps_card(look: Look, to: &str, moving: &MoveProgress) -> AnyElement {
    col()
        .w_full()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .text_size(px(12.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Moving to {to}")))
        .children(step_rows(moving).into_iter().enumerate().map(|(index, row)| {
            let (color, dim) = match row.state {
                MoveStepState::Done => (GREEN, false),
                MoveStepState::Running => (ORANGE, false),
                MoveStepState::Failed => (RED_STATUS, false),
                MoveStepState::Pending | MoveStepState::Skipped | MoveStepState::Undone => (look.tertiary, true),
            };
            widgets::row()
                .gap(px(8.))
                .whitespace_nowrap()
                .child(div().w(px(14.)).flex().justify_center().text_color(rgb(color)).child(row.mark))
                .child(
                    col()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_color(rgb(if dim { look.secondary } else { look.text }))
                                .child(row.title),
                        )
                        .children(row.bar.map(|bar| match bar {
                            StepBar::Measured(progress) => col()
                                .gap(px(3.))
                                .child(widgets::progress_bar(
                                    look,
                                    Some(progress.fraction),
                                    step_bar_metrics(look),
                                    ("move-copy", index),
                                ))
                                .child(div().text_color(rgb(look.secondary)).child(progress.label))
                                .into_any_element(),
                            StepBar::Sweep => widgets::progress_bar(
                                look,
                                None,
                                step_bar_metrics(look),
                                ("move-sweep", index),
                            ),
                        })),
                )
        }))
        .into_any_element()
}

fn notice(
    look: Look,
    glyph: IconName,
    color: u32,
    text: String,
    action: Option<AnyElement>,
) -> AnyElement {
    // Terminals::body places it under the terminal's content.
    // A status line a screen reader (and the view) reads, as the pane banner
    // it stands in for is.
    row()
        .id(SharedString::from(format!("tab-notice {text}")))
        .role(Role::Status)
        .aria_label(text.clone())
        .w_full()
        .px(px(10.))
        .py(px(7.))
        .gap(px(8.))
        .items_center()
        .rounded(px(7.))
        .bg(rgb(look.fill))
        .text_color(rgb(look.text))
        .text_size(px(12.))
        .child(chrome::icon(glyph, color, 12.))
        .child(div().flex_1().min_w_0().child(text))
        .children(action)
        .test_support()
        .into_any_element()
}

fn menu(ui: &Rc<Ui>, group: usize, look: Look) -> Stateful<Div> {
    // AGENTSET (D725): the enabled agents, then the plain shell.
    let offered: Vec<Agent> = ui
        .read(|state| state.settings.offered_agents())
        .into_iter()
        .chain([Agent::Shell])
        .collect();
    crate::action_menu::card("new-terminal-menu", 280., look)
        .top(px(32.))
        .left(px(8.))
        .children(offered.into_iter().map(|agent| {
            let name = if agent == Agent::Shell {
                "New terminal"
            } else {
                agent.name()
            };
            row()
                .gap(px(6.))
                .child(
                    send(
                        ui,
                        format!("new-terminal-{group}-{}", agent.command()),
                        name,
                        SessionAction::OpenTerminal {
                            agent,
                            split: false,
                        },
                        look,
                    )
                    .flex_1()
                    .px(px(8.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .hover(move |x| x.bg(rgb(look.fill)))
                    .text_color(rgb(look.text))
                    .child(name)
                    .test_support(),
                )
                .child(
                    send(
                        ui,
                        format!("split-terminal-{group}-{}", agent.command()),
                        "Open in second group",
                        SessionAction::OpenTerminal { agent, split: true },
                        look,
                    )
                    .px(px(6.))
                    .py(px(5.))
                    .rounded(px(4.))
                    .hover(move |x| x.bg(rgb(look.fill)))
                    .child(chrome::icon(IconName::Columns2, look.secondary, 14.))
                    .test_support(),
                )
        }))
        .child(
            send(
                ui,
                "close-terminal-menu".into(),
                "Cancel",
                SessionAction::TerminalMenu(None),
                look,
            )
            .px(px(8.))
            .py(px(5.))
            .child("Cancel")
            .test_support(),
        )
}

fn file_body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    repo: &str,
    path: &str,
    documents: &crate::document_tab::Documents,
    look: Look,
) -> AnyElement {
    let content =
        state
            .session
            .file_contents
            .get(&(session.id.clone(), repo.to_owned(), path.to_owned()));
    if let (Some(Ok(text)), Some(diff)) = (
        content,
        code_editor::diff(state, &session.id, repo, path),
    ) {
        return code_editor::body(
            ui,
            state,
            session,
            repo,
            path,
            text,
            diff,
            documents,
            look,
        );
    }
    col()
        .id("session-file")
        .role(Role::Document)
        .aria_label(format!("{repo}/{path}"))
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .bg(rgb(look.term))
        .text_color(rgb(look.text))
        .child(
            div()
                .px(px(14.))
                .py(px(8.))
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(format!("{repo}/{path}")),
        )
        .child(match content {
            Some(Ok(text)) => col()
                .font_family("Menlo")
                .text_size(px(12.))
                .line_height(px(18.))
                .children(text.lines().enumerate().map(|(index, line)| {
                    row()
                        .gap(px(12.))
                        .px(px(12.))
                        .child(
                            div()
                                .w(px(30.))
                                .text_right()
                                .text_color(rgb(look.tertiary))
                                .child((index + 1).to_string()),
                        )
                        .child(
                            div()
                                .whitespace_nowrap()
                                .child(line.to_owned())
                                .test_text(line.to_owned()),
                        )
                }))
                .into_any_element(),
            Some(Err(error)) => div()
                .p(px(14.))
                .child(error.to_string())
                .test_text(error.to_string())
                .into_any_element(),
            None => div()
                .id("file-loading")
                .p(px(14.))
                .role(Role::ProgressIndicator)
                .aria_label("Loading")
                .child(widgets::progress_bar(
                    look,
                    None,
                    widgets::BarMetrics::line(look),
                    "file-loading-sweep",
                ))
                .into_any_element(),
        })
        .test_support()
        .into_any_element()
}

/// When the daemon refused the session's last action until something is
/// migrated, say so inline (NV-10, frame 82), with the way to the item. It
/// draws from the refusal the session recorded (D377), nothing else.
fn migrate_first(ui: &Rc<Ui>, state: &AppState, look: Look) -> Option<ObservedElement<Stateful<Div>>> {
    let SessionRefused::MigrationNeeded { thing, machine, request } =
        state.session.refused.as_ref()?;
    let machine = state
        .workspace
        .machine(machine)
        .map_or_else(|| machine.to_string(), |machine| machine.name.clone());
    let text = format!(
        "Migrate the {} on {machine} first. What you type isn't sent until then; the agent keeps running.",
        thing.label()
    );
    let (ui, id) = (ui.clone(), request.clone());
    Some(
        row()
            .id("session-migrate-first")
            .role(Role::Alert)
            .aria_label(text.clone())
            .flex_shrink_0()
            .gap_3()
            .px_3()
            .py_2()
            .rounded(px(6.))
            .bg(rgb(look.attention_bg))
            .child(div().flex_shrink_0().child(chrome::icon(IconName::Inbox, ORANGE, 12.)))
            .child(div().flex_1().min_w_0().text_color(rgb(look.text)).child(text))
            .child(
                div()
                    .id("session-open-in-inbox")
                    .role(Role::Button)
                    .aria_label("Open in Inbox")
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_color(rgb(look.accent))
                    .child("Open in Inbox")
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::Navigate(Route::Inbox), cx);
                        if let Some(id) = &id {
                            ui.send(AppAction::Inbox(InboxAction::Select(id.clone())), cx);
                        }
                    })
                    .test_support(),
            )
            .test_support(),
    )
}

pub(crate) fn body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
    terminals: &crate::terminals::Terminals,
    documents: &crate::document_tab::Documents,
    cx: &mut App,
) -> Div {
    let fallback = TabGroup {
        tabs: (0..=session.shells.len()).map(Tab::Terminal).collect(),
        selected: state.session.view().map_or(0, |v| v.tab),
    };
    let dragging = cx.has_active_drag();
    let groups = state
        .session
        .view()
        .map(|v| v.groups.as_slice())
        .filter(|g| !g.is_empty())
        .unwrap_or(std::slice::from_ref(&fallback));
    // A terminal beside a Side-by-side document is a fixed width; the row's
    // own width is not known here, so the half-row limit is applied by the
    // group's max width.
    let widths = group_widths(
        f32::INFINITY,
        &(0..groups.len())
            .map(|group| {
                state
                    .session
                    .view()
                    .is_some_and(|v| v.selected_is_side_by_side(group))
            })
            .collect::<Vec<_>>(),
    );
    col()
        .size_full()
        .children(crate::view_tab::bars(ui, state, session, look))
        .children(state.session.error.clone().map(|error| {
            widgets::error_banner_in(look,
                ui,
                "session-error",
                error,
                AppAction::Session(SessionAction::DismissError),
            )
        }))
        .children(migrate_first(ui, state, look))
        .children(
            crate::desk_bar::bar(ui, state, session, look)
                .or_else(|| crate::desk_bar::channel_bar(ui, state, session, look)),
        )
        .child(
            row().flex_1().min_h_0().items_stretch().children(
                groups
                    .iter()
                    .enumerate()
                    .map(|(group, tabs)| {
                        let selected = tabs.tabs.get(tabs.selected);
                        let width = widths.get(group).copied().unwrap_or(GroupWidth::Flexible);
                        col()
                            .relative()
                            .map(|g| match width {
                                GroupWidth::Fixed(points) => {
                                    g.w(px(points)).max_w(relative(0.5)).flex_shrink_0()
                                }
                                GroupWidth::Flexible => g.flex_1(),
                            })
                            .min_w_0()
                            .h_full()
                            .overflow_hidden()
                            .when(group > 0, |g| g.border_l_1().border_color(rgb(look.line)))
                            .child(tab_row(ui, state, session, group, tabs, look, terminals))
                            // One group: while a tab is dragged, dropping it on the
                            // right edge opens the second group beside it. The zone
                            // exists only during a drag, so it never takes the
                            // terminal's clicks.
                            .when(groups.len() == 1 && tabs.tabs.len() > 1 && dragging, |g| {
                                let ui = ui.clone();
                                g.child(
                                    div()
                                        .id("tab-drop-right")
                                        .role(Role::Region)
                                        .aria_label("Drop the tab beside the terminal")
                                        .absolute()
                                        .top(px(32.))
                                        .right(px(0.))
                                        .bottom(px(0.))
                                        .w(px(96.))
                                        .drag_over::<TabDrag>(move |style, _, _, _| {
                                            style.bg(rgba(look.accent << 8 | 0x26))
                                        })
                                        .on_drop(move |drag: &TabDrag, _, cx| {
                                            ui.send(
                                                AppAction::Session(SessionAction::MoveTab {
                                                    group: drag.group,
                                                    index: drag.index,
                                                }),
                                                cx,
                                            )
                                        })
                                        .test_support(),
                                )
                            })
                            .child(match selected {
                                Some(Tab::File { repo, path }) => documents
                                    .body(ui, state, session, repo, path, look)
                                    .filter(|_| is_markdown(path))
                                    .unwrap_or_else(|| {
                                        file_body(ui, state, session, repo, path, documents, look)
                                    }),
                                Some(Tab::View(id)) => {
                                    crate::view_tab::body(ui, state, session, id, look)
                                }
                                Some(Tab::Terminal(tab)) => {
                                    let notice = match (*tab, session.status) {
                                        (0, Status::Exited { code }) => Some(notice(
                                            look,
                                            IconName::CircleX,
                                            0xff453a,
                                            format!(
                                                "{} exited with code {code}.{}",
                                                session.agent.command(),
                                                if offers_resume(session) {
                                                    " Resume starts it again with this conversation."
                                                } else {
                                                    ""
                                                }
                                            ),
                                            offers_resume(session).then(|| {
                                                widgets::send(
                                                    ui,
                                                    "resume-terminal",
                                                    "Resume",
                                                    AppAction::Session(SessionAction::Resume),
                                                )
                                                .outline()
                                                .h(px(24.))
                                                .px(px(11.))
                                                .into_any_element()
                                            }),
                                        )),
                                        // Frame 64: read-only until the agent resumes.
                                        (0, Status::Moving) => {
                                            let to = session
                                                .moving
                                                .as_ref()
                                                .and_then(|moving| {
                                                    state.workspace.machines.iter().find(|x| x.id == moving.to)
                                                })
                                                .map_or("the new machine", |x| x.name.as_str());
                                            let read_only = notice(
                                                look,
                                                IconName::ArrowRight,
                                                0xff9f0a,
                                                format!("Read-only until the agent resumes on {to}."),
                                                None,
                                            );
                                            Some(match session.moving.as_ref() {
                                                Some(moving) => col()
                                                    .w_full()
                                                    .gap(px(8.))
                                                    .child(steps_card(look, to, moving))
                                                    .child(read_only)
                                                    .into_any_element(),
                                                None => read_only,
                                            })
                                        }
                                        (_, Status::Offline) => {
                                            let found = state
                                                .workspace
                                                .machines
                                                .iter()
                                                .find(|x| x.id == session.machine);
                                            let machine = found.map_or("the machine", |x| x.name.as_str());
                                            let lost = found
                                                .and_then(|x| x.lost_at.as_deref())
                                                .map(|at| format!(" at {at}"))
                                                .unwrap_or_default();
                                            Some(notice(
                                                look,
                                                IconName::WifiOff,
                                                0x8e8e93,
                                                format!(
                                                    "Lost the connection to {machine}{lost}. The agent keeps running there; this view catches up when it is back."
                                                ),
                                                None,
                                            ))
                                        }
                                        _ => None,
                                    };
                                    // After a host restart (PER-15, PER-29, frame 46): the line that
                                    // marks it under the old output and, when the agent couldn't be
                                    // resumed, the card that says why, in place of the exit notice.
                                    // AW-9, D267: and the line that says what the host left out
                                    // of the agent's writes when its machine reconnected.
                                    let (reboot, failed, away) = if *tab == 0 {
                                        (
                                            crate::resume_failed::reboot_line(state, session, look),
                                            crate::resume_failed::note(ui, state, session, look),
                                            crate::away_left_out::line(session, look),
                                        )
                                    } else {
                                        (None, None, None)
                                    };
                                    let notice = if reboot.is_some() || failed.is_some() || away.is_some() {
                                        let usual = if failed.is_some() { None } else { notice };
                                        Some(
                                            col()
                                                .w_full()
                                                .gap(px(8.))
                                                .children(reboot)
                                                .children(away)
                                                .children(failed)
                                                .children(usual)
                                                .into_any_element(),
                                        )
                                    } else {
                                        notice
                                    };
                                    terminals.set_offline_banner(
                                        &session.id,
                                        *tab,
                                        session.status != Status::Offline,
                                        cx,
                                    );
                                    let body = terminals.body(&session.id, *tab, group, notice, cx);
                                    col().flex_1().min_h_0().bg(rgb(look.term)).child(body).into_any_element()
                                }
                                Some(Tab::Conversation) => crate::conversation_tab::body(ui, state, session, look, &documents.conversation),
                                None => div().into_any_element(),
                            })
                            .when(state.session.terminal_menu == Some(group), |g| {
                                g.child(crate::action_menu::scrim(
                                    ui,
                                    "close-terminal-menu-scrim",
                                    AppAction::Session(SessionAction::TerminalMenu(None)),
                                ))
                                .child(menu(ui, group, look).test_support())
                            })
                    }),
            ),
        )
        .children(queue(ui, state, session, look))
}

fn queue(ui: &Rc<Ui>, state: &AppState, session: &Session, look: Look) -> Option<AnyElement> {
    let view = state.session.view()?;
    if view.queue.is_empty() {
        return None;
    }
    let failed = state.session.failed_queues.contains(&session.id);
    let certain = !failed
        && matches!(session.agent, Agent::Claude | Agent::Copilot | Agent::OpenCode)
        && session.turn != Turn::Uncertain;
    let sending = state.session.sending;
    let waits = certain;
    let count = view.queue.len();
    let noun = "message";
    // SE-23 and D128.3: the bar says "waiting to send"; held text that goes
    // on its own at the turn's end needs no further note.
    let status = if failed {
        Some("send failed")
    } else if sending {
        Some("sending")
    } else if waits {
        None
    } else {
        Some("send manually")
    };
    let caption = format!(
        "{count} {noun}{} waiting to send{}",
        if count == 1 { "" } else { "s" },
        status
            .map(|status| format!(" · {status}"))
            .unwrap_or_default()
    );
    Some(
        col()
            .id("session-queue")
            .flex_shrink_0()
            .m(px(14.))
            .mt(px(0.))
            .mb(px(8.))
            .px(px(10.))
            .py(px(6.))
            .gap(px(8.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .text_color(rgb(look.text))
            .text_size(px(12.))
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        chrome::icon(IconName::MessageSquareText, look.secondary, 12.)
                            .flex_shrink_0(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(caption.clone())
                            .test_text(caption),
                    )
                    .when(!waits && !sending, |bar| {
                        bar.child(
                            widgets::send(
                                ui,
                                "send-next-queued",
                                "Send now",
                                AppAction::Session(SessionAction::SendNow),
                            )
                            .disabled(state.session.sending),
                        )
                    })
                    .child(
                        widgets::ghost_in(look,
                            ui,
                            "show-queued",
                            if state.session.queue_expanded {
                                "Hide"
                            } else {
                                "Show"
                            },
                            AppAction::Session(SessionAction::ShowQueue(
                                !state.session.queue_expanded,
                            )),
                        )
                        .h(px(18.))
                        .px(px(0.))
                        .text_color(rgb(look.accent)),
                    ),
            )
            .when(state.session.queue_expanded, |list| {
                list.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(rgb(look.secondary))
                        .child("Humata holds these messages before sending them to the terminal."),
                )
                .child(
                    col()
                        .id("queued-list")
                        .max_h(px(200.))
                        .overflow_y_scroll()
                        .children(view.queue.iter().enumerate().map(|(index, message)| {
                            let message = message.text();
                            row()
                                .id(SharedString::from(format!("queued-message-{index}")))
                                .items_start()
                                .gap(px(8.))
                                .py(px(5.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .child(message.clone())
                                        .test_text(message.clone()),
                                )
                                .child(
                                    widgets::send(
                                        ui,
                                        SharedString::from(format!("send-queued-{index}")),
                                        "Send now",
                                        AppAction::Session(SessionAction::SendQueued(index)),
                                    )
                                    .disabled(state.session.sending),
                                )
                                .child(
                                    widgets::ghost_in(look,
                                        ui,
                                        SharedString::from(format!("remove-queued-{index}")),
                                        "Remove",
                                        AppAction::Session(SessionAction::RemoveQueued(index)),
                                    )
                                    .disabled(state.session.sending),
                                )
                                .test_support()
                        })),
                )
            })
            .test_support()
            .into_any_element(),
    )
}

/// Whether the session offers Resume: it exited, and was not ended for good
/// (D784: only Restore or Undo brings an archived session back).
pub(crate) fn offers_resume(session: &Session) -> bool {
    session.status.agent_ended() && !session.archived
}

#[cfg(test)]
mod tests {
    // Named, not `super::*`: the module's gpui glob brings in gpui's own
    // `test` attribute, which would shadow the built-in one.
    use ::core::prelude::v1::test;

    use super::{copy_progress, offers_resume, step_rows, StepBar, StepRow};
    use humata2_clients::fixture;
    use humata2_clients::model::Status;

    #[test]
    fn an_exited_session_offers_resume_unless_it_was_archived() {
        let mut session = fixture::design().sessions.remove(0);
        session.status = Status::Exited { code: 1 };
        session.archived = false;
        assert!(offers_resume(&session));
        session.archived = true;
        assert!(!offers_resume(&session));
        session.archived = false;
        session.status = Status::Idle;
        assert!(!offers_resume(&session));
    }
    use humata2_clients::machines::OperationId;
    use humata2_clients::model::{MachineId, MoveProgress, MoveStep, MoveStepState};

    fn moving(states: &[MoveStepState]) -> MoveProgress {
        MoveProgress {
            to: MachineId::new("gpu-box"),
            operation: OperationId::new("op"),
            step: String::new(),
            done_units: 0,
            total_units: 0,
            steps: states
                .iter()
                .enumerate()
                .map(|(at, state)| MoveStep {
                    title: format!("step {at}"),
                    state: *state,
                    done_units: 0,
                    total_units: 0,
                })
                .collect(),
        }
    }

    #[test]
    fn the_card_has_one_row_per_step_in_order() {
        use MoveStepState::{Done, Failed, Pending, Running};
        let rows = step_rows(&moving(&[Done, Done, Running, Pending, Pending]));
        assert_eq!(rows.len(), 5);
        let titles: Vec<&str> = rows.iter().map(|row| row.title.as_str()).collect();
        assert_eq!(titles, ["step 0", "step 1", "step 2", "step 3", "step 4"]);
        let marks: Vec<&str> = rows.iter().map(|row| row.mark).collect();
        assert_eq!(marks[0], marks[1], "done steps read alike");
        assert_eq!(marks[3], marks[4], "waiting steps read alike");
        assert_ne!(marks[0], marks[2], "done reads unlike running");
        assert_ne!(marks[2], marks[3], "running reads unlike waiting");
        let failed = step_rows(&moving(&[Done, Failed]));
        assert_ne!(failed[1].mark, failed[0].mark, "failed reads unlike done");
    }

    #[test]
    fn the_bar_fills_the_share_copied() {
        let half = copy_progress(50, 100).map(|p| p.fraction);
        assert_eq!(half, Some(0.5));
        let over = copy_progress(150, 100).map(|p| p.fraction);
        assert_eq!(over, Some(1.0), "never past the card's width");
        assert_eq!(copy_progress(0, 100).map(|p| p.fraction), Some(0.0));
    }

    /// The fill of a row's measured bar; none for a sweep or no bar.
    fn measured_fraction(row: &StepRow) -> Option<f32> {
        match &row.bar {
            Some(StepBar::Measured(progress)) => Some(progress.fraction),
            _ => None,
        }
    }

    #[test]
    fn an_unmeasured_running_step_sweeps() {
        assert_eq!(copy_progress(10, 0), None);
        use MoveStepState::{Done, Pending, Running};
        let mut unknown = moving(&[Done, Running, Pending]);
        unknown.done_units = 10;
        let bars: Vec<Option<StepBar>> = step_rows(&unknown).into_iter().map(|row| row.bar).collect();
        assert_eq!(bars, [None, Some(StepBar::Sweep), None]);
    }

    #[test]
    fn only_the_running_step_carries_the_bar() {
        use MoveStepState::{Done, Pending, Running};
        let mut copying = moving(&[Done, Running, Pending]);
        copying.done_units = 25;
        copying.total_units = 100;
        let rows = step_rows(&copying);
        let bars: Vec<bool> = rows.iter().map(|row| row.bar.is_some()).collect();
        assert_eq!(bars, [false, true, false]);
        assert_eq!(measured_fraction(&rows[1]), Some(0.25));
    }

    #[test]
    fn a_step_that_is_not_running_never_carries_a_bar() {
        use MoveStepState::{Done, Failed, Pending, Skipped, Undone};
        let mut settled = moving(&[Done, Failed, Pending, Skipped, Undone]);
        assert!(step_rows(&settled).iter().all(|row| row.bar.is_none()));
        settled.done_units = 25;
        settled.total_units = 100;
        assert!(step_rows(&settled).iter().all(|row| row.bar.is_none()));
    }

    #[test]
    fn a_move_with_no_steps_read_yet_has_no_rows() {
        assert!(step_rows(&moving(&[])).is_empty());
    }
}
