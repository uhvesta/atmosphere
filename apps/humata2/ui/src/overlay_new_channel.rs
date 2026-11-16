//! The New channel sheet (CH-1).

use super::{Overlays, sheet, sync};
use crate::bridge::Ui;
use crate::chrome;
use crate::theme::{self, Look};
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, NewChannel, NewChannelAction};
use humata2_clients::model::{ChannelSetup, MachineKind, SetupState, SetupStep};
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
        sync(&self.repos, &sheet_state.query, window, cx);
        let machines = ui.read(|state| state.workspace.machines.clone());
        let label = |text: &str| {
            div()
                .w(px(110.))
                .flex_shrink_0()
                .pt(px(4.))
                .flex()
                .justify_end()
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
        let input = |state: &Entity<InputState>, aria: &'static str| {
            div().h(px(26.)).flex_1().min_w_0().child(
                Input::new(state)
                    .small()
                    .min_h(px(26.))
                    .text_size(px(12.5))
                    .focus_bordered(false)
                    .aria_label(aria),
            )
        };
        let hint = |text: String| {
            div()
                .text_size(px(11.))
                .text_color(rgb(look.tertiary))
                .child(text)
        };
        let name_value = col()
            .gap(px(5.))
            .child(input(&self.name, "Channel name"))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(format!("Folder: ~/Humata/{}/", sheet_state.folder())),
            );
        let mut repo_list = col().id("new-channel-repo-list").gap(px(7.));
        let shown = sheet_state.shown_repos();
        for repo in &shown {
            let on = sheet_state.repos.iter().any(|picked| picked == repo);
            repo_list = repo_list.child(check(
                ui,
                look,
                SharedString::from(format!("new-channel-repo-{repo}")).into(),
                (*repo).to_owned().into(),
                on,
                true,
                Some(IconName::FolderGit2),
                None,
                AppAction::NewChannel(NewChannelAction::ToggleRepo((*repo).to_owned())),
            ));
        }
        let more = sheet_state.more_repos();
        let mut repos = col().gap(px(7.)).child(
            row()
                .gap(px(8.))
                .child(chrome::icon(IconName::Search, look.secondary, 12.))
                .child(input(&self.repos, "Search repos")),
        );
        repos = repos.child(hint("From gh on This Mac. Zero or more.".to_owned()));
        if let Some(error) = &sheet_state.repos_error {
            repos = repos.child(
                row()
                    .gap(px(8.))
                    .child(
                        div()
                            .id("new-channel-repos-error")
                            .role(Role::Alert)
                            .test_support()
                            .text_color(rgb(theme::RED))
                            .child(error.clone()),
                    )
                    .child(widgets::push_quiet(
                        ui,
                        "new-channel-repos-retry",
                        "Retry",
                        AppAction::NewChannel(NewChannelAction::RetryRepos),
                        look,
                    )),
            );
        } else if sheet_state.listed.is_none() {
            repos = repos.child(hint("Listing repos from gh…".to_owned()));
        } else {
            repos = repos.child(
                col()
                    .max_h(px(8. * 21. + 7. * 7.))
                    .overflow_hidden()
                    .child(repo_list.max_h(px(8. * 21. + 7. * 7.)).overflow_y_scroll()),
            );
            if more > 0 {
                repos = repos.child(hint(format!("and {more} more; type to narrow")));
            }
        }
        let machine_rows = machines.iter().map(|machine| {
            let on = sheet_state.machines.contains(&machine.id);
            let note = match machine.kind {
                MachineKind::ThisMac => None,
                _ if machine.online => Some("online"),
                _ => Some("offline"),
            };
            check(
                ui,
                look,
                SharedString::from(format!("new-channel-machine-{}", machine.id)).into(),
                machine.name.clone().into(),
                on,
                machine.online,
                Some(IconName::Server),
                note,
                AppAction::NewChannel(NewChannelAction::ToggleMachine(machine.id.clone())),
            )
        });
        let machines_col = col().gap(px(7.)).children(machine_rows);
        sheet("new-channel", "New channel", look)
            .w(px(560.))
            .max_w_full()
            .gap(px(12.))
            .p(px(20.))
            .rounded(px(12.))
            .text_size(px(12.5))
            .test_support()
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(look.text))
                    .child("New channel"),
            )
            .child(form_row("Name", name_value.into_any_element()))
            .child(form_row("Repos", repos.into_any_element()))
            .child(form_row("Machines", machines_col.into_any_element()))
            .children(sheet_state.error.clone().map(|error| {
                widgets::error_banner_in(look, ui, "new-channel-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(widgets::push_quiet(
                        ui,
                        "new-channel-cancel",
                        "Cancel",
                        AppAction::Escape,
                        look,
                    ))
                    .child(
                        widgets::push_primary(
                            ui,
                            "new-channel-create",
                            sheet_state.create_label(),
                            AppAction::NewChannel(NewChannelAction::Create),
                            look,
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
            .child(widgets::title_in(
                look,
                format!("Creating # {}", setup.name),
            ))
            .children(groups)
            .children(sheet_state.error.clone().map(|error| {
                widgets::error_banner_in(look, ui, "new-channel-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .gap_2()
                    .child(widgets::faint_in(
                        look,
                        "The channel opens when every clone is done.",
                    ))
                    .child(div().flex_1())
                    .child(widgets::ghost_in(
                        look,
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
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name.to_owned()),
                ),
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
        .child(widgets::dim_in(look, "Folders made").text_color(rgb(look.secondary)))
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

/// A check box row: box, small icon, words and an optional note. A row that
/// is not `enabled` is dimmed and ignores clicks.
#[allow(clippy::too_many_arguments)]
fn check(
    ui: &Rc<Ui>,
    look: Look,
    id: ElementId,
    text: SharedString,
    on: bool,
    enabled: bool,
    icon: Option<IconName>,
    note: Option<&'static str>,
    action: AppAction,
) -> impl IntoElement {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::CheckBox)
        .aria_label(text.clone())
        .aria_selected(on)
        .test_support()
        .gap(px(8.))
        .when(!enabled, |x| x.opacity(0.5))
        // The box keeps 7pt more room before the icon, as the design draws it.
        .child(
            div().pr(px(7.)).flex_shrink_0().child(
                row()
                    .size(px(14.))
                    .flex_shrink_0()
                    .rounded(px(4.))
                    .justify_center()
                    .when(on, |x| {
                        x.bg(rgb(look.accent))
                            .child(chrome::icon(IconName::Check, 0xffffff, 10.))
                    })
                    .when(!on, |x| {
                        x.border_1()
                            .border_color(rgb(look.tertiary))
                            .bg(rgb(look.card))
                    }),
            ),
        )
        .children(icon.map(|icon| chrome::icon(icon, look.secondary, 12.)))
        .child(div().min_w_0().child(text))
        .children(note.map(|note| {
            div()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(note)
        }))
        .when(enabled, |x| {
            x.cursor_pointer()
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
        })
}

/// A finished clone's colour.
const GREEN: u32 = 0x30d158;
