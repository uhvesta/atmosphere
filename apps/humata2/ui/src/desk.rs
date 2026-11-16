//! The Desk page (DK-5, DK-11, DK-13, frame 36): the open sessions that
//! belong to no channel, the documents in their folders and the closed
//! sessions, under a header with Sort, Clean up… and New desk session.
use crate::{
    bridge::Ui,
    chrome,
    theme::Look,
    widgets::{self, col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, TestTextExt as _};
use gpui_kit::component::button::Button;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::desk::{DeskSession, Document, Folder};
use humata2_clients::model::{Agent, Session};
use humata2_desk::{DeskAction as A, Menu as DeskMenu, Sort};
use std::rc::Rc;

/// The frames' quiet text.
fn dim(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}

fn card(look: Look) -> Div {
    col()
        .w_full()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
}

/// A titled group: its small heading over `body`.
fn group(look: Look, title: impl Into<SharedString>, body: Div) -> Div {
    col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(look.secondary))
                .child(title.into()),
        )
        .child(body)
}

/// A repo's chip on a session's row.
fn chip(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(11.))
        .text_color(rgb(look.secondary))
        .flex_shrink_0()
        .child(text.into())
}

/// The frames' 24 pt button: filled with the accent, or grey with a hairline.
fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    action: AppAction,
    look: Look,
    primary: bool,
) -> Button {
    // The shared push buttons draw their label at the frames' 12 pt; the
    // kit's own label is smaller, which made these narrower than frame 36.
    if primary {
        widgets::push_primary(ui, id, label, action, look)
    } else {
        widgets::push_quiet(ui, id, label, action, look)
    }
}

/// What a Desk session runs, as the frames and dialogs name it.
pub(crate) fn runs(agent: Agent) -> &'static str {
    match agent {
        Agent::Claude => "Claude Code",
        Agent::Shell => "Terminal",
        other => other.name(),
    }
}

/// "<what it runs> · <machine name>".
fn runs_on(state: &AppState, session: &Session) -> String {
    let machine = state.workspace.machine(&session.machine).map_or_else(
        || session.machine.to_string(),
        |machine| machine.name.clone(),
    );
    format!("{} · {machine}", runs(session.agent))
}

/// An open session's folder, in words.
fn folder_text(folder: &Folder) -> String {
    match folder {
        Folder::Scratch { kept: false, .. } => "scratch folder".to_owned(),
        Folder::Scratch { kept: true, .. } => "scratch folder · kept".to_owned(),
        Folder::Picked { path } => path.clone(),
    }
}

/// What becomes of a closed session's folder (DK-11), checked in order: the
/// days left before it is deleted; a kept scratch folder; a picked folder's
/// path; otherwise the unpushed work that holds the folder.
fn closed_folder_text(desk: &DeskSession) -> String {
    match desk.closed.as_ref().and_then(|closed| closed.days_left) {
        Some(0) => return "deleted today".to_owned(),
        Some(1) => return "deleted in 1 day".to_owned(),
        Some(days) => return format!("deleted in {days} days"),
        None => {}
    }
    match (&desk.folder, &desk.unpushed) {
        (Folder::Scratch { kept: true, .. } | Folder::Picked { .. }, _) => {
            folder_text(&desk.folder)
        }
        (Folder::Scratch { kept: false, .. }, Some(work)) => format!("not deleted · {work}"),
        (Folder::Scratch { kept: false, .. }, None) => folder_text(&desk.folder),
    }
}

/// What every session row shares: its mark, the title over the line about
/// it, and "last used" over when. The caller gives it an id and its actions.
fn session_row(
    look: Look,
    mark: Div,
    title: &str,
    runs: String,
    folder: String,
    repos: &[String],
    used: &str,
) -> Div {
    row()
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(mark)
        .child(
            col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.to_owned()),
                )
                .child(
                    row()
                        .gap(px(6.))
                        .child(dim(look, runs))
                        .child(dim(look, "·"))
                        .child(chrome::icon(IconName::Folder, look.secondary, 12.))
                        .child(dim(look, folder.clone()).test_text(folder))
                        .children(repos.iter().map(|repo| chip(look, repo.clone()))),
                ),
        )
        .child(
            col()
                .w(px(110.))
                .flex_shrink_0()
                .items_end()
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(look.tertiary))
                        .child("last used"),
                )
                .child(dim(look, used.to_owned()).test_text(used.to_owned())),
        )
}

fn open_row(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    desk: &DeskSession,
    look: Look,
) -> impl IntoElement {
    let ui = ui.clone();
    let id = session.id.clone();
    session_row(
        look,
        chrome::status(session.status),
        &session.title,
        runs_on(state, session),
        folder_text(&desk.folder),
        &desk.repos,
        &desk.last_used.label,
    )
    .id(SharedString::from(format!("desk-open-{}", session.id)))
    .role(Role::Link)
    .aria_label(session.title.clone())
    .cursor_pointer()
    .on_click(move |_, _, cx| ui.send(AppAction::Desk(A::Open(id.clone())), cx))
    .test_support()
}

fn closed_row(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    desk: &DeskSession,
    look: Look,
) -> impl IntoElement {
    let pending = state.desk.pending.contains(&session.id);
    let expires = desk
        .closed
        .as_ref()
        .is_some_and(|closed| closed.days_left.is_some());
    let mark = div()
        .size(px(10.))
        .flex_shrink_0()
        .child(chrome::icon(IconName::X, 0xff453a, 10.));
    // A slot as wide as Keep (52 pt), the gap (6 pt) and Reopen (66 pt), so a
    // row without Keep leaves its space empty and the columns line up.
    let actions = row()
        .gap(px(6.))
        .w(px(124.))
        .justify_end()
        .flex_shrink_0()
        .children(expires.then(|| {
            button(
                ui,
                SharedString::from(format!("desk-keep-{}", session.id)),
                "Keep",
                AppAction::Desk(A::Keep(session.id.clone())),
                look,
                false,
            )
            .disabled(pending)
        }))
        .child(
            button(
                ui,
                SharedString::from(format!("desk-reopen-{}", session.id)),
                "Reopen",
                AppAction::Desk(A::Reopen(session.id.clone())),
                look,
                false,
            )
            .disabled(pending),
        );
    session_row(
        look,
        mark,
        &session.title,
        runs_on(state, session),
        closed_folder_text(desk),
        &desk.repos,
        &desk.last_used.label,
    )
    .id(SharedString::from(format!("desk-closed-{}", session.id)))
    .role(Role::ListItem)
    .aria_label(session.title.clone())
    .child(actions)
    .test_support()
}

fn document_row(
    ui: &Rc<Ui>,
    document: &Document,
    session: &Session,
    look: Look,
) -> impl IntoElement {
    let key = format!("{}-{}", document.session, document.path);
    row()
        .id(SharedString::from(format!("desk-document-{key}")))
        .role(Role::ListItem)
        .aria_label(document.path.clone())
        .gap(px(10.))
        .px(px(12.))
        .py(px(9.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(chrome::icon(IconName::FileText, look.secondary, 12.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .gap(px(8.))
                .child(
                    div()
                        .font_family("Menlo")
                        .text_size(px(12.))
                        .child(document.path.clone()),
                )
                .child(dim(look, session.title.clone())),
        )
        .child(button(
            ui,
            SharedString::from(format!("desk-copy-{key}")),
            "Copy to a channel…",
            AppAction::Desk(A::StartCopy(document.clone())),
            look,
            false,
        ))
        .test_support()
}

/// A transparent layer over the window behind an open pop-up: a click on
/// it closes the pop-up and reaches nothing else, so the button that opened
/// it does not reopen it. It overflows its small parent on purpose, and is
/// drawn deferred so nothing on the page paints over it.
pub(crate) fn scrim(ui: &Rc<Ui>, id: &'static str) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .absolute()
        .top(px(-4000.))
        .left(px(-4000.))
        .w(px(8000.))
        .h(px(8000.))
        .occlude()
        .on_click(move |_, _, cx| ui.send(AppAction::Desk(A::Menu(None)), cx))
}

/// The two orders under the Sort button, the current one ticked.
fn sort_menu(ui: &Rc<Ui>, current: Sort, look: Look) -> impl IntoElement {
    col()
        .id("desk-sort-menu")
        .role(Role::ListBox)
        .aria_label("Sort")
        .absolute()
        .top(px(28.))
        .right(px(0.))
        .w(px(140.))
        .p(px(4.))
        .gap(px(1.))
        .rounded(px(8.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(look.line))
        .shadow_lg()
        .occlude()
        .text_size(px(12.5))
        .children(
            [
                ("desk-sort-last-used", "Last used", Sort::LastUsed),
                ("desk-sort-title", "Title", Sort::Title),
            ]
            .into_iter()
            .map(|(id, name, sort)| {
                let ui = ui.clone();
                row()
                    .id(id)
                    .role(Role::ListBoxOption)
                    .aria_label(name)
                    .aria_selected(sort == current)
                    .gap(px(8.))
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(move |x| x.bg(rgb(look.fill)))
                    .child(
                        div().w(px(12.)).flex_shrink_0().children(
                            (sort == current)
                                .then(|| chrome::icon(IconName::Check, look.accent, 12.)),
                        ),
                    )
                    .child(name)
                    .on_click(move |_, _, cx| ui.send(AppAction::Desk(A::SetSort(sort)), cx))
                    .test_support()
            }),
        )
        .test_support()
}

/// The Sort pop-up (SE-26): its button says the order, and while it is
/// open the two orders hang under it, drawn over the page.
fn sort(ui: &Rc<Ui>, state: &AppState, look: Look) -> Div {
    let open = state.desk.menu == Some(DeskMenu::Sort);
    let label = match state.desk.sort {
        Sort::LastUsed => "Sort: last used",
        Sort::Title => "Sort: title",
    };
    let toggle = AppAction::Desk(A::Menu((!open).then_some(DeskMenu::Sort)));
    let send = ui.clone();
    let popup = row()
        .id("desk-sort")
        .role(Role::Button)
        .aria_label(label)
        .flex_shrink_0()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(9.))
        .pr(px(5.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .border_1()
        .border_color(rgb(look.line))
        .text_size(px(12.))
        .cursor_pointer()
        .child(label)
        .child(chrome::icon(IconName::ChevronsUpDown, look.secondary, 12.))
        .on_click(move |_, _, cx| send.send(toggle.clone(), cx))
        .test_support();
    div()
        .relative()
        .flex_shrink_0()
        .child(popup)
        .when(open, |wrap| {
            // Deferred, so both paint over the page under the toolbar and
            // take the clicks there; the scrim first, the menu over it.
            wrap.child(deferred(scrim(ui, "desk-sort-scrim")))
                .child(deferred(sort_menu(ui, state.desk.sort, look)))
        })
}

/// The Desk page's header: its title, then Sort, Clean up… and New desk
/// session.
pub(crate) fn toolbar(ui: &Rc<Ui>, state: &AppState, hidden: bool, look: Look) -> Div {
    row()
        .h(px(52.))
        .px(px(20.))
        .gap(px(10.))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        // With the sidebar hidden the window buttons sit over this bar.
        .when(hidden, |bar| bar.child(crate::chrome::lights()))
        .child(
            div()
                .id("screen-title")
                .role(Role::Heading)
                .aria_label("Desk")
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Desk")
                .test_support(),
        )
        .child(dim(look, "Sessions outside any channel"))
        .child(div().flex_1())
        .child(sort(ui, state, look))
        .child(button(
            ui,
            "desk-clean-up",
            "Clean up…",
            AppAction::Desk(A::OpenCleanUp),
            look,
            false,
        ))
        .child(button(
            ui,
            "desk-new",
            "New desk session",
            AppAction::StartNewDeskSession,
            look,
            true,
        ))
}

/// The Desk page: Open, Documents and Closed in one centred column. A
/// group with nothing in it is left out, except Open, which then says what
/// a Desk session is and offers to start one.
pub(crate) fn page(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    let open = state.desk.open();
    let documents = state.desk.documents();
    let closed = state.desk.closed();
    let (open_title, open_body) = if open.is_empty() {
        (
            "Open".to_owned(),
            card(look).child(
                row()
                    .gap(px(12.))
                    .px(px(14.))
                    .py(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_color(rgb(look.tertiary))
                            .child(
                                "No desk session is open. Start an agent or a shell in a scratch folder, no channel needed.",
                            ),
                    )
                    .child(button(
                        ui,
                        "desk-new-first",
                        "New desk session",
                        AppAction::StartNewDeskSession,
                        look,
                        true,
                    )),
            ),
        )
    } else {
        (
            format!("Open · {}", open.len()),
            card(look).overflow_hidden().children(
                open.iter()
                    .map(|entry| open_row(ui, state, entry.session, entry.desk, look)),
            ),
        )
    };
    let mut groups = col()
        .w_full()
        .max_w(px(760.))
        .gap(px(20.))
        .children(state.desk.error.clone().map(|error| {
            widgets::error_banner(ui, "desk-error", error, AppAction::Desk(A::DismissError))
        }))
        .child(group(look, open_title, open_body));
    if !documents.is_empty() {
        groups = groups.child(group(
            look,
            "Documents",
            card(look).overflow_hidden().children(
                documents
                    .iter()
                    .map(|&(document, session)| document_row(ui, document, session, look)),
            ),
        ));
    }
    if !closed.is_empty() {
        groups = groups.child(group(
            look,
            "Closed · scratch folders go after 7 days",
            card(look).overflow_hidden().children(
                closed
                    .iter()
                    .map(|entry| closed_row(ui, state, entry.session, entry.desk, look)),
            ),
        ));
    }
    col()
        .id("desk")
        .size_full()
        .overflow_y_scroll()
        .p(px(24.))
        .items_center()
        .text_size(px(13.))
        .child(groups)
}
