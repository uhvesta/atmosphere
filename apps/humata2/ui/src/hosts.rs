//! Host health, selection and reconnect against the fake clients (frame 28).
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{col, push_quiet, push_quiet_shape, row},
};
use gpui_kit::base::{Disableable as _, TestTextExt as _};
use gpui_kit::component::button::Button;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, NewSessionAction};
use humata2_clients::machines::tool_label;
use humata2_clients::model::{Agent, MachineId, MachineKind, Status};
use humata2_hosts::HostsAction as A;
use std::rc::Rc;
fn dim(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}
fn card(look: Look) -> Div {
    col()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
}
fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
) -> Button {
    push_quiet(ui, id, label, action, look)
}
fn new_session(ui: &Rc<Ui>, machine: &MachineId, shell: bool, cx: &mut App) {
    ui.send(AppAction::StartNewSession, cx);
    ui.send(
        AppAction::NewSession(NewSessionAction::Machine(machine.clone())),
        cx,
    );
    if shell {
        ui.send(
            AppAction::NewSession(NewSessionAction::Agent(Agent::Shell)),
            cx,
        );
    }
}
pub(crate) fn toolbar(ui: &Rc<Ui>, look: Look) -> Div {
    row()
        .h(px(52.))
        .flex_shrink_0()
        .px(px(20.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            div()
                .flex_1()
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Hosts"),
        )
        .child(button(
            ui,
            "hosts-add",
            "Add a machine…",
            AppAction::Hosts(A::StartAdd),
            look,
        ))
}
pub(crate) fn page(
    ui: &Rc<Ui>,
    state: &AppState,
    width: f32,
    look: Look,
) -> Div {
    let narrow = width < 850.;
    let mut table = card(look)
        .flex_1()
        .min_w(px(560.))
        .h_full()
        .overflow_hidden();
    for machine in &state.workspace.machines {
        let health = state.workspace.machine_health.get(&machine.id);
        let id = machine.id.clone();
        let send = ui.clone();
        let count = state
            .workspace
            .sessions
            .iter()
            .filter(|s| s.machine == machine.id && !s.archived)
            .count();
        let stopped = health.is_some_and(|h| h.stopped);
        let status = if machine.online {
            Status::Working
        } else if stopped {
            Status::Idle
        } else {
            Status::Offline
        };
        let mut line = row()
            .id(SharedString::from(format!("host-row {}", machine.id)))
            .role(Role::ListItem)
            .aria_label(machine.name.clone())
            .aria_selected(state.hosts.selected.as_ref() == Some(&machine.id))
            .test_support()
            .h(px(44.))
            .flex_shrink_0()
            .px(px(12.))
            .gap(px(10.))
            .border_b_1()
            .border_color(rgb(look.line))
            .cursor_pointer()
            .when(state.hosts.selected.as_ref() == Some(&machine.id), |x| {
                x.bg(rgb(look.fill))
            })
            .child(chrome::status(status))
            .child(
                col()
                    .w(px(150.))
                    .flex_shrink_0()
                    .child({
                        let (open, target) = (ui.clone(), machine.id.clone());
                        div()
                            .id(SharedString::from(format!("host-name {}", machine.id)))
                            .role(Role::Link)
                            .aria_label(machine.name.clone())
                            .font_weight(FontWeight::MEDIUM)
                            .child(machine.name.clone())
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                open.send(
                                    AppAction::Navigate(humata2_app::Route::Machine(
                                        target.clone(),
                                    )),
                                    cx,
                                );
                            })
                    })
                    .child(dim(
                        look,
                        health.map_or_else(|| machine.detail.clone(), |h| h.system.clone()),
                    )),
            )
            .child(div().w(px(110.)).flex_shrink_0().child(if count == 0 {
                "none".into()
            } else {
                format!(
                    "{count} {}",
                    if count == 1 { "session" } else { "sessions" }
                )
            }));
        if let Some(health) = health {
            line =
                line.child(
                    row()
                        .w(px(150.))
                        .flex_shrink_0()
                        .gap(px(8.))
                        .child(
                            div()
                                .w(px(70.))
                                .h(px(5.))
                                .rounded_full()
                                .bg(rgb(look.fill))
                                .child(
                                    div()
                                        .w(px(70. * f32::from(health.disk_used_percent.min(100))
                                            / 100.))
                                        .h_full()
                                        .rounded_full()
                                        .bg(rgb(if health.disk_used_percent > 85 {
                                            0xff453a
                                        } else {
                                            look.accent
                                        })),
                                ),
                        )
                        .child(dim(
                            look,
                            if stopped {
                                "stopped".into()
                            } else if machine.online {
                                format!("{} GB free", health.disk_free_gb)
                            } else {
                                format!("last seen {}", machine.last_seen)
                            },
                        )),
                );
        }
        line = line.child(div().flex_1());
        if !machine.online {
            line = line.child(
                button(
                    ui,
                    SharedString::from(format!("host-connect {}", machine.id)),
                    if state.hosts.connecting.contains(&machine.id) {
                        "Connecting…"
                    } else if stopped {
                        "Start"
                    } else {
                        "Reconnect"
                    },
                    AppAction::Hosts(A::Connect(machine.id.clone())),
                    look,
                )
                .disabled(state.hosts.connecting.contains(&machine.id)),
            );
        } else if health.is_some_and(|h| h.disk_used_percent > 85) {
            line = line.child(
                button(
                    ui,
                    SharedString::from(format!("host-free {}", machine.id)),
                    "Free space…",
                    AppAction::Hosts(A::Select(machine.id.clone())),
                    look,
                )
                .disabled(true),
            );
        }
        table = table.child(
            line.on_click(move |_, _, cx| send.send(AppAction::Hosts(A::Select(id.clone())), cx)),
        );
    }
    let mut details = col().w(px(300.)).flex_shrink_0().gap(px(10.));
    if let Some(machine) = state
        .hosts
        .selected
        .as_ref()
        .and_then(|id| state.workspace.machine(id))
    {
        let health = state.workspace.machine_health.get(&machine.id);
        let mut info = card(look).p(px(12.)).gap(px(6.)).child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child(machine.name.clone()),
        );
        let link = match &machine.kind {
            MachineKind::ThisMac => "local",
            MachineKind::Ssh { .. } => "ssh",
            MachineKind::Provider { .. } => "provider",
        };
        // What the machine's daemon reported wins over what the workspace carries (HO-4).
        let reported = state.hosts.health.get(&machine.id);
        let (summary, tools, disk) = if let Some(detail) = reported {
            let health = &detail.health;
            let mut tools = vec![tool_label("gh", &health.gh_version)];
            tools.extend(
                health
                    .agents
                    .iter()
                    .map(|(agent, version)| tool_label(agent.command(), version)),
            );
            let free_gb = health.disk_free / 1_000_000_000;
            let used = if health.disk_total == 0 {
                0
            } else {
                100 - (health.disk_free.min(health.disk_total) * 100 / health.disk_total)
            };
            (
                format!("humatad {} · linked over {link}", detail.daemon_version),
                tools,
                (used, free_gb),
            )
        } else if let Some(health) = health {
            (
                format!(
                    "humatad {} · up {} · linked over {link}",
                    health.daemon, health.uptime
                ),
                health.tools.clone(),
                (u64::from(health.disk_used_percent), health.disk_free_gb),
            )
        } else {
            (String::new(), Vec::new(), (0, 0))
        };
        if reported.is_some() || health.is_some() {
            info = info.child(dim(look, summary)).child(
                row()
                    .flex_wrap()
                    .gap(px(6.))
                    .children(tools.into_iter().map(|tool| {
                        dim(look, tool.clone())
                            .test_text(tool)
                            .text_size(px(11.))
                            .px(px(6.))
                            .py(px(1.))
                            .rounded(px(4.))
                            .border_1()
                            .border_color(rgb(look.line))
                    })),
            );
            if disk.0 > 85 {
                info = info.child(div().text_color(rgb(0xff9500)).child(format!(
                    "{} GB free: new clones stop at 5 GB.",
                    disk.1
                )));
            }
        }
        let send = ui.clone();
        let id = machine.id.clone();
        info = info.child(
            row()
                .gap(px(8.))
                .child(
                    button(
                        ui,
                        "host-detail-free",
                        "Free space…",
                        AppAction::Hosts(A::Select(machine.id.clone())),
                        look,
                    )
                    .disabled(true),
                )
                .child(
                    push_quiet_shape("host-open-shell", "Open a shell", look)
                        .disabled(!machine.online)
                        .on_click(move |_, _, cx| new_session(&send, &id, true, cx)),
                ),
        );
        details = details.child(info);
        if let Some(error) = state.hosts.errors.get(&machine.id) {
            details = details.child(div().text_color(rgb(0xff453a)).child(error.clone()));
        }
    }
    let mut bridge = card(look)
        .p(px(12.))
        .gap(px(6.))
        .child(div().font_weight(FontWeight::SEMIBOLD).child("Bridge"));
    for m in state.workspace.machines.iter().filter(|m| {
        !matches!(m.kind, MachineKind::ThisMac)
            && (m.online
                || !state
                    .workspace
                    .machine_health
                    .get(&m.id)
                    .is_some_and(|h| h.stopped))
    }) {
        let latency = state
            .workspace
            .machine_health
            .get(&m.id)
            .and_then(|h| h.latency_ms);
        bridge = bridge.child(dim(
            look,
            if m.online {
                format!(
                    "This Mac ⇄ {} (ssh{})",
                    m.name,
                    latency.map_or_else(String::new, |n| format!(", {n} ms"))
                )
            } else {
                format!(
                    "This Mac ⇢ {} (offline since {}; retrying)",
                    m.name, m.last_seen
                )
            },
        ));
    }
    details = details.child(bridge);
    col().size_full().min_h_0().child(
        row()
            .flex_1()
            .min_h_0()
            .min_h_0()
            .items_stretch()
            .when(narrow, |x| x.flex_col())
            .gap(px(16.))
            .p(px(20.))
            .text_size(px(12.5))
            .text_color(rgb(look.text))
            .child(
                div()
                    .id("hosts-scroll")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .when(narrow, |x| x.min_h(px(210.)))
                    .overflow_x_scroll()
                    .child(table),
            )
            .child(details.when(narrow, |x| x.w_full())),
    )
}
