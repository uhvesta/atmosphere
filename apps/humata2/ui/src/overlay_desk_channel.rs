//! A channel's Desk dialogs (CD-4, CD-6, frames 92 and 94): New desk session
//! with its switches, and the confirmation before Close session.

use super::super::sync;
use super::{Overlays, button, card, form_row, header, popup, primary, runs, secondary};
use crate::bridge::Ui;
use crate::chrome;
use crate::desk_info::{blocked_line, switch_row};
use crate::theme::Look;
use crate::widgets::{self, col, row};
use gpui_kit::assets::IconName;
use gpui_kit::base::FocusTrapElement as _;
use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::Input;
use gpui_kit::{prelude::*, *};
use humata2_app::AppAction;
use humata2_clients::desk::Switch;
use humata2_clients::model::{Agent, MachineId, SessionId};
use humata2_desk::{ChannelCloseSheet, ChannelForm, ChannelNewAction, DeskAction, DeskState};
use std::rc::Rc;

fn new_action(action: ChannelNewAction) -> AppAction {
    AppAction::Desk(DeskAction::ChannelNew(action))
}

/// What the New desk session dialog draws, read from the state once.
pub(super) struct NewView {
    form: ChannelForm,
    channel_name: String,
    blocked: Vec<Switch>,
    members: Vec<(SessionId, String)>,
    machines: Vec<(MachineId, String)>,
}

pub(super) fn new_view(desk: &DeskState) -> Option<NewView> {
    let form = desk.channel_new.clone()?;
    let channel_name = desk
        .workspace
        .channel(&form.channel)
        .map_or_else(|| form.channel.to_string(), |found| found.name.clone());
    let blocked = desk.blocked_for(&form.channel);
    let members = desk
        .channel_members(&form.channel)
        .into_iter()
        .map(|session| (session.id.clone(), session.title.clone()))
        .collect();
    let machines = desk
        .workspace
        .machines
        .iter()
        .filter(|machine| machine.online)
        .map(|machine| (machine.id.clone(), machine.name.clone()))
        .collect();
    Some(NewView {
        form,
        channel_name,
        blocked,
        members,
        machines,
    })
}

/// What the close confirmation draws.
pub(super) struct CloseView {
    sheet: ChannelCloseSheet,
    title: String,
    files: Vec<String>,
    lines: std::collections::BTreeMap<String, u32>,
    /// The folder holds a version, so Close keeps it (DV-13).
    kept: bool,
}

pub(super) fn close_view(desk: &DeskState) -> Option<CloseView> {
    let sheet = desk.channel_close.clone()?;
    let title = desk
        .workspace
        .session(&sheet.session)
        .map_or_else(|| sheet.session.to_string(), |found| found.title.clone());
    let record = desk.channel_record(&sheet.session);
    let files = record
        .map(|found| found.documents.clone())
        .unwrap_or_default();
    let lines = record
        .map(|found| found.document_lines.clone())
        .unwrap_or_default();
    let kept = record.is_some_and(|found| found.desk.version_count > 0);
    Some(CloseView {
        sheet,
        title,
        files,
        lines,
        kept,
    })
}

/// "54 lines", as a Markdown file's size reads in the confirmation.
fn lines_label(lines: u32) -> String {
    if lines == 1 {
        "1 line".to_owned()
    } else {
        format!("{lines} lines")
    }
}

/// The element id of a switch in the New desk session dialog.
const fn switch_id(switch: Switch) -> &'static str {
    match switch {
        Switch::ReadFiles => "new-channel-desk-read-files",
        Switch::SendNotes => "new-channel-desk-send-notes",
        Switch::TypeIntoTerminal => "new-channel-desk-type-into-terminal",
        Switch::StartAndFork => "new-channel-desk-start-and-fork",
        Switch::StopAndArchive => "new-channel-desk-stop-and-archive",
        Switch::ChangeBoardItems => "new-channel-desk-change-board-items",
    }
}

/// One choice of a pair: "This channel" or "A session".
fn segment(
    ui: &Rc<Ui>,
    look: Look,
    id: &'static str,
    text: &'static str,
    on: bool,
    action: AppAction,
) -> impl IntoElement {
    let ui = ui.clone();
    div()
        .id(id)
        .role(Role::Tab)
        .aria_label(text)
        .aria_selected(on)
        .test_support()
        .px(px(12.))
        .py(px(2.))
        .text_size(px(11.5))
        .rounded(px(5.))
        .when(on, |x| x.bg(rgb(look.card)).shadow_sm())
        .cursor_pointer()
        .child(text)
        .on_click(move |_, _, cx| ui.send(action.clone(), cx))
}

impl Overlays {
    /// New desk session on a channel's Desk (CD-6).
    pub(super) fn desk_channel_new(
        &self,
        view: &NewView,
        look: Look,
        ui: &Rc<Ui>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let form = &view.form;
        sync(&self.desk.title, &form.title, window, cx);
        let busy = form.starting;
        // AGENTSET (D725): the enabled agents, then the plain shell.
        let offered: Vec<Agent> = ui
            .read(|state| state.settings.offered_agents())
            .into_iter()
            .chain([Agent::Shell])
            .collect();

        let title = div().h(px(26.)).child(
            Input::new(&self.desk.title)
                .small()
                .min_h(px(26.))
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
                        "new-channel-desk-agent-{}",
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
                    .on_click(move |_, _, cx| {
                        ui.send(new_action(ChannelNewAction::Agent(agent)), cx);
                    })
            }));

        let scope = row()
            .flex_shrink_0()
            .p(px(2.))
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .child(segment(
                ui,
                look,
                "new-channel-desk-whole",
                "This channel",
                !form.on_session,
                new_action(ChannelNewAction::OnSession(false)),
            ))
            .child(segment(
                ui,
                look,
                "new-channel-desk-session-scope",
                "A session",
                form.on_session,
                new_action(ChannelNewAction::OnSession(true)),
            ));
        let picked = form
            .session
            .as_ref()
            .and_then(|id| view.members.iter().find(|(member, _)| member == id))
            .map_or_else(
                || {
                    if view.members.is_empty() {
                        "No sessions yet".to_owned()
                    } else {
                        "Pick a session".to_owned()
                    }
                },
                |(_, title)| title.clone(),
            );
        let mut target = col().gap(px(6.)).child(row().child(scope));
        if form.on_session {
            target = target.child(row().child(popup(
                ui,
                look,
                "new-channel-desk-session",
                picked.into(),
                new_action(ChannelNewAction::Sessions(!form.sessions_open)),
            )));
            if form.sessions_open {
                target = target.child(row().flex_wrap().gap(px(6.)).children(
                    view.members.iter().map(|(id, title)| {
                        button(
                            ui,
                            look,
                            SharedString::from(format!("new-channel-desk-session-{}", id.as_str())),
                            title.clone(),
                            new_action(ChannelNewAction::Session(id.clone())),
                        )
                    }),
                ));
            }
            target = target.child(secondary(
                look,
                "Covers that session and its forks, including forks made later.",
            ));
        }

        let machine = form
            .machine
            .as_ref()
            .and_then(|id| view.machines.iter().find(|(machine, _)| machine == id))
            .map_or("Choose a machine", |(_, name)| name.as_str())
            .to_owned();
        let mut machine_value = col().gap(px(6.)).child(
            row().gap(px(8.)).child(popup(
                ui,
                look,
                "new-channel-desk-machine",
                machine.into(),
                new_action(ChannelNewAction::Machines(!form.machines_open)),
            )),
        );
        if form.machines_open {
            machine_value = machine_value.child(row().flex_wrap().gap(px(6.)).children(
                view.machines.iter().map(|(id, name)| {
                    button(
                        ui,
                        look,
                        SharedString::from(format!("new-channel-desk-machine-{}", id.as_str())),
                        name.clone(),
                        new_action(ChannelNewAction::Machine(id.clone())),
                    )
                }),
            ));
        }

        let switches = col()
            .gap(px(8.))
            .children(Switch::ALL.into_iter().map(|switch| {
                let on = form.switches.get(switch);
                col()
                    .gap(px(2.))
                    .child(switch_row(
                        ui,
                        look,
                        switch_id(switch).into(),
                        switch.label(),
                        on,
                        !busy,
                        view.blocked.contains(&switch),
                        new_action(ChannelNewAction::Switch(switch, !on)),
                    ))
                    .children(view.blocked.contains(&switch).then(|| {
                        blocked_line(
                            ui,
                            look,
                            format!("{}-rule", switch_id(switch)).into(),
                            &form.channel,
                        )
                    }))
            }));

        card("new-channel-desk", "New desk session", 600., look)
            .gap(px(12.))
            .child(header(
                look,
                "New desk session",
                format!("in # {}", view.channel_name),
            ))
            .child(form_row(look, "Title", title))
            .child(form_row(look, "Runs", row().child(agents)))
            .child(form_row(look, "Target", target))
            .child(form_row(look, "Machine", machine_value))
            .child(form_row(look, "May do", switches))
            .child(secondary(
                look,
                "Anything left off follows the channel's rules. You can change these in its Info panel.",
            ))
            .children(form.error.clone().map(|error| {
                widgets::error_banner_in(look, ui, "new-channel-desk-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(div().flex_1())
                    .child(button(
                        ui,
                        look,
                        "new-channel-desk-cancel",
                        "Cancel",
                        AppAction::Escape,
                    ))
                    .child(primary(
                        ui,
                        look,
                        "new-channel-desk-start",
                        if busy { "Starting…" } else { "Start" },
                        new_action(ChannelNewAction::Start),
                        busy,
                    )),
            )
            .focus_trap("new-channel-desk", &self.desk.title.focus_handle(cx))
            .into_any_element()
    }

    /// Close session on a channel Desk session asks first (CD-4): a folder
    /// with no version goes to the trash now, and its Markdown files can be
    /// copied out; one that holds a version stays under Closed (DV-13).
    pub(super) fn desk_channel_close(
        &self,
        view: &CloseView,
        look: Look,
        ui: &Rc<Ui>,
    ) -> AnyElement {
        let busy = view.sheet.closing;
        let files = (!view.files.is_empty()).then(|| {
            col()
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(look.secondary))
                        .child(format!("Markdown files in it · {}", view.files.len())),
                )
                .children(view.files.iter().map(|name| {
                    let copied = view.sheet.copied.contains(name);
                    row()
                        .gap(px(10.))
                        .py(px(7.))
                        .border_b_1()
                        .border_color(rgb(look.line))
                        .child(chrome::icon(IconName::FileText, look.secondary, 14.))
                        .child(
                            row()
                                .flex_1()
                                .gap(px(8.))
                                .child(
                                    div()
                                        .font_family("Menlo")
                                        .text_size(px(12.))
                                        .child(name.clone()),
                                )
                                .children(
                                    view.lines
                                        .get(name)
                                        .map(|lines| secondary(look, lines_label(*lines))),
                                ),
                        )
                        .child(button(
                            ui,
                            look,
                            SharedString::from(format!("channel-desk-copy-{name}")),
                            if copied { "Copied" } else { "Copy to shared/" },
                            AppAction::Desk(DeskAction::CopyToShared(name.clone())),
                        ))
                }))
        });
        card("channel-desk-close", "Close session", 560., look)
            .gap(px(12.))
            .child(
                col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("Close \u{201c}{}\u{201d}?", view.title)),
                    )
                    // DV-13: a folder that holds a version is kept.
                    .child(if view.kept {
                        div()
                            .id("channel-desk-close-kept")
                            .role(Role::Status)
                            .aria_label("Folder kept")
                            .test_support()
                            .child("Its folder holds a version, so it stays on the channel's Desk under Closed, where you can reopen it.")
                    } else {
                        div()
                            .id("channel-desk-close-trash")
                            .role(Role::Status)
                            .aria_label("Folder goes to the trash")
                            .test_support()
                            .child("Its scratch folder goes to the trash now.")
                    }),
            )
            .children(files)
            .children((!view.kept).then(|| {
                secondary(
                    look,
                    "Its record and conversation stay in the trash for 30 days. You can restore them from there.",
                )
            }))
            .children(view.sheet.error.clone().map(|error| {
                widgets::error_banner_in(look, ui, "channel-desk-close-error", error, AppAction::Escape)
            }))
            .child(
                row()
                    .gap(px(8.))
                    .pt(px(4.))
                    .child(div().flex_1())
                    .child(button(
                        ui,
                        look,
                        "channel-desk-close-cancel",
                        "Cancel",
                        AppAction::Escape,
                    ))
                    .child(primary(
                        ui,
                        look,
                        "channel-desk-close-confirm",
                        if busy { "Closing…" } else { "Close session" },
                        AppAction::Desk(DeskAction::ConfirmCloseChannel),
                        busy,
                    )),
            )
            .focus_trap("channel-desk-close", &self.sheet)
            .into_any_element()
    }
}
