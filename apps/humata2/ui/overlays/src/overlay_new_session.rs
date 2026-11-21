//! The New session sheet (SE-1).

use super::{Overlays, sync};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::widgets::{self, col, row};
use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, NewSession, NewSessionAction, NewSessionPicker};
use humata2_clients::model::Agent;
use std::rc::Rc;

impl Overlays {
    /// The New session sheet.
    pub(super) fn new_session(
        &mut self,
        sheet_state: &NewSession,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.title, &sheet_state.title, window, cx);
        if self.first_message.read(cx).value().as_ref() != sheet_state.first_message {
            self.first_message.update(cx, |input, cx| {
                input.set_value(sheet_state.first_message.clone(), window, cx)
            });
        }
        let placeholder = sheet_state.item.as_ref().map_or_else(
            || "Optional. Sent to the agent on start.".into(),
            |id| format!("Optional. Sent on start, e.g. “Work on {id}”."),
        );
        if self.first_message_placeholder != placeholder {
            self.first_message_placeholder = placeholder.clone();
            self.first_message.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        let workspace = self.ui.read(|s| s.workspace.clone());
        // AGENTSET (D725): the enabled agents, then Terminal.
        let offered: Vec<Agent> = self
            .ui
            .read(|s| s.settings.offered_agents())
            .into_iter()
            .chain([Agent::Shell])
            .collect();
        let channel = sheet_state
            .channel
            .as_ref()
            .and_then(|id| workspace.channel(id));
        let machine = sheet_state
            .machine
            .as_ref()
            .and_then(|id| workspace.machine(id));
        let label = |text: &str| {
            div()
                .w(px(110.))
                .flex_shrink_0()
                .flex()
                .justify_end()
                .pt(px(4.))
                .text_color(rgb(look.secondary))
                .child(text.to_owned())
        };
        let form_row = |text: &str, value: AnyElement| {
            row()
                .items_start()
                .gap(px(12.))
                .child(label(text))
                .child(div().flex_1().min_w_0().child(value))
        };
        let button = |id: ElementId, text: SharedString, action: AppAction| {
            widgets::send(ui, id, text, action)
                .small()
                .h(px(24.))
                .px(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .bg(rgb(look.fill))
                .text_color(rgb(look.text))
        };
        let popup = |id: ElementId, text: SharedString, action: AppAction| {
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
                .text_size(px(12.))
                .flex_shrink_0()
                .cursor_pointer()
                .child(text)
                .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
        };
        let context = format!(
            "in # {}{}",
            channel.map_or("Choose a channel", |c| c.name.as_str()),
            sheet_state
                .item
                .as_ref()
                .map_or_else(String::new, |id| format!(" · for {id}"))
        );
        let context_ui = ui.clone();
        let mut content = col()
            .id("new-session")
            .role(Role::Dialog)
            .aria_label("New session")
            .w(px(596.))
            .max_w_full()
            .max_h(relative(0.85))
            .overflow_y_scroll()
            .gap(px(12.))
            .p(px(22.))
            .rounded(px(12.))
            .bg(rgb(look.card))
            .border(px(0.5))
            .border_color(look.hairline())
            .shadow(humata2_ui_core::theme::shadow_sheet(look))
            .text_size(px(12.5))
            .text_color(rgb(look.text))
            .test_support()
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("New session"),
                    )
                    .child(
                        div()
                            .id("new-session-channel-picker")
                            .role(Role::Button)
                            .aria_label("Choose channel")
                            .test_support()
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .cursor_pointer()
                            .child(context)
                            .on_click(move |_, _, cx| {
                                context_ui.send(
                                    AppAction::NewSession(NewSessionAction::Picker(Some(
                                        NewSessionPicker::Channel,
                                    ))),
                                    cx,
                                )
                            }),
                    ),
            );
        if sheet_state.picker == Some(NewSessionPicker::Channel) {
            content = content.child(row().flex_wrap().gap(px(6.)).children(
                workspace.channels.iter().enumerate().map(|(index, c)| {
                    button(
                        ("new-session-channel", index).into(),
                        format!("# {}", c.name).into(),
                        AppAction::NewSession(NewSessionAction::Channel(c.id.clone())),
                    )
                }),
            ));
        }
        content = content.child(form_row(
            "Title",
            div()
                .h(px(26.))
                .child(
                    Input::new(&self.title)
                        .small()
                        .min_h(px(26.))
                        .text_size(px(12.5))
                        .focus_bordered(false)
                        .aria_label("Title"),
                )
                .into_any_element(),
        ));
        let agents = row().flex_shrink_0().p(px(2.)).rounded(px(7.)).bg(rgb(look.fill)).children(
            offered.into_iter().enumerate().map(|(index, agent)| {
                let ui = ui.clone();
                let name = match agent {
                    Agent::Claude => "Claude Code",
                    Agent::Shell => "Terminal",
                    _ => agent.name(),
                };
                div()
                    .id(("new-session-agent", index))
                    .role(Role::Tab)
                    .aria_label(name)
                    .aria_selected(sheet_state.agent == agent)
                    .test_support()
                    .px(px(8.))
                    .py(px(2.))
                    .text_size(px(11.5))
                    .rounded(px(5.))
                    .when(sheet_state.agent == agent, |x| {
                        x.bg(rgb(look.card))
                            .font_weight(FontWeight::SEMIBOLD)
                            .border_1()
                            .border_color(look.control_border)
                            .shadow_xs()
                    })
                    .cursor_pointer()
                    .child(name)
                    .on_click(move |_, _, cx| {
                        ui.send(AppAction::NewSession(NewSessionAction::Agent(agent)), cx)
                    })
            }),
        );
        let runs = col()
            .gap(px(6.))
            .child(row().child(agents).child(div().flex_1()))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child("An agent's own TUI, with its own defaults, or a plain shell"),
            );
        content = content.child(form_row("Runs", runs.into_any_element()));
        // Center the free-space note vertically with the machine picker button.
        let machine_picker = row()
            .gap(px(8.))
            .child(
                popup(
                    "new-session-machine-picker".into(),
                    machine
                        .map_or("Choose a machine", |m| m.name.as_str())
                        .to_owned()
                        .into(),
                    AppAction::NewSession(NewSessionAction::Picker(Some(
                        NewSessionPicker::Machine,
                    ))),
                ),
            )
            .children(
                machine
                    .and_then(|m| workspace.machine_health.get(&m.id))
                    .map(|h| {
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(format!("{} GB free", h.disk_free_gb))
                    }),
            );
        content = content.child(form_row("Machine", machine_picker.into_any_element()));
        if sheet_state.picker == Some(NewSessionPicker::Machine) {
            content = content.child(
                row().flex_wrap().gap(px(6.)).children(
                    workspace
                        .machines
                        .iter()
                        .filter(|m| {
                            m.online
                                && channel.is_some_and(|c| {
                                    c.machines.contains(&m.id)
                                        || workspace
                                            .sessions
                                            .iter()
                                            .any(|s| s.channel == c.id && s.machine == m.id)
                                })
                        })
                        .enumerate()
                        .map(|(index, m)| {
                            button(
                                ("new-session-machine", index).into(),
                                m.name.clone().into(),
                                AppAction::NewSession(NewSessionAction::Machine(m.id.clone())),
                            )
                        }),
                ),
            );
        }
        let repos = col()
            .gap(px(6.))
            .children(channel.into_iter().flat_map(|c| &c.repos).map(|repo| {
                row()
                    .gap(px(8.))
                    .child(chrome::icon(IconName::FolderGit2, look.secondary, 12.))
                    .child(
                        div()
                            .w(px(120.))
                            .child(repo.rsplit('/').next().unwrap_or(repo).to_owned()),
                    )
                    .child(popup(
                        SharedString::from(format!("new-session-repo-{repo}")).into(),
                        if sheet_state.excluded_repos.contains(repo) {
                            "Leave this repo out".into()
                        } else {
                            "Own clone, new branch from main".into()
                        },
                        AppAction::NewSession(NewSessionAction::ToggleRepo(repo.clone())),
                    ))
            }));
        content = content
            .child(form_row("Repos", repos.into_any_element()))
            .child(form_row(
                "First message",
                Textarea::new(&self.first_message)
                    .h(px(70.))
                    .text_size(px(12.5))
                    .aria_label("First message")
                    .into_any_element(),
            ));
        // Do next (frame b4-3): "Skip to" the next ranked item.
        let skip = self.ui.read(|s| {
            let ranked = s.todo.ranked();
            let at = ranked.next.iter().position(|r| Some(&r.key) == s.todo.do_next.as_ref())?;
            let row = ranked.next.get(at + 1)?;
            let title = s.todo.items.iter().find(|e| e.todo.key == row.key)?.todo.title.clone();
            Some((row.key.clone(), title))
        });
        if let Some(error) = &sheet_state.error {
            content = content.child(
                div()
                    .id("new-session-error")
                    .role(Role::Alert)
                    .test_support()
                    .text_color(rgb(0xff453a))
                    .child(error.clone()),
            );
        }
        if let Some(note) = &sheet_state.note {
            content = content.child(
                div()
                    .id("new-session-note")
                    .test_support()
                    .text_size(px(12.))
                    .text_color(rgb(look.secondary))
                    .child(note.clone()),
            );
        }
        content
            .child(
                row()
                    .items_start()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(format!("Branch: {}", sheet_state.branch())),
                    )
                    .children(skip.map(|(key, title)| {
                        widgets::push_quiet(
                            ui,
                            "new-session-skip",
                            &format!("Skip to {key} {title}"),
                            AppAction::Todo(humata2_app::TodoAction::SkipDoNext),
                            look,
                        )
                    }))
                    .child(
                        widgets::push_quiet(
                            ui,
                            "new-session-cancel",
                            "Cancel",
                            AppAction::Escape,
                            look,
                        ),
                    )
                    .child(
                        widgets::push_primary(
                            ui,
                            "new-session-create",
                            if sheet_state.creating {
                                "Starting…"
                            } else {
                                "Start session"
                            },
                            AppAction::NewSession(NewSessionAction::Create),
                            look,
                        )
                        .disabled(sheet_state.creating),
                    ),
            )
            .children(
                sheet_state
                    .progress
                    .as_ref()
                    .filter(|_| sheet_state.creating)
                    .map(|p| widgets::progress_line(look, p, Some(ui.now()))),
            )
            .focus_trap("new-session-trap", &self.title.focus_handle(cx))
            .into_any_element()
    }
}
