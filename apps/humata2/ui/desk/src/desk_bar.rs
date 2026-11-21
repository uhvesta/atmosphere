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
use gpui_kit::base::{ObservedElement, TestTextExt as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::desk::Folder;
use humata2_clients::model::{Session, SessionId};
use humata2_desk::{DeskAction as A, Menu as DeskMenu};
use std::rc::Rc;

/// The folder bar of a channel's own Desk session (frames 93 and 94): its
/// scratch folder, with Copy path in place of the path (PL-10), and Close
/// session, which asks first (CD-4).
pub fn channel_bar(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
) -> Option<AnyElement> {
    let record = state.desk.channel_record(&session.id)?;
    let busy = state.desk.pending.contains(&session.id);
    let close = AppAction::Desk(A::AskClose(session.id.clone()));
    let lost = record.target_gone;
    let fix = lost.then(|| target_fix(ui, state, session, look));
    let ui = ui.clone();
    let path = record.desk.folder.path().to_owned();
    // PL-10: the path is hidden; Copy path copies it.
    let copy_path = div()
        .id("channel-desk-bar-copy-path")
        .role(Role::Button)
        .aria_label("Copy path")
        .flex_shrink_0()
        .text_color(rgb(look.accent))
        .cursor_pointer()
        .child("Copy path")
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
        })
        .test_support();
    Some(
        row()
            .id("channel-desk-bar")
            .role(Role::Toolbar)
            .aria_label("Desk folder")
            .gap_x(px(10.))
            .gap_y(px(4.))
            .min_h(px(34.))
            .px(px(14.))
            .py(px(6.))
            .flex_wrap()
            .flex_shrink_0()
            .bg(rgb(look.content))
            .border_b_1()
            .border_color(rgb(look.line))
            .text_size(px(12.))
            .child(chrome::icon(IconName::Folder, look.secondary, 12.))
            .child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .child("Desk folder")
                    .test_text("Desk folder".to_owned()),
            )
            .child(copy_path)
            .child(
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
            // CD-5: the target went while the folder held Markdown files, so
            // the session stopped; Close (below) asks, and offers the files.
            .children(lost.then(|| {
                div()
                    .id("channel-desk-bar-target-gone")
                    .role(Role::Status)
                    .aria_label("Target gone")
                    .px(px(6.))
                    .py(px(1.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(rgb(crate::theme::ORANGE))
                    .text_size(px(11.))
                    .text_color(rgb(crate::theme::ORANGE))
                    .flex_shrink_0()
                    .child("Target gone")
                    .test_support()
            }))
            .children(fix)
            .child(div().flex_1())
            .child(
                div()
                    .id("channel-desk-bar-close")
                    .role(Role::Button)
                    .aria_label("Close session")
                    .flex_shrink_0()
                    .opacity(if busy { 0.5 } else { 1. })
                    .text_color(rgb(look.accent))
                    .child("Close session")
                    .when(!busy, |x| {
                        x.cursor_pointer()
                            .on_click(move |_, _, cx| ui.send(close.clone(), cx))
                    })
                    .test_support(),
            )
            .test_support()
            .into_any_element(),
    )
}

/// CD-5: what to do about a target that is gone: pick another session of
/// the channel, or have the session look after the whole channel.
fn target_fix(ui: &Rc<Ui>, state: &AppState, session: &Session, look: Look) -> AnyElement {
    let id = &session.id;
    let busy = state.desk.pending.contains(id);
    let fade = if busy { 0.5 } else { 1. };
    let open = state.desk.menu == Some(DeskMenu::Target(id.clone()));
    let toggle = AppAction::Desk(A::Menu((!open).then(|| DeskMenu::Target(id.clone()))));
    let clear = AppAction::Desk(A::SetTarget(id.clone(), None));
    let pick = {
        let ui = ui.clone();
        div()
            .id("channel-desk-bar-pick-target")
            .role(Role::Button)
            .aria_label("Pick a new target")
            .flex_shrink_0()
            .opacity(fade)
            .text_color(rgb(look.accent))
            .child("Pick target")
            .when(!busy, |x| {
                x.cursor_pointer()
                    .on_click(move |_, _, cx| ui.send(toggle.clone(), cx))
            })
            .test_support()
    };
    let picker = div()
        .relative()
        .flex_shrink_0()
        .child(pick)
        .when(open, |wrap| {
            wrap.child(deferred(crate::desk::scrim(ui, "channel-desk-bar-target-scrim")))
                .child(deferred(target_menu(ui, state, id, look)))
        });
    let whole_channel = {
        let ui = ui.clone();
        div()
            .id("channel-desk-bar-clear-target")
            .role(Role::Button)
            .aria_label("Look after the whole channel")
            .flex_shrink_0()
            .opacity(fade)
            .text_color(rgb(look.accent))
            .child("Look after the channel")
            .when(!busy, |x| {
                x.cursor_pointer()
                    .on_click(move |_, _, cx| ui.send(clear.clone(), cx))
            })
            .test_support()
    };
    row()
        .gap(px(10.))
        .flex_shrink_0()
        .child(picker)
        .child(whole_channel)
        .into_any_element()
}

/// The channel's other sessions a Desk session can look after, hanging under
/// Pick target.
fn target_menu(ui: &Rc<Ui>, state: &AppState, id: &SessionId, look: Look) -> impl IntoElement {
    let choices = state.desk.target_choices(id);
    col()
        .id("channel-desk-bar-target-menu")
        .role(Role::ListBox)
        .aria_label("Pick a new target")
        .absolute()
        .top(px(26.))
        .left(px(0.))
        .w(px(260.))
        .max_h(px(240.))
        .overflow_y_scroll()
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .when(choices.is_empty(), |list| {
            list.child(
                div()
                    .px(px(8.))
                    .py(px(5.))
                    .text_color(rgb(look.tertiary))
                    .child("This channel has no other session."),
            )
        })
        .children(choices.into_iter().map(|target| {
            let ui = ui.clone();
            let action = AppAction::Desk(A::SetTarget(id.clone(), Some(target.id.clone())));
            row()
                .id(SharedString::from(format!(
                    "channel-desk-bar-target-{}",
                    target.id.as_str()
                )))
                .role(Role::ListBoxOption)
                .aria_label(target.title.clone())
                .gap(px(8.))
                .h(px(26.))
                .px(px(8.))
                .rounded(px(5.))
                .cursor_pointer()
                .hover(move |x| x.bg(rgb(look.fill)))
                .child(chrome::status(target.status))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_nowrap()
                        .child(target.title.clone()),
                )
                .on_click(move |_, _, cx| ui.send(action.clone(), cx))
                .test_support()
        }))
        .test_support()
}

/// The hairline between the bar's parts.
fn divider(look: Look) -> Div {
    div()
        .w(px(1.))
        .h(px(16.))
        .flex_shrink_0()
        .bg(rgb(look.line))
}

/// Whether the folder is kept (frame 2, DV-4, D734): "Goes 7 days after
/// close" for a scratch folder nobody kept, "Kept" for one that is, nothing
/// for a folder the person picked.
fn kept_chip(look: Look, folder: &Folder, versions: u32) -> Option<ObservedElement<Stateful<Div>>> {
    let text = match (folder, versions) {
        (Folder::Scratch { kept: false, .. }, _) => "Goes 7 days after close".to_owned(),
        (Folder::Scratch { kept: true, .. }, 0) => "Kept".to_owned(),
        (Folder::Scratch { kept: true, .. }, 1) => "Kept · 1 version".to_owned(),
        (Folder::Scratch { kept: true, .. }, n) => format!("Kept · {n} versions"),
        (Folder::Picked { .. }, _) => return None,
    };
    Some(
        div()
            .id("desk-bar-kept-chip")
            .role(Role::Status)
            .aria_label(text.clone())
            .px(px(6.))
            .py(px(1.))
            .rounded(px(4.))
            .border_1()
            .border_color(rgb(look.line))
            .text_size(px(11.))
            .text_color(rgb(look.secondary))
            // The chip keeps its whole text (frame 37); the bar's free space
            // sits between Add repo and Keep folder.
            .flex_shrink_0()
            .whitespace_nowrap()
            .child(text)
            .test_support(),
    )
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
pub fn bar(
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
            // Deferred, so both paint over the tabs and terminal under the
            // bar and take the clicks there; the scrim first, the menu over it.
            wrap.child(deferred(crate::desk::scrim(ui, "desk-bar-machine-scrim")))
                .child(deferred(machine_menu(ui, state, session, look)))
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

    let copy_path = div()
        .id("desk-bar-copy-path")
        .role(Role::Button)
        .aria_label("Copy path")
        .flex_shrink_0()
        .text_color(rgb(look.accent))
        .cursor_pointer()
        .child("Copy path")
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
        })
        .test_support();

    Some(
        row()
            .id("desk-bar")
            .role(Role::Toolbar)
            .aria_label("Desk folder")
            .gap_x(px(10.))
            .gap_y(px(4.))
            .min_h(px(34.))
            .px(px(14.))
            .py(px(6.))
            .flex_wrap()
            .flex_shrink_0()
            .bg(rgb(look.content))
            .border_b_1()
            .border_color(rgb(look.line))
            .text_size(px(12.))
            .child(chrome::icon(IconName::Folder, look.secondary, 12.))
            .child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .child("Desk folder")
                    .test_text("Desk folder".to_owned()),
            )
            .child(copy_path)
            // Frame 2 (DV-4): whether the folder is kept; a picked folder has
            // no chip. A kept folder adds its count of versions.
            .children(kept_chip(look, &record.folder, record.version_count))
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
