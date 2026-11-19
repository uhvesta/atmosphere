//! The Fork session dialog (frame 62; FK-1 to FK-5, FC-15), and the pieces
//! the Move and Archive dialogs share with it.

use super::{Overlays, sheet, sync};
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use crate::chrome;
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::model::{Agent, ForkKind, Session, Speaker};
use humata2_session::{ForkDraft, ForkPicker, SessionAction};
use std::rc::Rc;

const GREEN: u32 = 0x30b350;

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

/// The unwrapped line of a preview.
fn line(color: u32, text: String) -> Div {
    div().text_color(rgb(color)).whitespace_nowrap().child(text)
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
        let (machines, repos) = ui.read(|state| {
            let repos = parent
                .as_ref()
                .map(|p| {
                    state
                        .workspace
                        .session_files
                        .get(&p.id)
                        .into_iter()
                        .flatten()
                        .map(|f| f.repo.clone())
                        .collect::<std::collections::BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (state.workspace.machines.clone(), repos)
        });
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
        let send = |action: SessionAction| AppAction::Session(action);

        let mut body = card("fork-session", "Fork session", 640., look)
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

        // The preview, as a tree under the parent.
        let repo_list = if repos.is_empty() {
            parent.as_ref().map_or_else(String::new, |p| p.repo.clone())
        } else {
            repos.join(", ")
        };
        let turns = parent
            .as_ref()
            .map_or(0, |p| p.transcript.iter().filter(|l| l.from == Speaker::You).count());
        let fork_title = fork.title.trim().to_owned();
        let how = fork.kind.name().to_lowercase();
        let conversation = match (fork.kind, same_agent) {
            (ForkKind::Shared, _) => "fresh agent, no conversation".to_owned(),
            (_, true) => "conversation copied exactly (same agent)".to_owned(),
            (_, false) => format!("conversation sent to {} as a summary", agent.name()),
        };
        let place = if fork.kind == ForkKind::Conversation {
            format!("     works in the files of {parent_title}, no copies")
        } else {
            format!("     new clones of {repo_list} on {}", fork.branch)
        };
        let mut preview = col()
            .id("fork-preview")
            .role(Role::Group)
            .aria_label("Preview")
            .test_support()
            .gap(px(2.))
            .px(px(12.))
            .py(px(9.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .font_family("Menlo")
            .text_size(px(11.))
            .line_height(px(16.))
            .child(line(
                look.text,
                format!(
                    "{parent_title}   [{repo_list}]  {} · {} · {turns} turns",
                    parent.as_ref().map_or("", |p| p.agent.name()),
                    parent.as_ref().map_or_else(String::new, |p| p.machine.to_string()),
                ),
            ))
            .child(line(look.text, format!("└─⑂ {fork_title}   {how}")))
            .child(line(look.secondary, place));
        preview = preview.child(line(look.secondary, format!("     {conversation}")));
        body = body.child(
            col()
                .gap(px(5.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(look.secondary))
                        .child("Preview"),
                )
                .child(preview),
        );
        if fork.kind != ForkKind::Shared {
            let (mark, words) = if same_agent {
                (GREEN, "Same agent: the conversation is copied exactly.".to_owned())
            } else {
                (
                    0xff9f0a,
                    format!(
                        "A different agent can't read this conversation, so {} starts from a summary of it.",
                        agent.name()
                    ),
                )
            };
            body = body.child(
                row()
                    .gap(px(6.))
                    .child(chrome::icon(
                        if same_agent { IconName::Check } else { IconName::Info },
                        mark,
                        12.,
                    ))
                    .child(div().text_color(rgb(look.secondary)).child(words)),
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
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(4.))
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
            );
        body = body.children(
            fork.progress
                .as_ref()
                .filter(|_| forking)
                .map(|p| widgets::progress_line(look, p, Some(ui.clients().daemon.now_ms()))),
        );
        body.test_support()
            .focus_trap("fork-trap", &self.sheet)
            .into_any_element()
    }
}
