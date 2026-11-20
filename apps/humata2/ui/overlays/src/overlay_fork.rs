//! The Fork session dialog (frame 62; FK-1 to FK-5, FC-15), and the pieces
//! the Move and Archive dialogs share with it.

use super::{Overlays, sheet, sync};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};
use humata2_ui_core::chrome;
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::{Agent, ForkKind, Session};
use humata2_session::{ForkDraft, ForkPicker, SessionAction};
use std::rc::Rc;

/// A dialog card as the frames draw it: padded, 12 pt corners, 12.5 pt text.
pub(super) fn card(id: &'static str, label: &'static str, width: f32, look: Look) -> Stateful<Div> {
    sheet(id, label, look)
        .w(px(width))
        .max_w_full()
        .max_h(relative(0.97))
        .p(px(20.))
        .gap(px(12.))
        .rounded(px(12.))
        .text_size(px(12.5))
}

pub(super) fn heading(text: String) -> Div {
    div()
        .text_size(px(15.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

/// A radio whose words wrap inside its card.
pub(super) fn radio(look: Look, on: bool, name: String, note: &str) -> Div {
    row()
        .items_start()
        .min_w_0()
        .gap(px(8.))
        .child(
            div()
                .mt(px(2.))
                .size(px(14.))
                .flex_shrink_0()
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .when(on, |b| {
                    b.bg(rgb(look.accent))
                        .child(div().size(px(5.)).rounded_full().bg(rgb(0xffffff)))
                })
                .when(!on, |b| {
                    b.border_1()
                        .border_color(rgb(look.tertiary))
                        .bg(rgb(look.card))
                }),
        )
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(div().child(name))
                .when(!note.is_empty(), |words| {
                    words.child(div().text_color(rgb(look.secondary)).child(note.to_owned()))
                }),
        )
}

/// A choice as a bordered card, accent-edged when picked, sending `action`.
pub(super) fn choice(
    ui: &Rc<Ui>,
    id: SharedString,
    look: Look,
    on: bool,
    dim: bool,
    action: AppAction,
    body: Div,
) -> impl IntoElement {
    let ui = ui.clone();
    row()
        .id(id.clone())
        .role(Role::RadioButton)
        .aria_label(id)
        .aria_selected(on)
        .test_support()
        .px(px(12.))
        .py(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(rgb(if on { look.accent } else { look.line }))
        .bg(rgb(if on { look.fill } else { look.card }))
        .when(dim, |card| card.opacity(0.5))
        .cursor_pointer()
        .overflow_hidden()
        .child(body.flex_1().min_w_0())
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

/// The grey pop-up button of a form row.
pub(super) fn popup(ui: &Rc<Ui>, look: Look, id: &'static str, text: String, action: AppAction) -> impl IntoElement {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::Button)
        .aria_label(text.clone())
        .test_support()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(9.))
        .pr(px(5.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .border_1()
        .border_color(rgb(look.line))
        .flex_shrink_0()
        .cursor_pointer()
        .child(text)
        .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

pub(super) fn form_row(look: Look, key: &'static str, value: impl IntoElement) -> Div {
    row()
        .gap(px(12.))
        .child(
            div()
                .w(px(100.))
                .flex()
                .justify_end()
                .flex_shrink_0()
                .text_color(rgb(look.secondary))
                .child(key)
                .test_text(key),
        )
        .child(div().flex_1().min_w_0().child(value))
}

/// The quiet "More options" button with its chevron (› closed, ⌄ open).
fn more_options(ui: &Rc<Ui>, look: Look, open: bool) -> impl IntoElement {
    let ui = ui.clone();
    widgets::push(
        "fork-more-options",
        "More options",
    )
    .text_color(rgb(look.secondary))
    .child(chrome::icon(
        if open { IconName::ChevronDown } else { IconName::ChevronRight },
        look.secondary,
        12.,
    ))
    .on_click(move |_, _, cx| {
        cx.stop_propagation();
        ui.send(AppAction::Session(SessionAction::ForkMoreOptions), cx);
    })
}

impl Overlays {
    /// The Fork session dialog.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fork(
        &self,
        fork: ForkDraft,
        forking: bool,
        fork_error: Option<String>,
        parent: Option<Session>,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.fork_branch, &fork.branch, window, cx);
        let machines = ui.read(|state| state.workspace.machines.clone());
        // AGENTSET (D725): the enabled agents, then the plain shell.
        let offered: Vec<Agent> = ui
            .read(|state| state.settings.offered_agents())
            .into_iter()
            .chain([Agent::Shell])
            .collect();
        let parent_title = parent.as_ref().map_or("this session", |s| s.title.as_str());
        let agent = fork
            .agent
            .or_else(|| parent.as_ref().map(|p| p.agent))
            .unwrap_or(Agent::Claude);
        let machine_id = fork
            .machine
            .clone()
            .or_else(|| parent.as_ref().map(|p| p.machine.clone()));
        let machine_name = machine_id
            .as_ref()
            .and_then(|id| machines.iter().find(|m| &m.id == id))
            .map_or_else(
                || machine_id.as_ref().map_or_else(String::new, |m| m.to_string()),
                |m| m.name.clone(),
            );
        let same_agent = parent.as_ref().is_none_or(|p| p.agent == agent);
        let agent_name = match agent {
            Agent::Claude => "Claude Code",
            _ => agent.name(),
        };
        let send = |action: SessionAction| AppAction::Session(action);

        let mut body = card("fork-session", "Fork session", 640., look)
            .overflow_y_scroll()
            .child(heading(format!("Fork “{parent_title}”")));
        let kinds = [
            (
                0usize,
                ForkKind::Worktree,
                "New copies of the files as they are now, uncommitted changes included, and an agent that knows the conversation.",
            ),
            (
                1,
                ForkKind::Conversation,
                "An agent that knows the conversation and works in this session's files, with no copies, so its edits land here.",
            ),
            (
                2,
                ForkKind::Shared,
                "New copies of the files and a fresh agent with no conversation.",
            ),
        ];
        let sentence = kinds
            .iter()
            .find(|(_, kind, _)| *kind == fork.kind)
            .map_or("", |(_, _, note)| *note);
        if !fork.more {
            body = body.child(
                div()
                    .id("fork-sentence")
                    .text_color(rgb(look.secondary))
                    .child(sentence),
            );
        } else {
            body = body.child(col().gap(px(6.)).children(kinds.map(|(index, kind, note)| {
            let on = fork.kind == kind;
            let id = if on {
                format!("fork-kind-{index}-on")
            } else {
                format!("fork-kind-{index}")
            };
            choice(
                ui,
                id.into(),
                look,
                on,
                false,
                send(SessionAction::ForkKind(kind)),
                radio(look, on, kind.name().to_owned(), note),
            )
        })));
        }
        if fork.more {
        if fork.kind != ForkKind::Shared {
            let from = match fork.before_turn {
                None => "Now".to_owned(),
                Some(turn) => format!("Before turn {turn}"),
            };
            body = body.child(form_row(
                look,
                "Fork from",
                row()
                    .gap(px(8.))
                    .child(popup(
                        ui,
                        look,
                        "fork-from",
                        from,
                        send(SessionAction::ForkPicker(Some(ForkPicker::From))),
                    ))
                    .child(
                        div()
                            .text_color(rgb(look.secondary))
                            .child("or before an earlier turn"),
                    ),
            ));
            if fork.picker == Some(ForkPicker::From) {
                let mut options = row().flex_wrap().gap(px(6.)).child(widgets::push_quiet(
                    ui,
                    "fork-from-now",
                    "Now",
                    send(SessionAction::ForkBeforeTurn(None)),
                    look,
                ));
                for turn in fork.turns.iter().flatten() {
                    let text: String = turn.text.chars().take(40).collect();
                    options = options.child(widgets::push_quiet(
                        ui,
                        SharedString::from(format!("fork-from-{}", turn.index)),
                        format!("Before {}: {text}", turn.index),
                        send(SessionAction::ForkBeforeTurn(Some(turn.index))),
                        look,
                    ));
                }
                body = body.child(options);
            }
        }

        body = body.child(form_row(
            look,
            "Agent",
            row().child(popup(
                ui,
                look,
                "fork-agent",
                match agent {
                    Agent::Claude => "Claude Code",
                    _ => agent.name(),
                }
                .to_owned(),
                send(SessionAction::ForkPicker(Some(ForkPicker::Agent))),
            )),
        ));
        if fork.picker == Some(ForkPicker::Agent) {
            body = body.child(row().flex_wrap().gap(px(6.)).children(
                offered.iter().copied().enumerate().map(|(index, a)| {
                    widgets::push_quiet(
                        ui,
                        ("fork-agent-pick", index),
                        a.name(),
                        send(SessionAction::ForkAgent(a)),
                        look,
                    )
                }),
            ));
        }
        body = body.child(form_row(
            look,
            "Machine",
            row().child(popup(
                ui,
                look,
                "fork-machine",
                machine_name.clone(),
                send(SessionAction::ForkPicker(Some(ForkPicker::Machine))),
            )),
        ));
        if fork.picker == Some(ForkPicker::Machine) {
            body = body.child(
                row().flex_wrap().gap(px(6.)).children(
                    machines
                        .iter()
                        .filter(|m| m.online)
                        .map(|m: &humata2_clients::model::Machine| {
                            widgets::push_quiet(
                                ui,
                                SharedString::from(format!("fork-machine-{}", m.id)),
                                m.name.clone(),
                                send(SessionAction::ForkMachine(m.id.clone())),
                                look,
                            )
                        }),
                ),
            );
        }
        if fork.kind != ForkKind::Conversation {
            body = body.child(form_row(
                look,
                "Branch",
                col()
                    .gap(px(3.))
                    .child(
                        div()
                            .id("fork-branch")
                            .role(Role::Group)
                            .aria_label("Branch")
                            .test_support()
                            .child(
                                Input::new(&self.fork_branch)
                                    .disabled(forking)
                                    .aria_label("Fork branch"),
                            ),
                    )
                    .child(
                        div()
                            .text_color(rgb(look.secondary))
                            .child("Humata creates this branch in each new clone."),
                    ),
            ));
        }

        }

        // What the fork will be, live under the fields (D873).
        let mut facts = format!("{agent_name} · {machine_name}");
        if fork.kind != ForkKind::Conversation {
            facts.push_str(&format!(" · branch {}", fork.branch));
        }
        if let Some(turn) = fork.before_turn.filter(|_| fork.kind != ForkKind::Shared) {
            facts.push_str(&format!(" · before turn {turn}"));
        }
        body = body.child(
            div()
                .id("fork-facts")
                .role(Role::Group)
                .aria_label("Fork facts")
                .test_support()
                .text_color(rgb(look.secondary))
                .child(facts),
        );
        if fork.kind != ForkKind::Shared && !same_agent {
            body = body.child(
                row()
                    .gap(px(6.))
                    .child(chrome::icon(IconName::Info, 0xff9f0a, 12.))
                    .child(div().text_color(rgb(look.secondary)).child(format!(
                        "A different agent can't read this conversation, so {} starts from a summary of it.",
                        agent.name()
                    ))),
            );
        }
        body = body
            .children(fork_error.map(|error| {
                div()
                    .id("fork-error")
                    .role(Role::Alert)
                    .text_color(rgb(0xff453a))
                    .child(error)
            }))
            .child(
                row()
                    .justify_between()
                    .items_center()
                    .pt(px(4.))
                    .child(more_options(ui, look, fork.more))
                    .child(
                        row()
                            .gap(px(8.))
                            .child(
                                widgets::push_quiet(ui, "fork-cancel", "Cancel", send(SessionAction::CancelFork), look)
                                    .disabled(forking),
                            )
                            .child(
                                widgets::push_primary(
                                    ui,
                                    "fork-create",
                                    if forking { "Forking…" } else { "Fork" },
                                    send(SessionAction::ConfirmFork),
                                    look,
                                )
                                .disabled(forking || fork.title.trim().is_empty()),
                            ),
                    ),
            );
        body = body.children(
            fork.progress
                .as_ref()
                .filter(|_| forking)
                .map(|p| widgets::progress_line(look, p, Some(ui.now()))),
        );
        // Return forks and Escape cancels while the dialog itself has focus.
        let (keys, sheet) = (ui.clone(), self.sheet.clone());
        body.track_focus(&self.sheet)
            .on_key_down(move |event, window, cx| {
                if event.keystroke.key == "enter" && sheet.is_focused(window) {
                    cx.stop_propagation();
                    keys.send(AppAction::Session(SessionAction::ConfirmFork), cx);
                }
            })
            .test_support()
            .focus_trap("fork-trap", &self.sheet)
            .into_any_element()
    }
}
