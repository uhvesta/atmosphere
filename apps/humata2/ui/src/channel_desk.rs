//! A channel's Desk tab (CD-2, CD-6, frame 91): the open Desk sessions that
//! look after the channel or one of its sessions, under New desk session.

use crate::{
    bridge::Ui,
    chrome,
    desk::runs,
    theme::{Look, ORANGE},
    widgets::{self, col, row},
};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState};
use humata2_clients::model::{Channel, Status};
use humata2_desk::{ChannelRow, DeskAction};
use std::rc::Rc;

fn dim(look: Look, text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}

fn words(status: Status) -> &'static str {
    match status {
        Status::Working => "Working",
        Status::NeedsYou => "Needs you",
        Status::Idle | Status::ProbablyIdle => "Idle",
        Status::Exited { .. } => "Exited",
        Status::Offline => "Offline",
        Status::Stopped | Status::AgentStopped => "Stopped",
        Status::Moving => "Moving",
    }
}

fn head(look: Look, text: &'static str) -> Div {
    div()
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.secondary))
        .child(text)
}

/// One Desk session: its line of the table, and under it the place a
/// document-history strip can be drawn (DV-5).
fn session_row(ui: &Rc<Ui>, state: &AppState, item: &ChannelRow<'_>, look: Look) -> AnyElement {
    let session = item.session;
    let target = match &item.desk.target {
        None => "This channel".to_owned(),
        Some(target) => format!(
            "Session: {}",
            state
                .workspace
                .session(target)
                .map_or_else(|| target.to_string(), |found| found.title.clone())
        ),
    };
    let machine = state.workspace.machine(&session.machine).map_or_else(
        || session.machine.to_string(),
        |machine| machine.name.clone(),
    );
    let open = AppAction::Desk(DeskAction::Open(session.id.clone()));
    // CD-5: the target went while the folder held Markdown files, so the
    // session stopped; the row says so, and Close is on its page.
    let gone = item.desk.target_gone;
    let target_cell = div()
        .w(px(190.))
        .pr(px(12.))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis();
    let target_cell = if gone {
        target_cell
            .id(SharedString::from(format!(
                "channel-desk-target-gone-{}",
                session.id.as_str()
            )))
            .role(Role::Status)
            .aria_label("Target gone")
            .text_color(rgb(ORANGE))
            .child("Target gone")
            .test_support()
            .into_any_element()
    } else {
        target_cell.child(target).into_any_element()
    };
    let ui = ui.clone();
    let line = row()
        .id(SharedString::from(format!(
            "channel-desk-row-{}",
            session.id.as_str()
        )))
        .role(Role::Button)
        .aria_label(session.title.clone())
        .gap(px(8.))
        .px(px(12.))
        .h(px(34.))
        .border_b_1()
        .border_color(rgb(look.line))
        .cursor_pointer()
        .child(
            row()
                .flex_1()
                .min_w_0()
                .gap(px(7.))
                .child(chrome::status(session.status))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .whitespace_nowrap()
                        .child(session.title.clone()),
                ),
        )
        .child(target_cell)
        .child(div().w(px(100.)).child(runs(session.agent)))
        .child(div().w(px(80.)).child(machine))
        .child(
            div()
                .w(px(80.))
                .text_color(rgb(look.secondary))
                .child(words(session.status)),
        )
        .child(
            div()
                .w(px(90.))
                .text_color(rgb(look.secondary))
                .child(item.desk.desk.last_used.label.clone()),
        )
        .on_click(move |_, _, cx| ui.send(open.clone(), cx))
        .test_support();
    col()
        .child(line)
        // DOCV: doc history strip slot
        .children(kept_line(look, session.id.as_str(), item.desk.desk.version_count))
        .into_any_element()
}

/// "Kept" and how many versions the folder holds (frame 2's chip text).
fn kept_text(versions: u32) -> String {
    match versions {
        0 => "Kept".to_owned(),
        1 => "Kept · 1 version".to_owned(),
        n => format!("Kept · {n} versions"),
    }
}

/// The caption under an open row whose folder holds versions (DV-13).
fn kept_line(look: Look, id: &str, versions: u32) -> Option<AnyElement> {
    if versions == 0 {
        return None;
    }
    let text = kept_text(versions);
    Some(
        div()
            .id(SharedString::from(format!("channel-desk-versions-{id}")))
            .role(Role::Status)
            .aria_label(text.clone())
            .px(px(12.))
            .pb(px(4.))
            .text_size(px(11.))
            .text_color(rgb(look.secondary))
            .child(text)
            .test_support()
            .into_any_element(),
    )
}

fn card(look: Look) -> Div {
    col()
        .w_full()
        .rounded(px(9.))
        .border_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.card))
        .overflow_hidden()
}

/// A titled group: its small heading over `body`.
fn group(look: Look, title: &'static str, body: Div) -> Div {
    col().gap(px(6.)).child(head(look, title)).child(body)
}

/// A closed kept Desk session: its title, how many versions it holds, when
/// it was last used and Reopen (DV-13).
fn closed_row(ui: &Rc<Ui>, item: &ChannelRow<'_>, look: Look) -> AnyElement {
    let session = item.session;
    let reopen = AppAction::Desk(DeskAction::Reopen(session.id.clone()));
    row()
        .id(SharedString::from(format!(
            "channel-desk-closed-{}",
            session.id.as_str()
        )))
        .role(Role::ListItem)
        .aria_label(session.title.clone())
        .gap(px(8.))
        .px(px(12.))
        .h(px(34.))
        .border_b_1()
        .border_color(rgb(look.line))
        .child(
            row()
                .flex_1()
                .min_w_0()
                .gap(px(7.))
                .child(chrome::icon(IconName::X, 0xff453a, 10.))
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .whitespace_nowrap()
                        .child(session.title.clone()),
                ),
        )
        .child(
            div()
                .w(px(130.))
                .text_color(rgb(look.secondary))
                .child(kept_text(item.desk.desk.version_count)),
        )
        .child(
            div()
                .w(px(110.))
                .text_color(rgb(look.secondary))
                .child(item.desk.desk.last_used.label.clone()),
        )
        .child(widgets::push_quiet(
            ui,
            SharedString::from(format!("channel-desk-reopen-{}", session.id.as_str())),
            "Reopen",
            reopen,
            look,
        ))
        .test_support()
        .into_any_element()
}

/// One Markdown file of a channel Desk session. Clicking a file of an open
/// session opens the session, as its row does; a closed session's file waits
/// for Reopen.
fn document_row(ui: &Rc<Ui>, name: &str, item: &ChannelRow<'_>, look: Look) -> AnyElement {
    let session = item.session;
    let open = item
        .desk
        .desk
        .closed
        .is_none()
        .then(|| AppAction::Desk(DeskAction::Open(session.id.clone())));
    let ui = ui.clone();
    row()
        .id(SharedString::from(format!(
            "channel-desk-document-{}-{name}",
            session.id.as_str()
        )))
        .role(Role::ListItem)
        .aria_label(name.to_owned())
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
                        .child(name.to_owned()),
                )
                .child(dim(look, session.title.clone())),
        )
        .on_click(move |_, _, cx| {
            if let Some(action) = &open {
                ui.send(action.clone(), cx);
            }
        })
        .test_support()
        .into_any_element()
}

/// The tab: New desk session, and the open sessions as a table.
pub(crate) fn tab(ui: &Rc<Ui>, state: &AppState, channel: &Channel, look: Look) -> AnyElement {
    let rows = state.desk.channel_rows(&channel.id);
    let agent = state.settings.preferences.default_agent;
    let new = widgets::push_primary(
        ui,
        "channel-desk-new",
        "New desk session",
        AppAction::Desk(DeskAction::StartChannelNew {
            channel: channel.id.clone(),
            target: None,
            agent,
        }),
        look,
    );
    let documents = state.desk.channel_documents(&channel.id);
    let closed = state.desk.channel_closed(&channel.id);
    let documents_group = (!documents.is_empty()).then(|| {
        group(
            look,
            "Documents",
            card(look).children(
                documents
                    .iter()
                    .map(|(name, item)| document_row(ui, name, item, look)),
            ),
        )
    });
    let closed_group = (!closed.is_empty()).then(|| {
        group(
            look,
            "Closed",
            card(look).children(closed.iter().map(|item| closed_row(ui, item, look))),
        )
    });
    let intro = row()
        .gap(px(10.))
        .child(dim(
            look,
            "Short-lived sessions that look after this channel or one of its sessions.",
        ))
        .child(div().flex_1())
        .child(new);
    let body = if rows.is_empty() {
        widgets::empty_in(
            look,
            "No desk sessions yet",
            "A desk session works in a scratch folder and has no clones of its own.",
        )
        .into_any_element()
    } else {
        col()
            .w_full()
            .rounded(px(9.))
            .border_1()
            .border_color(rgb(look.line))
            .bg(rgb(look.card))
            .overflow_hidden()
            .child(
                row()
                    .gap(px(8.))
                    .px(px(12.))
                    .h(px(30.))
                    .bg(rgb(look.panel))
                    .border_b_1()
                    .border_color(rgb(look.line))
                    .child(head(look, "Session").flex_1())
                    .child(head(look, "Target").w(px(190.)))
                    .child(head(look, "Agent").w(px(100.)))
                    .child(head(look, "Machine").w(px(80.)))
                    .child(head(look, "Status").w(px(80.)))
                    .child(head(look, "Last used ↓").w(px(90.))),
            )
            .children(rows.iter().map(|item| session_row(ui, state, item, look)))
            .into_any_element()
    };
    col()
        .id("channel-desk")
        .size_full()
        .min_h_0()
        .overflow_y_scroll()
        .p(px(20.))
        .gap(px(10.))
        .text_size(px(12.5))
        .child(intro)
        .child(body)
        .children(documents_group)
        .children(closed_group)
        .child(dim(
            look,
            "A desk session works in a scratch folder and has no clones of its own. Closing it sends the folder to the trash, unless it holds a saved version.",
        ))
        .into_any_element()
}
