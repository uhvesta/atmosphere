//! The Humata folder sheet (PL-1, FC-10, ST-5), opened by Change… in
//! Settings › Machines. One field holds the folder, prefilled with the
//! current one; Plan shows the channels under the old root and the sessions
//! that run; Move them or Use for new channels only is then the person's
//! choice. Nothing moves before that. No frame draws it; it follows the
//! Rename dialog's card.

use super::Overlays;
use super::fork::{card, heading};
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{self, row};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_hosts::{HostsAction, RootChange};
use std::rc::Rc;

/// What the sheet shows, read from the app's state: the sheet itself with
/// the names of what its plan lists.
#[derive(Clone)]
pub(super) struct RootView {
    sheet: RootChange,
    machine: String,
    channels: Vec<String>,
    running: Vec<String>,
}

impl RootView {
    /// The open sheet, if there is one.
    pub(super) fn read(state: &AppState) -> Option<Self> {
        let sheet = state.hosts.root_change.clone()?;
        let machine = state
            .workspace
            .machine(&sheet.machine)
            .map_or_else(|| sheet.machine.to_string(), |found| found.name.clone());
        let (channels, running) = sheet.plan.as_ref().map_or_else(
            || (Vec::new(), Vec::new()),
            |plan| {
                let channels = plan
                    .channels
                    .iter()
                    .map(|id| {
                        state
                            .workspace
                            .channels
                            .iter()
                            .find(|channel| &channel.id == id)
                            .map_or_else(|| id.to_string(), |channel| format!("#{}", channel.name))
                    })
                    .collect();
                let running = plan
                    .running
                    .iter()
                    .map(|id| {
                        state
                            .workspace
                            .session(id)
                            .map_or_else(|| id.to_string(), |session| session.title.clone())
                    })
                    .collect();
                (channels, running)
            },
        );
        Some(Self { sheet, machine, channels, running })
    }
}

impl Overlays {
    /// The Humata folder sheet.
    pub(super) fn root_dialog(
        &self,
        view: &RootView,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sheet = &view.sheet;
        super::sync(&self.root_text, &sheet.text, window, cx);
        let send = |action: HostsAction| AppAction::Hosts(action);
        let mut body = card("root-sheet", "Humata folder", 520., look)
            .child(heading(format!("Humata folder on {}", view.machine)))
            .child(
                div()
                    .text_color(rgb(look.secondary))
                    .child("New channels are made under this folder. Type a full path or one that starts with ~/."),
            )
            .child(
                row()
                    .gap(px(8.))
                    .items_center()
                    .child(
                        div()
                            .id("root-folder")
                            .role(Role::Group)
                            .aria_label("Folder")
                            .test_support()
                            .flex_1()
                            .min_w_0()
                            .child(
                                Input::new(&self.root_text)
                                    .disabled(sheet.running)
                                    .aria_label("Humata folder"),
                            ),
                    )
                    .child(
                        widgets::push_quiet(ui, "root-plan", "Plan", send(HostsAction::PlanRoot), look)
                            .disabled(sheet.running || sheet.planning || sheet.text.trim().is_empty()),
                    ),
            );
        if let Some(plan) = &sheet.plan {
            body = body.child(
                div()
                    .id("root-plan-summary")
                    .role(Role::Group)
                    .aria_label("Plan")
                    .test_support()
                    .child(if view.channels.is_empty() {
                        format!("No channels are under {}. Only new channels will use {}.", plan.old_root, plan.new_root)
                    } else {
                        format!(
                            "{} channel{} under {}: {}.",
                            view.channels.len(),
                            if view.channels.len() == 1 { "" } else { "s" },
                            plan.old_root,
                            view.channels.join(", ")
                        )
                    }),
            );
            if !view.running.is_empty() {
                body = body.child(
                    div()
                        .id("root-running")
                        .text_color(rgb(look.secondary))
                        .child(format!(
                            "Move them is off while {} run{}. Stop {} first, or use the folder for new channels only.",
                            view.running.join(", "),
                            if view.running.len() == 1 { "s" } else { "" },
                            if view.running.len() == 1 { "it" } else { "them" },
                        )),
                );
            }
        }
        if sheet.running {
            body = body.child(div().id("root-progress").child("Changing the folder\u{2026}"));
        }
        if let Some(error) = &sheet.error {
            body = body.child(
                div()
                    .id("root-error")
                    .role(Role::Alert)
                    .text_color(rgb(0xff453a))
                    .child(error.clone()),
            );
        }
        let mut actions = row().gap(px(8.)).pt(px(4.)).justify_end().child(
            widgets::push_quiet(ui, "root-cancel", "Cancel", send(HostsAction::DismissRoot), look)
                .disabled(sheet.running),
        );
        if let Some(plan) = &sheet.plan {
            let apply = |move_channels| send(HostsAction::ApplyRoot { move_channels });
            actions = if plan.channels.is_empty() {
                actions.child(
                    widgets::push_primary(ui, "root-change", "Change", apply(false), look).disabled(sheet.running),
                )
            } else {
                actions
                    .child(
                        widgets::push_quiet(ui, "root-new-only", "Use for new channels only", apply(false), look)
                            .disabled(sheet.running),
                    )
                    .child(
                        widgets::push_primary(ui, "root-move", "Move them", apply(true), look)
                            .disabled(!sheet.can_move()),
                    )
            };
        }
        let body = body.child(actions).test_support().focus_trap("root-trap", &self.sheet);
        div()
            .id("root-dialog")
            .role(Role::Group)
            .aria_label("Humata folder dialog")
            .test_support()
            .child(body)
            .into_any_element()
    }
}
