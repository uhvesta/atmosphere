//! New desk session (DK-14, frame 40).

use super::{Overlays, agent_label, button, check, form_row, header, popup, primary, radio, secondary};
use crate::bridge::Ui;
use crate::chrome;
use super::super::sync;
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::Workspace;
use humata2_clients::desk::Access;
use humata2_clients::model::Agent;
use humata2_desk::{DeskAction, FolderPick, NewAction, NewForm};
use std::rc::Rc;

fn new_action(action: NewAction) -> AppAction {
    AppAction::Desk(DeskAction::New(action))
}

impl Overlays {
    /// The New desk session form.
    pub(super) fn desk_new(
        &self,
        form: &NewForm,
        preview: &str,
        workspace: &Workspace,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.desk.title, &form.title, window, cx);
        sync(&self.desk.path, &form.path, window, cx);
        if !form.adding_repo {
            sync(&self.desk.repo, "", window, cx);
        }
        let busy = form.starting;
        let machine = form
            .machine
            .as_ref()
            .and_then(|id| workspace.machine(id))
            .map_or("Choose a machine", |machine| machine.name.as_str())
            .to_owned();

        let title = div().h(px(26.)).child(
            Input::new(&self.desk.title)
                .small()
                .min_h(px(26.))
                .focus_bordered(false)
                .aria_label("Title"),
        );

        let agents = row()
            .flex_shrink_0()
            .p(px(2.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .children(Agent::ALL.into_iter().map(|agent| {
                let ui = ui.clone();
                let name = agent_label(agent);
                let selected = form.agent == agent;
                div()
                    .id(SharedString::from(format!(
                        "new-desk-agent-{}",
                        agent.name().to_lowercase()
                    )))
                    .role(Role::Tab)
                    .aria_label(name)
                    .aria_selected(selected)
                    .test_support()
                    .px(px(12.))
                    .py(px(2.))
                    .text_size(px(11.5))
                    .rounded(px(5.))
                    .when(selected, |x| x.bg(rgb(look.card)).shadow_sm())
                    .cursor_pointer()
                    .child(name)
                    .on_click(move |_, _, cx| ui.send(new_action(NewAction::Agent(agent)), cx))
            }));

        let mut machine_value = col().gap(px(6.)).child(
            row()
                .gap(px(8.))
                .child(popup(
                    ui,
                    look,
                    "new-desk-machine",
                    machine.into(),
                    new_action(NewAction::Machines(!form.machines_open)),
                ))
                .child(secondary(look, "Move it later with humata desk move")),
        );
        if form.machines_open {
            machine_value = machine_value.child(row().flex_wrap().gap(px(6.)).children(
                workspace.machines.iter().filter(|m| m.online).map(|m| {
                    button(
                        ui,
                        look,
                        SharedString::from(format!("new-desk-machine-{}", m.id.as_str())),
                        m.name.clone(),
                        new_action(NewAction::Machine(m.id.clone())),
                    )
                }),
            ));
        }

        let mut folder = col()
            .gap(px(8.))
            .child(radio(
                ui,
                look,
                "new-desk-folder-scratch",
                form.folder == FolderPick::Scratch,
                "A new scratch folder",
                format!("{preview} · deleted 7 days after you close it, unless you keep it"),
                new_action(NewAction::Folder(FolderPick::Scratch)),
            ))
            .child(radio(
                ui,
                look,
                "new-desk-folder-picked",
                form.folder == FolderPick::Picked,
                "A folder on the machine",
                "Pick one. Humata never deletes it.".to_owned(),
                new_action(NewAction::Folder(FolderPick::Picked)),
            ));
        if form.folder == FolderPick::Picked {
            folder = folder.child(
                div().pl(px(22.)).h(px(26.)).child(
                    Input::new(&self.desk.path)
                        .small()
                        .min_h(px(26.))
                        .focus_bordered(false)
                        .aria_label("Folder path"),
                ),
            );
        }

        let chips = row()
            .flex_wrap()
            .gap(px(6.))
            .children(form.repos.iter().map(|repo| {
                let ui = ui.clone();
                let remove = new_action(NewAction::RemoveRepo(repo.clone()));
                row()
                    .gap(px(5.))
                    .h(px(22.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .rounded(px(6.))
                    .bg(rgb(look.fill))
                    .border_1()
                    .border_color(rgb(look.line))
                    .text_size(px(12.))
                    .child(repo.clone())
                    .child(
                        div()
                            .id(SharedString::from(format!("new-desk-repo-remove-{repo}")))
                            .role(Role::Button)
                            .aria_label(format!("Remove {repo}"))
                            .test_support()
                            .cursor_pointer()
                            .child(chrome::icon(IconName::X, look.secondary, 11.))
                            .on_click(move |_, _, cx| ui.send(remove.clone(), cx)),
                    )
            }));
        let adder = if form.adding_repo {
            row()
                .gap(px(8.))
                .child(
                    div().w(px(220.)).h(px(26.)).child(
                        Input::new(&self.desk.repo)
                            .small()
                            .min_h(px(26.))
                            .focus_bordered(false)
                            .aria_label("Repo to add"),
                    ),
                )
                .child(button(
                    ui,
                    look,
                    "new-desk-repo-cancel",
                    "Cancel",
                    new_action(NewAction::AddingRepo(false)),
                ))
        } else {
            row()
                .gap(px(8.))
                .child(button(
                    ui,
                    look,
                    "new-desk-add-repo",
                    "Add a repo…",
                    new_action(NewAction::AddingRepo(true)),
                ))
                .child(secondary(look, "Optional. Cloned into the folder."))
        };
        let repos = col()
            .gap(px(6.))
            .when(!form.repos.is_empty(), |x| x.child(chips))
            .child(adder);

        let access = form.access;
        let switches = col()
            .gap(px(4.))
            .child(check(
                ui,
                look,
                "new-desk-access-boards".into(),
                "Read and change every channel's board".into(),
                access.boards,
                !busy,
                new_action(NewAction::Access(Access {
                    boards: !access.boards,
                    ..access
                })),
            ))
            .child(check(
                ui,
                look,
                "new-desk-access-channels".into(),
                "Make channels and add machines (asks you each time when off)".into(),
                access.channels_and_machines,
                !busy,
                new_action(NewAction::Access(Access {
                    channels_and_machines: !access.channels_and_machines,
                    ..access
                })),
            ));

        super::card("new-desk", "New desk session", 560., look)
            .gap(px(12.))
            .child(header(look, "New desk session", "outside any channel"))
            .child(form_row(look, "Title", title))
            .child(form_row(look, "Runs", row().child(agents)))
            .child(form_row(look, "Machine", machine_value))
            .child(form_row(look, "Folder", folder))
            .child(form_row(look, "Repos", repos))
            .child(form_row(look, "Agent access", switches))
            .children(
                form.error
                    .clone()
                    .map(|error| widgets::error_banner(ui, "new-desk-error", error, AppAction::Escape)),
            )
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(div().flex_1())
                    .child(button(ui, look, "new-desk-cancel", "Cancel", AppAction::Escape))
                    .child(primary(
                        ui,
                        look,
                        "new-desk-start",
                        if busy { "Starting…" } else { "Start" },
                        new_action(NewAction::Start),
                        busy,
                    )),
            )
            .focus_trap("new-desk-trap", &self.desk.title.focus_handle(cx))
            .into_any_element()
    }
}
