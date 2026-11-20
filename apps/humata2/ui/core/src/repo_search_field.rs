//! The repo search field (RL-5 to RL-10; frames 108 to 112), the one a repo
//! is picked with: the scope as chips (each with ✕, **+ Org**, **All of
//! GitHub**, **Save as default** while the scope differs), the text field,
//! then the Library group and the GitHub group under their headings, and
//! under GitHub the one line that says why it is missing. It draws the
//! `RepoSearchState` it is given and sends `AppAction::RepoSearch`; the
//! text inputs belong to the overlay that shows it.

use crate::bridge::Ui;
use crate::chrome;
use crate::theme::{self, Look};
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::{ObservedElement, TestTextExt as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{
    AppAction, PickRow, RepoGroup, RepoPicker, RepoSearchAction as S, RepoSearchState, used_line,
};
use humata2_clients::Workspace;
use std::rc::Rc;

/// What the field draws.
pub struct RepoField<'a> {
    /// Ids and labels start with this ("new-channel", "desk-add-repo").
    pub id: &'static str,
    /// The picker it belongs to; its actions carry it.
    pub picker: RepoPicker,
    /// A line of words between the text field and the list, if any.
    pub hint: Option<&'static str>,
    /// The search being shown.
    pub search: &'a RepoSearchState,
    /// The text input for the query, owned by the overlay.
    pub query: AnyElement,
    /// The text input after **+ Org**, owned by the overlay.
    pub owner: AnyElement,
    /// The rows to list (`RepoSearchState::rows`), and how many matches
    /// the limit left out.
    pub rows: &'a [PickRow],
    /// How many matches `rows` leaves out.
    pub more: usize,
    /// Rows have a check box (several repos); otherwise a click adds one.
    pub checkable: bool,
    /// What clicking a repo's row sends.
    pub pick: &'a dyn Fn(&str) -> AppAction,
    /// For where a library repo was last used.
    pub workspace: &'a Workspace,
}

/// The field: chips, text, groups, problem line.
pub fn repo_search_field(ui: &Rc<Ui>, look: Look, field: RepoField<'_>) -> Div {
    let RepoField {
        id,
        picker,
        hint,
        search,
        query,
        owner,
        rows,
        more,
        checkable,
        pick,
        workspace,
    } = field;
    let send = |action: S| AppAction::RepoSearch(picker, action);

    let mut chips = row().id(SharedString::from(format!("{id}-scope"))).flex_wrap().gap(px(6.));
    for chip in search.chips() {
        chips = chips.child(chip_pill(ui, look, id, &chip.label, chip.unknown, send(S::ScopeChipRemoved(chip.chip))));
    }
    if search.adding_owner.is_none() {
        chips = chips.child(action_pill(
            ui,
            look,
            format!("{id}-add-org"),
            "+ Org",
            "Add an org or user to this search",
            send(S::AddOwnerOpened),
        ));
    }
    if !matches!(search.scope(), humata2_clients::repos::RepoScope::AllOfGitHub) {
        chips = chips.child(action_pill(
            ui,
            look,
            format!("{id}-all-of-github"),
            "All of GitHub",
            "Search all of GitHub, this once",
            send(S::AllOfGitHubPicked),
        ));
    }
    if search.widened() {
        chips = chips.child(action_pill(
            ui,
            look,
            format!("{id}-scope-reset"),
            "Reset",
            "Go back to the saved scope",
            send(S::ScopeReset),
        ));
    }
    if search.can_save_default() {
        chips = chips.child(
            action_pill(
                ui,
                look,
                format!("{id}-save-default"),
                "Save as default",
                "Save this scope as the default",
                send(S::SaveScopeAsDefault),
            )
            .text_color(rgb(look.accent)),
        );
    }

    let owner_suggestions = search.owner_suggestions();
    let owner_count = owner_suggestions.len();
    let highlighted_owner = search.highlighted_index(owner_count);
    let mut owner_row = None;
    if search.adding_owner.is_some() {
        let mut suggestions = row().flex_wrap().gap(px(6.));
        for (idx, name) in owner_suggestions.iter().enumerate() {
            let selected = highlighted_owner == Some(idx);
            let pill = action_pill(
                ui,
                look,
                format!("{id}-suggest-{name}"),
                name,
                &format!("Add {name} to this search"),
                send(S::OwnerAdded(name.clone())),
            )
            .when(selected, |x| {
                x.bg(rgb(look.fill))
                    .border_color(rgb(look.accent))
                    .text_color(rgb(look.text))
            });
            suggestions = suggestions.child(pill);
        }
        owner_row = Some(
            col()
                .gap(px(6.))
                .child(
                    row()
                        .gap(px(8.))
                        .child(div().h(px(26.)).flex_1().min_w_0().child(owner))
                        .child(widgets::push_quiet(
                            ui,
                            SharedString::from(format!("{id}-owner-cancel")),
                            "Cancel",
                            send(S::AddOwnerClosed),
                            look,
                        )),
                )
                .child(suggestions),
        );
    }

    let highlighted_row = search
        .adding_owner
        .is_none()
        .then(|| search.highlighted_index(rows.len()))
        .flatten();
    let mut list = col().id(SharedString::from(format!("{id}-repo-list"))).gap(px(7.));
    let mut shown_group: Option<RepoGroup> = None;
    let now_ms = search.now_ms;
    for (idx, pick_row) in rows.iter().enumerate() {
        if let Some(group) = pick_row.group
            && shown_group != Some(group)
        {
            shown_group = Some(group);
            list = list.child(heading(
                look,
                match group {
                    RepoGroup::Library => "Library".to_owned(),
                    RepoGroup::GitHub => search.github_heading(),
                },
            ));
        }
        list = list.child(repo_row(
            ui,
            look,
            id,
            pick_row,
            checkable,
            highlighted_row == Some(idx),
            pick(&pick_row.repo),
            now_ms,
            workspace,
        ));
    }
    let problem = search.problem().map(str::to_owned);
    if problem.is_some() && shown_group != Some(RepoGroup::GitHub) {
        list = list.child(heading(look, search.github_heading()));
    }
    if let Some(problem) = problem {
        list = list.child(
            row()
                .gap(px(8.))
                .items_start()
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-repos-problem")))
                        .role(Role::Alert)
                        .aria_label(problem.clone())
                        .test_support()
                        .flex_1()
                        .min_w_0()
                        .text_color(rgb(theme::RED))
                        .child(problem),
                )
                .child(widgets::push_quiet(
                    ui,
                    SharedString::from(format!("{id}-repos-retry")),
                    "Retry",
                    send(S::Retry),
                    look,
                )),
        );
    } else if let Some(hint) = search.too_short() {
        list = list.child(hint_line(look, hint.to_owned()));
    } else if search.listing() {
        list = list.child(hint_line(look, "Looking…".to_owned()));
    }
    if more > 0 {
        list = list.child(hint_line(look, format!("and {more} more; type to narrow")));
    }
    let list = col()
        .max_h(px(8. * 21. + 7. * 7.))
        .overflow_hidden()
        .child(list.max_h(px(8. * 21. + 7. * 7.)).overflow_y_scroll());

    let row_actions: Vec<AppAction> = rows.iter().map(|row| pick(&row.repo)).collect();
    let ui_keys = ui.clone();

    col()
        .gap(px(7.))
        .capture_key_down(move |event, _, cx| {
            let modifiers = &event.keystroke.modifiers;
            // macOS marks arrow keys with the function flag, so it doesn't count here.
            if modifiers.platform || modifiers.control || modifiers.alt || modifiers.shift {
                return;
            }
            let key = event.keystroke.key.as_str();
            if !matches!(key, "down" | "up" | "tab" | "enter") {
                return;
            }
            let adding_owner = ui_keys.read(|state| {
                picker
                    .search(state)
                    .is_some_and(|search| search.adding_owner.is_some())
            });
            if adding_owner {
                match key {
                    "down" if owner_count > 0 => {
                        cx.stop_propagation();
                        ui_keys.send(
                            AppAction::RepoSearch(
                                picker,
                                S::MoveHighlight {
                                    delta: 1,
                                    count: owner_count,
                                },
                            ),
                            cx,
                        );
                    }
                    "up" if owner_count > 0 => {
                        cx.stop_propagation();
                        ui_keys.send(
                            AppAction::RepoSearch(
                                picker,
                                S::MoveHighlight {
                                    delta: -1,
                                    count: owner_count,
                                },
                            ),
                            cx,
                        );
                    }
                    "tab" if owner_count > 0 => {
                        cx.stop_propagation();
                        ui_keys.send(AppAction::RepoSearch(picker, S::CompleteOwner), cx);
                    }
                    "enter" => {
                        cx.stop_propagation();
                        ui_keys.send(AppAction::RepoSearch(picker, S::OwnerEntered), cx);
                    }
                    _ => {}
                }
                return;
            }
            if row_actions.is_empty() {
                return;
            }
            match key {
                "down" => {
                    cx.stop_propagation();
                    ui_keys.send(
                        AppAction::RepoSearch(
                            picker,
                            S::MoveHighlight {
                                delta: 1,
                                count: row_actions.len(),
                            },
                        ),
                        cx,
                    );
                }
                "up" => {
                    cx.stop_propagation();
                    ui_keys.send(
                        AppAction::RepoSearch(
                            picker,
                            S::MoveHighlight {
                                delta: -1,
                                count: row_actions.len(),
                            },
                        ),
                        cx,
                    );
                }
                "tab" => {
                    let (idx, can_tab_pick) = ui_keys.read(|state| {
                        let Some(search) = picker.search(state) else {
                            return (0, !checkable);
                        };
                        let idx = search.highlighted_index(row_actions.len()).unwrap_or(0);
                        let can_pick =
                            !checkable || !search.query.trim().is_empty() || search.highlighted > 0;
                        (idx, can_pick)
                    });
                    if can_tab_pick {
                        cx.stop_propagation();
                        ui_keys.send(row_actions[idx].clone(), cx);
                    }
                }
                "enter" => {
                    let idx = ui_keys.read(|state| {
                        picker
                            .search(state)
                            .and_then(|search| search.highlighted_index(row_actions.len()))
                            .unwrap_or(0)
                    });
                    cx.stop_propagation();
                    ui_keys.send(row_actions[idx].clone(), cx);
                }
                _ => {}
            }
        })
        .child(chips)
        .children(owner_row)
        .children(search.save_error.clone().map(|error| {
            div()
                .id(SharedString::from(format!("{id}-save-error")))
                .role(Role::Alert)
                .aria_label(error.clone())
                .test_support()
                .text_color(rgb(theme::RED))
                .child(error)
        }))
        .child(
            row()
                .gap(px(8.))
                .child(chrome::icon(IconName::Search, look.secondary, 12.))
                .child(div().h(px(26.)).flex_1().min_w_0().child(query)),
        )
        .children(hint.map(|text| hint_line(look, text.to_owned())))
        .child(list)
}

fn heading(look: Look, text: String) -> impl IntoElement {
    div()
        .pt(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.tertiary))
        .child(text.clone())
        .test_text(text)
}

fn hint_line(look: Look, text: String) -> Div {
    div()
        .text_size(px(11.))
        .text_color(rgb(look.tertiary))
        .child(text)
}

/// A scope chip: its name and a ✕ that narrows the search. An owner GitHub
/// does not know says so (RL-10).
fn chip_pill(ui: &Rc<Ui>, look: Look, id: &str, label: &str, unknown: bool, remove: AppAction) -> Div {
    let ui = ui.clone();
    row()
        .gap(px(5.))
        .h(px(22.))
        .pl(px(8.))
        .pr(px(4.))
        .rounded_full()
        .bg(rgb(look.fill))
        .text_size(px(11.5))
        .child(label.to_owned())
        .children(unknown.then(|| {
            div()
                .text_color(rgb(theme::RED))
                .child("no such org or user")
        }))
        .child(
            row()
                .id(SharedString::from(format!("{id}-chip-{label}-remove")))
                .role(Role::Button)
                .aria_label(format!("Remove {label} from this search"))
                .test_support()
                .size(px(14.))
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .child(chrome::icon(IconName::X, look.secondary, 10.))
                .on_click(move |_, _, cx| ui.send(remove.clone(), cx)),
        )
}

/// A quiet pill that sends `action`.
fn action_pill(
    ui: &Rc<Ui>,
    look: Look,
    id: String,
    label: &str,
    aria: &str,
    action: AppAction,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    row()
        .id(SharedString::from(id))
        .role(Role::Button)
        .aria_label(aria.to_owned())
        .test_support()
        .h(px(22.))
        .px(px(9.))
        .rounded_full()
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .cursor_pointer()
        .child(label.to_owned())
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

/// One repo: a check box when several can be picked, its name, and a note:
/// where a library repo was last used, or GitHub's description.
#[allow(clippy::too_many_arguments)]
fn repo_row(
    ui: &Rc<Ui>,
    look: Look,
    id: &str,
    pick_row: &PickRow,
    checkable: bool,
    highlighted: bool,
    action: AppAction,
    now_ms: i64,
    workspace: &Workspace,
) -> impl IntoElement {
    let ui = ui.clone();
    let note = match &pick_row.library {
        Some(held) => used_line(held, now_ms, workspace),
        None => pick_row.description.clone(),
    };
    let on = pick_row.ticked;
    row()
        .id(SharedString::from(format!("{id}-repo-{}", pick_row.repo)))
        .role(if checkable { Role::CheckBox } else { Role::Button })
        .aria_label(pick_row.repo.clone())
        .aria_selected(on)
        .test_support()
        .px(px(6.))
        .py(px(2.))
        .rounded(px(6.))
        .when(highlighted, |x| x.bg(rgb(look.fill)))
        .gap(px(8.))
        .when(checkable, |x| {
            x.child(
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
        })
        .child(chrome::icon(IconName::FolderGit2, look.secondary, 12.))
        .child(div().min_w_0().child(pick_row.repo.clone()))
        .child(div().flex_1())
        .children((!note.is_empty()).then(|| {
            div()
                .min_w_0()
                .truncate()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(note)
        }))
        .children(pick_row.private.then(|| {
            div()
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(rgb(look.tertiary))
                .child("private")
        }))
        .cursor_pointer()
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}
