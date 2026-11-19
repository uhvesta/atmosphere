//! Settings › General › Repositories (ST-21): the default scope as chips
//! with an Include my repos switch, then the repo library with Remove per
//! repo and Forget all… behind a confirm.

use crate::bridge::Ui;
use crate::chrome;
use crate::theme::{self, Look};
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, used_line};
use humata2_settings::{RepositoriesAction as R, RepositoriesState, SettingsAction};
use std::rc::Rc;

fn send(action: R) -> AppAction {
    AppAction::Settings(SettingsAction::Repos(action))
}

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
        .overflow_hidden()
}

fn pill(ui: &Rc<Ui>, look: Look, id: String, label: String, aria: String, action: AppAction) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    row()
        .id(SharedString::from(id))
        .role(Role::Button)
        .aria_label(aria)
        .test_support()
        .h(px(22.))
        .px(px(9.))
        .rounded_full()
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .cursor_pointer()
        .child(label)
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

fn switch(ui: &Rc<Ui>, look: Look, on: bool) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id("settings-repos-include-me")
        .role(Role::Button)
        .aria_label(if on { "Include my repos: on" } else { "Include my repos: off" })
        .aria_selected(on)
        .test_support()
        .w(px(28.))
        .h(px(16.))
        .rounded_full()
        .p(px(2.))
        .bg(rgb(if on { 0x28cd41 } else { look.fill }))
        .flex()
        .when(on, |x| x.justify_end())
        .child(div().size(px(12.)).rounded_full().bg(rgb(0xffffff)))
        .cursor_pointer()
        .on_click(move |_, _, cx| ui.send(send(R::ToggleIncludeMe), cx))
}

/// The Repositories part of Settings › General.
pub fn section(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let repos: &RepositoriesState = &state.settings.repos;
    let now_ms = repos.now_ms;

    let mut chips = row().flex_wrap().gap(px(6.));
    for owner in repos.owners() {
        let ui = ui.clone();
        let remove = send(R::OwnerRemoved(owner.clone()));
        chips = chips.child(
            row()
                .gap(px(5.))
                .h(px(22.))
                .pl(px(8.))
                .pr(px(4.))
                .rounded_full()
                .bg(rgb(look.fill))
                .text_size(px(11.5))
                .child(owner.clone())
                .child(
                    row()
                        .id(SharedString::from(format!("settings-repos-owner-{owner}-remove")))
                        .role(Role::Button)
                        .aria_label(format!("Remove {owner} from the default scope"))
                        .test_support()
                        .size(px(14.))
                        .justify_center()
                        .rounded_full()
                        .cursor_pointer()
                        .child(chrome::icon(IconName::X, look.secondary, 10.))
                        .on_click(move |_, _, cx| ui.send(remove.clone(), cx)),
                ),
        );
    }
    for owner in repos.owner_suggestions() {
        chips = chips.child(pill(
            ui,
            look,
            format!("settings-repos-add-{owner}"),
            format!("+ {owner}"),
            format!("Add {owner} to the default scope"),
            send(R::OwnerAdded(owner.clone())),
        ));
    }
    let empty = repos.owners().is_empty();
    let scope = card(look)
        .child(
            col()
                .gap(px(8.))
                .px(px(14.))
                .py(px(10.))
                .child(
                    row()
                        .gap(px(16.))
                        .child(
                            col()
                                .flex_1()
                                .min_w_0()
                                .child("Search scope")
                                .child(dim(
                                    look,
                                    if empty {
                                        "Your repos, until you add an org or user. A search can widen it for itself."
                                    } else {
                                        "Repo searches stay inside these owners. A search can widen it for itself."
                                    },
                                )),
                        ),
                )
                .child(chips),
        )
        .child(
            row()
                .gap(px(16.))
                .px(px(14.))
                .py(px(10.))
                .border_t_1()
                .border_color(rgb(look.line))
                .child(
                    col()
                        .flex_1()
                        .min_w_0()
                        .child("Include my repos")
                        .child(dim(look, "The repos of your own gh login.")),
                )
                .child(switch(ui, look, repos.include_me())),
        );

    let mut library = card(look);
    let held = repos.library.as_deref().unwrap_or_default();
    library = library.child(
        row()
            .gap(px(16.))
            .px(px(14.))
            .py(px(10.))
            .child(
                col()
                    .flex_1()
                    .min_w_0()
                    .child("Repo library")
                    .child(dim(look, "Repos you added, most recently used first. Removing one keeps its channels and clones.")),
            )
            .child(widgets::push_quiet(
                ui,
                "settings-repos-forget",
                "Forget all…",
                send(R::ForgetAsked),
                look,
            )),
    );
    if repos.confirming_forget {
        let count = repos.library_len();
        library = library.child(
            row()
                .id("settings-repos-forget-confirm")
                .role(Role::Dialog)
                .aria_label("Forget all repos")
                .test_support()
                .gap(px(10.))
                .px(px(14.))
                .py(px(10.))
                .border_t_1()
                .border_color(rgb(look.line))
                .child(
                    div().flex_1().min_w_0().child(format!(
                        "Forget {count} {}? Nothing is deleted on disk or on GitHub; a repo returns to the library when you add it again.",
                        if count == 1 { "repo" } else { "repos" }
                    )),
                )
                .child(widgets::push_quiet(
                    ui,
                    "settings-repos-forget-cancel",
                    "Cancel",
                    send(R::ForgetCancelled),
                    look,
                ))
                .child(widgets::push_primary(
                    ui,
                    "settings-repos-forget-confirm-button",
                    "Forget all",
                    send(R::ForgetConfirmed),
                    look,
                )),
        );
    }
    if repos.library.is_none() {
        library = library.child(
            div()
                .px(px(14.))
                .py(px(10.))
                .child(dim(look, "Reading the library…")),
        );
    } else if held.is_empty() {
        library = library.child(
            div()
                .px(px(14.))
                .py(px(10.))
                .border_t_1()
                .border_color(rgb(look.line))
                .child(dim(look, "No repos yet. A repo you add to a channel or the Desk shows up here.")),
        );
    }
    for repo in held {
        let note = used_line(repo, now_ms, &state.workspace);
        library = library.child(
            row()
                .id(SharedString::from(format!("settings-repos-row-{}", repo.repo)))
                .gap(px(12.))
                .px(px(14.))
                .py(px(8.))
                .border_t_1()
                .border_color(rgb(look.line))
                .child(chrome::icon(IconName::FolderGit2, look.secondary, 12.))
                .child(
                    col()
                        .flex_1()
                        .min_w_0()
                        .child(repo.repo.clone())
                        .child(dim(look, format!("{} · {note}", repo.remote))),
                )
                .child(widgets::push_quiet(
                    ui,
                    SharedString::from(format!("settings-repos-remove-{}", repo.repo)),
                    "Remove",
                    send(R::Remove(repo.repo.clone())),
                    look,
                )),
        );
    }

    col()
        .gap(px(10.))
        .child(dim(look, "Repositories").font_weight(FontWeight::SEMIBOLD))
        .child(scope)
        .child(library)
        .children(repos.error.clone().map(|error| {
            div()
                .id("settings-repos-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .test_support()
                .text_size(px(11.5))
                .text_color(rgb(theme::RED))
                .child(error)
        }))
}
