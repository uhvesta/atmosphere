//! A channel's "Add a GitHub repo…" sheet (CH-6, CH-18, CH-19, frame 20): the
//! one repo search with plain click rows, like the Desk's Add repo. A click
//! on a row, or Return after typing or pasting a GitHub address, adds the
//! repo and closes the sheet. It shows no path and has no local-folder
//! option; the daemon places the clone.

use super::{Overlays, sheet, sync};
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::repo_search_field::{RepoField, repo_search_field};
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{self, col, row};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, ChannelAddRepo, ChannelAddRepoAction, Overlay};
use std::rc::Rc;

/// The sheet's text field.
pub(super) struct Inputs {
    query: Entity<InputState>,
}

impl Inputs {
    /// Makes the field and subscribes it: typing sends `Query`, Return adds
    /// what is typed.
    pub(super) fn new(
        window: &mut Window,
        cx: &mut Context<Overlays>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Self {
        let query = cx.new(|cx| InputState::new(window, cx).placeholder("Search repos or paste a GitHub address"));
        subscriptions.push(cx.subscribe(
            &query,
            |this: &mut Overlays, input, event: &InputEvent, cx| {
                // The field is shared by nothing else, but only this sheet listens.
                if !this
                    .ui
                    .read(|state| matches!(state.overlay(), Some(Overlay::ChannelAddRepo(_))))
                {
                    return;
                }
                match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.ui
                            .send(AppAction::ChannelAddRepo(ChannelAddRepoAction::Query(text)), cx);
                    }
                    InputEvent::PressEnter { .. } => this
                        .ui
                        .send(AppAction::ChannelAddRepo(ChannelAddRepoAction::Submit), cx),
                    InputEvent::Focus | InputEvent::Blur => {}
                }
            },
        ));
        Self { query }
    }
}

impl Overlays {
    /// Puts focus in the sheet's field.
    pub(super) fn channel_add_repo_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.channel_add_repo
            .query
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// The sheet.
    pub(super) fn channel_add_repo(
        &self,
        sheet_state: &ChannelAddRepo,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        sync(&self.channel_add_repo.query, &sheet_state.query, window, cx);
        self.sync_owner(sheet_state.search.adding_owner.as_deref(), window, cx);
        let (channel, workspace) = ui.read(|state| {
            (
                state
                    .workspace
                    .channel(&sheet_state.channel)
                    .map(|found| format!("#{}", found.name)),
                state.workspace.clone(),
            )
        });
        let adding = sheet_state.adding.is_some();
        let (rows, more) = sheet_state.search.rows(&[], 50);
        let pick = |repo: &str| {
            AppAction::ChannelAddRepo(ChannelAddRepoAction::Pick(repo.to_owned()))
        };
        let field = repo_search_field(
            ui,
            look,
            RepoField {
                id: "channel-add-repo",
                picker: humata2_app::RepoPicker::ChannelAddRepo,
                hint: Some("Your library first, then GitHub. Or paste a GitHub address."),
                search: &sheet_state.search,
                query: Input::new(&self.channel_add_repo.query)
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
                workspace: &workspace,
            },
        );
        sheet("channel-add-repo-dialog", "Add a GitHub repo", look)
            .w(px(520.))
            .max_w_full()
            .gap(px(12.))
            .p(px(20.))
            .rounded(px(12.))
            .text_size(px(12.5))
            .test_support()
            .child(
                row()
                    .gap(px(8.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Add a GitHub repo"),
                    )
                    .children(channel.map(|name| {
                        div()
                            .text_size(px(11.5))
                            .text_color(rgb(look.secondary))
                            .child(name)
                    })),
            )
            .child(col().gap(px(6.)).child(field))
            .children(sheet_state.adding.as_ref().map(|repo| {
                div()
                    .id("channel-add-repo-adding")
                    .role(Role::Status)
                    .aria_label(format!("Adding {repo}"))
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(format!(
                        "Adding {repo}. It is cloned on each of the channel's machines; this can take a moment."
                    ))
            }))
            .children(sheet_state.error.clone().map(|error| {
                widgets::error_banner_in(
                    look,
                    ui,
                    "channel-add-repo-error",
                    error,
                    AppAction::ChannelAddRepo(ChannelAddRepoAction::DismissError),
                )
            }))
            .child(
                row()
                    .justify_end()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(widgets::push_quiet(
                        ui,
                        "channel-add-repo-cancel",
                        "Cancel",
                        AppAction::Escape,
                        look,
                    ))
                    .child(
                        widgets::push_primary(
                            ui,
                            "channel-add-repo-submit",
                            if adding { "Adding…" } else { "Add repo" },
                            AppAction::ChannelAddRepo(ChannelAddRepoAction::Submit),
                            look,
                        )
                        .disabled(adding),
                    ),
            )
            .focus_trap(
                "channel-add-repo-trap",
                &self.channel_add_repo.query.focus_handle(cx),
            )
            .into_any_element()
    }
}
