//! The folder bar above a Desk session's tabs (DK-3, frames 37 to 39):
//! where the session works, with a "scratch" chip for a scratch folder, the
//! machine it runs on and the machines it can move to, a chip per repo with
//! its ✕, Add repo, and Keep folder for a scratch folder nobody kept.
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestTextExt as _;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::Session;
use humata2_desk::{DeskAction as A, Menu as DeskMenu};
use std::rc::Rc;

/// The hairline between the bar's parts.
fn divider(look: Look) -> Div {
    div()
        .w(px(1.))
        .h(px(16.))
        .flex_shrink_0()
        .bg(rgb(look.line))
}

/// The machines the session can move to (DK-3): the workspace's other
/// online machines, hanging under the picker and drawn over the tabs.
fn machine_menu(ui: &Rc<Ui>, state: &AppState, session: &Session, look: Look) -> impl IntoElement {
    let others: Vec<_> = state
        .workspace
        .machines
        .iter()
        .filter(|machine| machine.online && machine.id != session.machine)
        .collect();
    col()
        .id("desk-bar-machine-menu")
        .role(Role::ListBox)
        .aria_label("Move to a machine")
        .absolute()
        .top(px(26.))
        .left(px(0.))
        .w(px(220.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .when(others.is_empty(), |list| {
            list.child(
                div()
                    .px(px(8.))
                    .py(px(5.))
                    .text_color(rgb(look.tertiary))
                    .child("No other machine is online."),
            )
        })
        .children(others.into_iter().map(|machine| {
            let ui = ui.clone();
            let action = AppAction::Desk(A::Move(session.id.clone(), machine.id.clone()));
            row()
                .id(SharedString::from(format!(
                    "desk-bar-machine-{}",
                    machine.id
                )))
                .role(Role::ListBoxOption)
                .aria_label(machine.name.clone())
                .gap(px(8.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(move |x| x.bg(rgb(look.fill)))
                .child(chrome::icon(IconName::Server, look.secondary, 12.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_nowrap()
                        .child(machine.name.clone()),
                )
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
                .test_support()
        }))
        .test_support()
}

/// The folder bar of `session`, or `None` when it isn't a Desk session.
/// While a change to the session is in flight its controls are dimmed and
/// do nothing.
pub(crate) fn bar(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Option<AnyElement> {
    let record = state.desk.record(&session.id)?;
    let id = &session.id;
    let busy = state.desk.pending.contains(id);
    let fade = if busy { 0.5 } else { 1. };
    let path = record.folder.path().to_owned();

    let machine_open = state.desk.menu == Some(DeskMenu::Machine(id.clone()));
    let machine_name = state.workspace.machine(&session.machine).map_or_else(
        || session.machine.to_string(),
        |machine| machine.name.clone(),
    );
    let picker = {
        let ui = ui.clone();
        let toggle = AppAction::Desk(A::Menu(
            (!machine_open).then(|| DeskMenu::Machine(id.clone())),
        ));
        row()
            .id("desk-bar-machine")
            .role(Role::Button)
            .aria_label(format!("Machine: {machine_name}"))
            .flex_shrink_0()
            .gap(px(10.))
            .opacity(fade)
            .child(chrome::icon(IconName::Server, look.secondary, 12.))
            .child(
                row()
                    .gap(px(3.))
                    .child(machine_name)
                    .child(chrome::icon(IconName::ChevronDown, look.secondary, 12.)),
            )
            .when(!busy, |x| {
                x.cursor_pointer()
                    .on_click(move |_, _, cx| ui.send(toggle.clone(), cx))
            })
            .test_support()
    };
    let machine = div()
        .relative()
        .flex_shrink_0()
        .child(picker)
        .when(machine_open, |wrap| {
            wrap.child(deferred(machine_menu(ui, state, session, look)))
        });

    let repos = record.repos.iter().map(|repo| {
        let ui = ui.clone();
        let remove = AppAction::Desk(A::RemoveRepo(id.clone(), repo.clone()));
        row()
            .gap(px(4.))
            .px(px(7.))
            .h(px(20.))
            .flex_shrink_0()
            .rounded(px(5.))
            .bg(rgb(look.fill))
            .child(chrome::icon(IconName::FolderGit2, look.secondary, 12.))
            .child(repo.clone())
            .child(
                div()
                    .id(SharedString::from(format!("desk-bar-repo-remove-{repo}")))
                    .role(Role::Button)
                    .aria_label(format!("Remove {repo}"))
                    .flex_shrink_0()
                    .opacity(fade)
                    .child(chrome::icon(IconName::X, look.tertiary, 12.))
                    .when(!busy, |x| {
                        x.cursor_pointer()
                            .on_click(move |_, _, cx| ui.send(remove.clone(), cx))
                    })
                    .test_support(),
            )
    });

    // The button opens `Menu::AddRepo`; the overlay layer draws the sheet
    // with the field for the repo's name (`overlay_desk_add_repo.rs`).
    let add = {
        let ui = ui.clone();
        let add_open = state.desk.menu == Some(DeskMenu::AddRepo(id.clone()));
        let toggle = AppAction::Desk(A::Menu(
            (!add_open).then(|| DeskMenu::AddRepo(id.clone())),
        ));
        row()
            .id("desk-bar-add-repo")
            .role(Role::Button)
            .aria_label("Add repo")
            .flex_shrink_0()
            .gap(px(4.))
            .opacity(fade)
            .text_color(rgb(look.accent))
            .child(chrome::icon(IconName::Plus, look.accent, 12.))
            .when(record.repos.is_empty(), |add| add.child("Add repo"))
            .when(!busy, |x| {
                x.cursor_pointer()
                    .on_click(move |_, _, cx| ui.send(toggle.clone(), cx))
            })
            .test_support()
    };

    let keep = record.folder.expires().then(|| {
        let ui = ui.clone();
        let keep = AppAction::Desk(A::Keep(id.clone()));
        div()
            .id("desk-bar-keep")
            .role(Role::Button)
            .aria_label("Keep folder")
            .flex_shrink_0()
            .opacity(fade)
            .text_color(rgb(look.accent))
            .child("Keep folder")
            .when(!busy, |x| {
                x.cursor_pointer()
                    .on_click(move |_, _, cx| ui.send(keep.clone(), cx))
            })
            .test_support()
    });

    Some(
        row()
            .id("desk-bar")
            .role(Role::Toolbar)
            .aria_label("Desk folder")
            .gap(px(10.))
            .h(px(34.))
            .px(px(14.))
            .flex_shrink_0()
            .bg(rgb(look.content))
            .border_b_1()
            .border_color(rgb(look.line))
            .text_size(px(12.))
            .child(chrome::icon(IconName::Folder, look.secondary, 12.))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_family("Menlo")
                    .text_size(px(11.5))
                    .child(path.clone())
                    .test_text(path),
            )
            // Frame 38: a kept folder no longer carries the chip.
            .when(record.folder.expires(), |bar| {
                bar.child(
                    div()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .text_size(px(11.))
                        .text_color(rgb(look.secondary))
                        .flex_shrink_0()
                        .child("scratch"),
                )
            })
            .child(divider(look))
            .child(machine)
            .child(divider(look))
            .children(repos)
            .child(add)
            .child(div().flex_1())
            .children(keep)
            .test_support()
            .into_any_element(),
    )
}
