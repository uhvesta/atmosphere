//! Add repo to this session (CH-21): a sheet over a running session, opened
//! from the header's ⋯ menu and from the Diff and Files panels. It is the one
//! repo search (RL-5); a click on a row, or Return on what was typed, adds
//! the repo.

use super::{Overlays, sheet, sync};
use crate::bridge::Ui;
use crate::repo_search_field::{RepoField, repo_search_field};
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{
    AppAction, RepoPicker, RepoSearchAction, SessionAddRepo, SessionAddRepoAction,
};
use std::rc::Rc;

/// Makes the sheet's query field and subscribes it: typing searches, Return
/// adds what was typed. Both apply only while this sheet is the picker on
/// screen.
pub(super) fn query_input(
    window: &mut Window,
    cx: &mut Context<Overlays>,
    subscriptions: &mut Vec<Subscription>,
) -> Entity<InputState> {
    let query = cx.new(|cx| {
        InputState::new(window, cx).placeholder("Search repos, or type owner/name")
    });
    subscriptions.push(cx.subscribe(
        &query,
        |this: &mut Overlays, input, event: &InputEvent, cx| {
            let on_screen = this.ui.read(RepoPicker::on_screen);
            if on_screen != Some(RepoPicker::SessionAddRepo) {
                return;
            }
            let action = match event {
                InputEvent::Change => AppAction::RepoSearch(
                    RepoPicker::SessionAddRepo,
                    RepoSearchAction::QueryChanged(input.read(cx).value().to_string()),
                ),
                InputEvent::PressEnter { .. } => {
                    AppAction::SessionAddRepo(SessionAddRepoAction::Add(None))
                }
                InputEvent::Focus | InputEvent::Blur => return,
            };
            this.ui.send(action, cx);
        },
    ));
    query
}

impl Overlays {
    /// The Add repo to this session sheet.
    pub(super) fn session_add_repo(
        &self,
        sheet_state: &SessionAddRepo,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.session_repo, &sheet_state.search.query, window, cx);
        sync(
            &self.owner,
            sheet_state.search.adding_owner.as_deref().unwrap_or_default(),
            window,
            cx,
        );
        let (workspace, title) = ui.read(|state| {
            (
                state.workspace.clone(),
                state
                    .workspace
                    .session(&sheet_state.session)
                    .map_or_else(|| sheet_state.session.to_string(), |found| found.title.clone()),
            )
        });
        let (rows, more) = sheet_state.search.rows(&[], 50);
        let pick = |repo: &str| {
            AppAction::SessionAddRepo(SessionAddRepoAction::Add(Some(repo.to_owned())))
        };
        let field = repo_search_field(
            ui,
            look,
            RepoField {
                id: "session-add-repo",
                picker: RepoPicker::SessionAddRepo,
                hint: Some(
                    "It is cloned in this session on its branch, and the agent is told where it is.",
                ),
                search: &sheet_state.search,
                query: Input::new(&self.session_repo)
                    .small()
                    .min_h(px(26.))
                    .text_size(px(12.5))
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
                checkable: false,
                pick: &pick,
                workspace: &workspace,
            },
        );
        let adding = sheet_state.adding.is_some();
        sheet("session-add-repo", "Add repo to this session", look)
            .w(px(520.))
            .max_w_full()
            .gap(px(12.))
            .p(px(20.))
            .rounded(px(12.))
            .text_size(px(12.5))
            .test_support()
            .child(
                col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(look.text))
                            .child("Add repo to this session"),
                    )
                    .child(
                        div()
                            .text_color(rgb(look.secondary))
                            .child(title),
                    ),
            )
            .child(field)
            .children(sheet_state.error.clone().map(|error| {
                widgets::error_banner_in(look, ui, "session-add-repo-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(widgets::push_quiet(
                        ui,
                        "session-add-repo-cancel",
                        "Cancel",
                        AppAction::Escape,
                        look,
                    ))
                    .child(
                        widgets::push_primary(
                            ui,
                            "session-add-repo-add",
                            if adding { "Adding…" } else { "Add repo" },
                            AppAction::SessionAddRepo(SessionAddRepoAction::Add(None)),
                            look,
                        )
                        .disabled(adding),
                    ),
            )
            .focus_trap("session-add-repo-trap", &self.session_repo.focus_handle(cx))
            .into_any_element()
    }
}
