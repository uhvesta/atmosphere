//! The New channel sheet (CH-1).

use super::{Overlays, field, sheet, sync};
use crate::bridge::Ui;
use crate::theme::{self, Look};
use crate::widgets::{self, col, row};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use crate::chrome;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, NewChannel, NewChannelAction};
use humata2_clients::model::{ChannelSetup, SetupState, SetupStep};
use std::rc::Rc;

impl Overlays {
    /// The New channel sheet.
    pub(super) fn new_channel(
        &self,
        sheet_state: &NewChannel,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(setup) = &sheet_state.setup {
            return self.setup_progress(setup, sheet_state, look, ui);
        }
        sync(&self.name, &sheet_state.name, window, cx);
        sync(&self.repos, &sheet_state.repos, window, cx);
        sheet("new-channel", "New channel", look)
            .test_support()
            .child(widgets::title("New channel"))
            .child(field(
                "Name",
                Input::new(&self.name).aria_label("Channel name"),
            ))
            .child(field(
                "Repositories, separated by commas",
                Input::new(&self.repos).aria_label("Repositories"),
            ))
            .children(sheet_state.error.clone().map(|error| {
                widgets::error_banner(ui, "new-channel-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .justify_end()
                    .gap_2()
                    .child(widgets::ghost(
                        ui,
                        "new-channel-cancel",
                        "Cancel",
                        AppAction::Escape,
                    ))
                    .child(
                        widgets::primary(
                            ui,
                            "new-channel-create",
                            if sheet_state.creating {
                                "Making…"
                            } else {
                                "Make channel"
                            },
                            AppAction::NewChannel(NewChannelAction::Create),
                        )
                        .disabled(sheet_state.creating),
                    ),
            )
            .focus_trap("new-channel-trap", &self.name.focus_handle(cx))
            .into_any_element()
    }

    /// The sheet while the folders and clones run (frame 45, CH-5): a group
    /// per host, a bar per clone, Retry on a failed one and Cancel.
    fn setup_progress(
        &self,
        setup: &ChannelSetup,
        sheet_state: &NewChannel,
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let mut hosts: Vec<&humata2_clients::model::MachineId> = Vec::new();
        for step in &setup.steps {
            if !hosts.contains(&&step.machine) {
                hosts.push(&step.machine);
            }
        }
        let names = ui.read(|state| {
            hosts
                .iter()
                .map(|id| {
                    state
                        .workspace
                        .machines
                        .iter()
                        .find(|machine| &machine.id == *id)
                        .map_or_else(|| id.to_string(), |machine| machine.name.clone())
                })
                .collect::<Vec<_>>()
        });
        let groups = hosts.iter().zip(names).map(|(host, name)| {
            let steps: Vec<&SetupStep> = setup
                .steps
                .iter()
                .filter(|step| &step.machine == *host)
                .collect();
            host_group(&name, &steps, look, ui)
        });
        sheet("new-channel-setup", "Creating channel", look)
            .test_support()
            .child(widgets::title(format!("Creating # {}", setup.name)))
            .children(groups)
            .children(sheet_state.error.clone().map(|error| {
                widgets::error_banner(ui, "new-channel-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .gap_2()
                    .child(widgets::faint(
                        "The channel opens when every clone is done.",
                    ))
                    .child(div().flex_1())
                    .child(widgets::ghost(
                        ui,
                        "new-channel-setup-cancel",
                        "Cancel",
                        AppAction::NewChannel(NewChannelAction::CancelSetup),
                    )),
            )
            .into_any_element()
    }
}

/// One host's group: its folders and each clone.
fn host_group(name: &str, steps: &[&SetupStep], look: Look, ui: &Rc<Ui>) -> Div {
    let mut group = col()
        .gap(px(7.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(rgb(look.fill))
        .child(
            row()
                .gap(px(7.))
                .child(chrome::icon(IconName::Server, look.secondary, 13.))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(name.to_owned())),
        );
    for step in steps {
        group = group.child(match &step.repo {
            None => made_row(look).into_any_element(),
            Some(repo) => clone_row(repo, &step.state, look, ui),
        });
    }
    group
}

fn made_row(look: Look) -> Div {
    row()
        .gap(px(8.))
        .pl(px(8.))
        .child(chrome::icon(IconName::Check, GREEN, 12.))
        .child(widgets::dim("Folders made").text_color(rgb(look.secondary)))
}

fn bar(look: Look, percent: u8, color: u32) -> Div {
    let width = 160. * f32::from(percent.min(100)) / 100.;
    div()
        .w(px(160.))
        .h(px(4.))
        .rounded_full()
        .bg(rgb(look.fill))
        .flex_shrink_0()
        .child(div().h_full().w(px(width)).rounded_full().bg(rgb(color)))
}

fn clone_row(repo: &str, state: &SetupState, look: Look, ui: &Rc<Ui>) -> AnyElement {
    let line = |percent, color, mark: IconName, mark_color, text: String| {
        row()
            .gap(px(8.))
            .pl(px(8.))
            .child(chrome::icon(IconName::FolderGit2, look.secondary, 13.))
            .child(div().w(px(110.)).child(repo.to_owned()))
            .child(bar(look, percent, color))
            .child(chrome::icon(mark, mark_color, 12.))
            .child(div().text_color(rgb(look.secondary)).child(text))
            .into_any_element()
    };
    match state {
        SetupState::Waiting => line(
            0,
            look.accent,
            IconName::ArrowRight,
            look.tertiary,
            "Waiting".to_owned(),
        ),
        SetupState::Running { percent, detail } => line(
            *percent,
            look.accent,
            IconName::ArrowRight,
            look.tertiary,
            format!("{detail} {percent}%"),
        ),
        SetupState::Done => line(100, GREEN, IconName::Check, GREEN, "Done".to_owned()),
        SetupState::Failed { reason } => col()
            .id(SharedString::from(format!("clone-failed-{repo}")))
            .role(Role::Alert)
            .aria_label(format!("Clone of {repo} failed: {reason}"))
            .gap(px(3.))
            .child(
                row()
                    .gap(px(8.))
                    .pl(px(8.))
                    .child(chrome::icon(IconName::FolderGit2, look.secondary, 13.))
                    .child(div().w(px(110.)).child(repo.to_owned()))
                    .child(bar(look, 12, theme::RED))
                    .child(chrome::icon(IconName::CircleX, theme::RED, 12.))
                    .child(div().text_color(rgb(theme::RED)).child("Clone failed"))
                    .child(div().flex_1())
                    .child(widgets::send(
                        ui,
                        SharedString::from(format!("clone-retry-{repo}")),
                        "Retry",
                        AppAction::NewChannel(NewChannelAction::RetrySetup),
                    )),
            )
            .child(
                div()
                    .pl(px(30.))
                    .text_color(rgb(look.secondary))
                    .child(reason.clone()),
            )
            .into_any_element(),
    }
}

/// A finished clone's colour.
const GREEN: u32 = 0x30d158;
