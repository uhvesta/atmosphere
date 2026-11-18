//! New desk session (DK-14, frame 40).

use super::{
    Overlays, button, check, form_row, form_row_wrapping, header, popup, primary, runs, secondary,
};
use crate::bridge::Ui;
use crate::chrome;
use crate::repo_search_field::{RepoField, repo_search_field};
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
use humata2_desk::{DeskAction, NewAction, NewForm};
use std::rc::Rc;

fn new_action(action: NewAction) -> AppAction {
    AppAction::Desk(DeskAction::New(action))
}

impl Overlays {
    /// The New desk session form.
    pub(super) fn desk_new(
        &self,
        form: &NewForm,
        workspace: &Workspace,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.desk.title, &form.title, window, cx);
        if !form.adding_repo {
            sync(&self.desk.repo, "", window, cx);
        }
        // REPOS (RL-8): the + Org field shows what is typed and takes focus as it opens.
        let adding_owner = ui.read(|state| state.new_desk_repos.adding_owner.clone());
        // AGENTSET (D725): the enabled agents, then the plain shell.
        let offered: Vec<Agent> = ui
            .read(|state| state.settings.offered_agents())
            .into_iter()
            .chain([Agent::Shell])
            .collect();
        self.sync_owner(
            adding_owner.as_deref().filter(|_| form.adding_repo),
            window,
            cx,
        );
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
                .text_size(px(12.5))
                .focus_bordered(false)
                .aria_label("Title"),
        );

        let agents = row()
            .flex_shrink_0()
            .p(px(2.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .children(offered.into_iter().map(|agent| {
                let ui = ui.clone();
                let name = runs(agent);
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
            div()
                .flex()
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

        // DK-14: no folder choice and no path; Humata makes the folder, and
        // the session is deleted on Close unless it holds a saved version
        // or the user keeps it (DK-2, DV-4).
        // Frame 40 sets the caption 3 pt down, level with the key.
        let folder = div().pt(px(3.)).child(secondary(
            look,
            "Humata makes a folder for it \u{b7} deleted 7 days after you close it unless it holds a saved version or you keep it",
        ));

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
            // REPOS (RL-5, DK-14): the one repo search; ticking adds or removes.
            let search = ui.read(|state| state.new_desk_repos.clone());
            let (rows, more) = search.rows(&form.repos, 50);
            let pick = |repo: &str| new_action(NewAction::ToggleRepo(repo.to_owned()));
            col()
                .gap(px(6.))
                .child(repo_search_field(
                    ui,
                    look,
                    RepoField {
                        id: "new-desk",
                        picker: humata2_app::RepoPicker::NewDeskSession,
                        hint: Some("Your library first, then GitHub. Zero or more."),
                        search: &search,
                        query: Input::new(&self.desk.repo)
                            .small()
                            .min_h(px(26.))
                            .focus_bordered(false)
                            .aria_label("Repo to add")
                            .into_any_element(),
                        owner: Input::new(&self.owner)
                            .small()
                            .min_h(px(26.))
                            .aria_label("Org or user")
                            .into_any_element(),
                        rows: &rows,
                        more,
                        checkable: true,
                        pick: &pick,
                        workspace,
                    },
                ))
                .child(row().child(button(
                    ui,
                    look,
                    "new-desk-repo-cancel",
                    "Done",
                    new_action(NewAction::AddingRepo(false)),
                )))
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
            ).whitespace_nowrap())
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
            ).whitespace_nowrap());

        super::card("new-desk", "New desk session", 560., look)
            .gap(px(12.))
            .child(header(look, "New desk session", "outside any channel"))
            .child(form_row(look, "Title", title))
            .child(form_row(look, "Runs", row().child(agents)))
            .child(form_row(look, "Machine", machine_value))
            .child(form_row_wrapping(look, "Folder", folder))
            .child(form_row(look, "Repos", repos))
            // Frame 40: both switch labels stay on one line, so the key
            // column gives way 2 px (110 to 108) and "Agent access" still
            // lines up right with the other keys.
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .w(px(108.))
                            .flex_shrink_0()
                            .pt(px(3.))
                            .flex()
                            .justify_end()
                            .text_color(rgb(look.secondary))
                            .child("Agent access"),
                    )
                    .child(div().flex_1().min_w_0().child(switches)),
            )
            .children(
                form.error
                    .clone()
                    .map(|error| widgets::error_banner_in(look, ui, "new-desk-error", error, AppAction::Escape)),
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
            .focus_trap("new-desk", &self.desk.title.focus_handle(cx))
            .into_any_element()
    }
}
