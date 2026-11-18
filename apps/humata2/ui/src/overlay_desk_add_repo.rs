//! Add repo, from a Desk session's folder bar (DK-3): type `owner/name` and
//! press Return; the repo's default branch is cloned into the folder. No
//! frame draws the field, so it is a small sheet built from the same parts
//! as New desk session.

use super::{Overlays, button, card, header, secondary};
use crate::bridge::Ui;
use crate::theme::Look;
use crate::repo_search_field::{RepoField, repo_search_field};
use crate::widgets::{col, row};
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::Workspace;
use humata2_clients::model::SessionId;
use humata2_desk::DeskAction;
use std::rc::Rc;

impl Overlays {
    /// The Add repo sheet for `session`.
    pub(super) fn desk_add_repo(
        &self,
        session: &SessionId,
        workspace: &Workspace,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = workspace
            .session(session)
            .map_or_else(|| session.to_string(), |found| found.title.clone());
        let adding = ui.read(|state| state.desk_repos.adding_owner.clone());
        self.sync_owner(adding.as_deref(), window, cx);
        let add = {
            let ui = ui.clone();
            let input = self.desk.repo.clone();
            let session = session.clone();
            row()
                .id("desk-add-repo-add")
                .role(Role::Button)
                .aria_label("Add repo")
                .test_support()
                .h(px(24.))
                .px(px(11.))
                .rounded(px(6.))
                .bg(rgb(look.accent))
                .text_color(rgb(0xffffff))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .flex_shrink_0()
                .cursor_pointer()
                .child("Add repo")
                .on_click(move |_, _, cx| {
                    let text = input.read(cx).value().to_string();
                    ui.send(
                        AppAction::Desk(DeskAction::AddRepo(session.clone(), text)),
                        cx,
                    );
                })
        };
        card("desk-add-repo-dialog", "Add repo", 440., look)
            .gap(px(12.))
            .child(header(look, "Add repo", title))
            .child(
                col()
                    .gap(px(6.))
                    .child({
                        let search = ui.read(|state| state.desk_repos.clone());
                        let (rows, more) = search.rows(&[], 50);
                        let pick = |repo: &str| {
                            AppAction::Desk(DeskAction::AddRepo(session.clone(), repo.to_owned()))
                        };
                        repo_search_field(
                            ui,
                            look,
                            RepoField {
                                id: "desk-add-repo",
                                picker: humata2_app::RepoPicker::DeskAddRepo,
                                hint: None,
                                search: &search,
                                query: Input::new(&self.desk.repo)
                                    .small()
                                    .min_h(px(26.))
                                    .aria_label("Repo to add")
                                    .into_any_element(),
                                owner: Input::new(&self.owner)
                                    .small()
                                    .min_h(px(26.))
                                    .aria_label("Org or user")
                                    .into_any_element(),
                                rows: &rows,
                                more,
                                checkable: false,
                                pick: &pick,
                                workspace,
                            },
                        )
                    })
                    .child(secondary(
                        look,
                        "Its default branch is cloned into this session's folder.",
                    )),
            )
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(div().flex_1())
                    .child(button(
                        ui,
                        look,
                        "desk-add-repo-cancel",
                        "Cancel",
                        AppAction::Escape,
                    ))
                    .child(add),
            )
            .focus_trap("desk-add-repo-dialog", &self.desk.repo.focus_handle(cx))
            .into_any_element()
    }
}
