//! The machine page's running holders (D784, frame 59's "Sessions on it"):
//! one row per terminal of ours on the machine, sessions first (running,
//! then stopped) and the ones no channel has under "Not in a channel", each
//! with agent, folder, uptime, pid and a ⋯ menu.

use crate::bridge::Ui;
use crate::chrome;
use crate::theme::{Look, RED_STATUS};
use crate::widgets::{col, row};
use humata2_ui_core::clock::{Now, ticked_now, until_next};
use gpui_kit::assets::IconName;
use gpui_kit::{prelude::*, *};
use humata2_clients::RunningHolder;
use humata2_clients::model::{Agent, ChannelId, Session, Status};
use humata2_hosts::RunningAction;
use humata2_hosts::RowAction;
use humata2_hosts::page::{MachinePageAction, MachinePageState};
use std::rc::Rc;

/// How a row's ⋯ menu words an action.
pub const fn action_label(action: RowAction) -> &'static str {
    match action {
        RowAction::Open => "Open",
        RowAction::Stop => "Stop",
        RowAction::EndForGood => "End for good\u{2026}",
        RowAction::Resume => "Resume",
        RowAction::Archive => "Archive\u{2026}",
        RowAction::Adopt => "Adopt\u{2026}",
    }
}

/// Whether a menu draws a separator before `action`: running rows set End
/// for good apart, stopped rows Archive, and End for good.
pub const fn separated(action: RowAction) -> bool {
    matches!(action, RowAction::EndForGood | RowAction::Archive)
}

pub use crate::chrome::agent_label;

/// How long a holder has run: "under a minute", "12 min", "3 h 5 min",
/// "2 d 4 h".
pub fn uptime(now_ms: i64, started_ms: i64) -> String {
    let minutes = (now_ms - started_ms).max(0) / 60_000;
    let (hours, minutes) = (minutes / 60, minutes % 60);
    let (days, hours) = (hours / 24, hours % 24);
    match (days, hours, minutes) {
        (0, 0, 0) => "under a minute".to_owned(),
        (0, 0, m) => format!("{m} min"),
        (0, h, 0) => format!("{h} h"),
        (0, h, m) => format!("{h} h {m} min"),
        (d, 0, _) => format!("{d} d"),
        (d, h, _) => format!("{d} d {h} h"),
    }
}

/// One row's ⋯ menu: its open flag is the menu's own, so it needs no state
/// of the app's.
#[derive(IntoElement)]
struct RowMenu {
    ui: Rc<Ui>,
    id: SharedString,
    holder: RunningHolder,
    actions: Vec<RowAction>,
    wrap: super::machine_page::Wrap,
    look: Look,
}

impl RenderOnce for RowMenu {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let open = window.use_keyed_state(self.id.clone(), cx, |_, _| false);
        let flip_up = window.use_keyed_state(
            SharedString::from(format!("{}-flip", self.id)),
            cx,
            |_, _| false,
        );
        let shown = *open.read(cx);
        let look = self.look;
        let toggle = open.clone();
        let set_flip = flip_up.clone();
        let est_h = self.actions.len() as f32 * 28. + 20.;
        let mut host = div().relative().flex_shrink_0().child(
            div()
                .id(SharedString::from(format!("{}-button", self.id)))
                .role(Role::Button)
                // Each row's ⋯ is named for its row, apart from the page's own
                // "More" (D784).
                .aria_label(format!("More for {}", row_name(&self.holder)))
                .size(px(15.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(chrome::icon(IconName::Ellipsis, look.secondary, 15.))
                .on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    let viewport_h = f32::from(window.viewport_size().height);
                    let click_y = f32::from(event.position().y);
                    let upward = viewport_h > 0. && click_y + est_h + 12. > viewport_h;
                    set_flip.update(cx, |flip, _| *flip = upward);
                    toggle.update(cx, |open, cx| {
                        *open = !*open;
                        cx.notify();
                    });
                })
                .test_support(),
        );
        if shown {
            let close = open.clone();
            let upward = *flip_up.read(cx);
            let mut card = crate::action_menu::card("running-menu-card", 160., look)
                .right(px(0.))
                .occlude()
                .when(upward, |c| c.bottom(px(20.)))
                .when(!upward, |c| c.top(px(20.)));
            for (index, action) in self.actions.iter().copied().enumerate() {
                if index > 0 && separated(action) {
                    card = card.child(div().h(px(1.)).mx(px(4.)).my(px(2.)).bg(rgb(look.line)));
                }
                let (ui, close) = (self.ui.clone(), open.clone());
                let send = (self.wrap)(MachinePageAction::Running(RunningAction::Row(
                    self.holder.clone(),
                    action,
                )));
                let label = action_label(action);
                let red = action == RowAction::EndForGood;
                card = card.child(
                    div()
                        .id(SharedString::from(format!("{}-{index}", self.id)))
                        .role(Role::Button)
                        .aria_label(label)
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(5.))
                        .whitespace_nowrap()
                        .cursor_pointer()
                        .hover(|item| item.bg(rgb(look.fill)))
                        .text_color(rgb(if red { RED_STATUS } else { look.text }))
                        .child(label)
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            close.update(cx, |open, cx| {
                                *open = false;
                                cx.notify();
                            });
                            ui.send(send.clone(), cx);
                        })
                        .test_support(),
                );
            }
            host = host
                .child(deferred(
                    div()
                        .id(SharedString::from(format!("{}-scrim", self.id)))
                        .absolute()
                        .top(px(-10_000.))
                        .left(px(-10_000.))
                        .w(px(20_000.))
                        .h(px(20_000.))
                        .occlude()
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            close.update(cx, |open, cx| {
                                *open = false;
                                cx.notify();
                            });
                        }),
                ))
                .child(deferred(card));
        }
        host
    }
}

fn cell(look: Look, width: f32, text: impl Into<SharedString>) -> Div {
    div()
        .w(px(width))
        .flex_shrink_0()
        .whitespace_nowrap()
        .overflow_hidden()
        .text_ellipsis()
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
        .child(text.into())
}

/// A running holder's uptime cell. It reads the app's clock when it ticks,
/// on each whole minute of the run, never when the page is drawn
/// ([`humata2_ui_core::clock`]).
#[derive(IntoElement)]
struct Uptime {
    key: SharedString,
    now: Now,
    started_ms: i64,
    look: Look,
}

impl RenderOnce for Uptime {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let started_ms = self.started_ms;
        let now_ms = ticked_now(window, cx, self.key, &self.now, |at| {
            Some(until_next(at.saturating_sub(started_ms), 60_000))
        });
        cell(self.look, 56., uptime(now_ms, started_ms))
    }
}

/// What a row draws, apart from the menu.
struct Draw<'a> {
    ui: &'a Rc<Ui>,
    wrap: super::machine_page::Wrap,
    state: &'a MachinePageState,
    sessions: &'a [Session],
    channel_name: &'a dyn Fn(&ChannelId) -> String,
    look: Look,
}

/// A row's badge when its session is already archived, so a leftover shell
/// never masquerades as an active or stopped session.
pub const fn row_badge(holder: &RunningHolder) -> Option<&'static str> {
    if holder.archived {
        Some("Archived")
    } else {
        None
    }
}

/// Accessible label for a row, distinguishing archived sessions' leftovers.
pub fn row_aria_label(holder: &RunningHolder) -> String {
    let title = if holder.title.is_empty() {
        agent_label(holder.agent).to_owned()
    } else {
        holder.title.clone()
    };
    match row_badge(holder) {
        Some(badge) => format!("{title} ({badge})"),
        None => title,
    }
}

/// A row as its ⋯ button names it: its title, or its agent and folder for
/// an untitled holder, so two orphans of one agent differ.
fn row_name(holder: &RunningHolder) -> String {
    if holder.title.is_empty() {
        format!("{} in {}", agent_label(holder.agent), holder.folder)
    } else {
        holder.title.clone()
    }
}

/// What clicking a running-holder row itself does, when it names a live session.
#[must_use]
pub fn row_click_action(holder: &RunningHolder) -> Option<MachinePageAction> {
    if holder.archived {
        return None;
    }
    holder.session.clone().map(MachinePageAction::OpenSession)
}

fn holder_row(draw: &Draw<'_>, index: usize, group: &'static str, holder: &RunningHolder) -> AnyElement {
    let look = draw.look;
    let orphan = holder.is_orphan();
    let title = if holder.title.is_empty() {
        agent_label(holder.agent).to_owned()
    } else {
        holder.title.clone()
    };
    let status = holder
        .session
        .as_ref()
        .and_then(|id| draw.sessions.iter().find(|session| &session.id == id))
        .map_or(Status::Idle, |session| session.status);
    let place = if holder.desk {
        "Desk".to_owned()
    } else {
        holder.channel.as_ref().map_or_else(String::new, |channel| (draw.channel_name)(channel))
    };
    let when = match (holder.stopped, holder.started_unix_ms) {
        (true, _) => cell(look, 56., "Stopped").into_any_element(),
        (false, _) if holder.paused_for_move => cell(look, 56., "Paused for move").into_any_element(),
        (false, Some(started)) => Uptime {
            key: SharedString::from(format!("{group}-uptime-{index}-{started}")),
            now: draw.ui.now(),
            started_ms: started,
            look,
        }
        .into_any_element(),
        (false, None) => cell(look, 56., "").into_any_element(),
    };
    let actions = draw.state.running.row_actions(holder);
    let busy = draw.state.running.busy.as_ref() == Some(holder);
    let open = {
        let ui = draw.ui.clone();
        let click = (draw.wrap)(MachinePageAction::Running(RunningAction::Row(holder.clone(), RowAction::Open)));
        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            cx.stop_propagation();
            ui.send(click.clone(), cx);
        }
    };
    let row_action = row_click_action(holder).map(draw.wrap);
    let mut line = row()
        .id((group, index))
        .role(Role::ListItem)
        .aria_label(row_aria_label(holder))
        .gap(px(8.))
        .px(px(12.))
        .h(px(36.))
        .border_b_1()
        .border_color(rgb(look.line))
        .when(busy, |line| line.opacity(0.5))
        .when_some(row_action, |line, click_action| {
            let ui = draw.ui.clone();
            line.cursor_pointer()
                .hover(move |item| item.bg(rgb(look.fill)))
                .on_click(move |_, _, cx| ui.send(click_action.clone(), cx))
        })
        .child(if holder.stopped { chrome::stopped() } else { chrome::status(status) })
        .child(
            row()
                .when(orphan, |name| name.w(px(90.)).flex_shrink_0())
                .when(!orphan, |name| name.flex_1().min_w_0())
                .gap(px(6.))
                .child(
                    div()
                        .min_w_0()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .font_weight(FontWeight::MEDIUM)
                        .when(holder.stopped || holder.archived, |name| name.text_color(rgb(look.secondary)))
                        .child(title),
                )
                .children(row_badge(holder).map(|badge| {
                    div()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(rgb(look.line))
                        .text_size(px(10.5))
                        .text_color(rgb(look.secondary))
                        .flex_shrink_0()
                        .whitespace_nowrap()
                        .child(badge)
                })),
        );
    // A session row drops the folder column: its channel and agent say
    // where it is, and the card's width goes to the title and the buttons.
    if orphan {
        line = line.child(
            div()
                .flex_1()
                .min_w_0()
                .whitespace_nowrap()
                .overflow_hidden()
                .text_size(px(11.5))
                .text_color(rgb(look.secondary))
                .child(holder.folder.clone()),
        );
    } else {
        line = line.child(cell(look, 114., place)).child(cell(look, 74., agent_label(holder.agent)));
    }
    line = line.child(when);
    // Only an orphan's row has room for its pid.
    if orphan {
        line = line.child(
            div()
                .w(px(60.))
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(rgb(look.tertiary))
                .child(holder.pid.map_or_else(String::new, |pid| format!("pid {pid}"))),
        );
    }
    // A slot of its own, so a row that has the link and one that has not
    // keep their columns where they are.
    line = line.child(
        div()
            .w(px(72.))
            .flex_shrink_0()
            .flex()
            .justify_end()
            .children(holder.attached.then(|| {
                div()
                    .id((group, 1000 + index))
                    .role(Role::Button)
                    .aria_label("open in a tab")
                    .whitespace_nowrap()
                    .text_size(px(11.5))
                    .text_color(rgb(look.accent))
                    .cursor_pointer()
                    .child("open in a tab")
                    .on_click(open)
                    .test_support()
            })),
    );
    if actions.is_empty() {
        return line.test_support().into_any_element();
    }
    line.child(RowMenu {
        ui: draw.ui.clone(),
        id: SharedString::from(format!("{group}-menu-{index}")),
        holder: holder.clone(),
        actions,
        wrap: draw.wrap,
        look,
    })
    .test_support()
    .into_any_element()
}

/// The list: the sessions' card with how many rows it has, and the "Not in
/// a channel" group when any terminal is in it.
pub fn listing(
    ui: &Rc<Ui>,
    wrap: super::machine_page::Wrap,
    state: &MachinePageState,
    sessions: &[Session],
    channel_name: &dyn Fn(&ChannelId) -> String,
    look: Look,
) -> (usize, Div, Option<Div>) {
    let draw = Draw { ui, wrap, state, sessions, channel_name, look };
    let card = || {
        col()
            .rounded(px(9.))
            .border_1()
            .border_color(rgb(look.line))
            .bg(rgb(look.card))
    };
    let listed = state.running.sessions();
    let mut main = card();
    for (index, holder) in listed.iter().enumerate() {
        main = main.child(holder_row(&draw, index, "running-session", holder));
    }
    // A row's Stop, Resume, Archive or End for good the daemon refused says
    // why, under the rows, instead of failing silently.
    if let Some(error) = &state.running.error {
        main = main.child(
            div()
                .id("machine-page-running-error")
                .role(Role::Alert)
                .aria_label(error.clone())
                .px(px(12.))
                .py(px(8.))
                .text_size(px(12.))
                .text_color(rgb(RED_STATUS))
                .child(error.clone())
                .test_support(),
        );
    }
    let orphans = state.running.not_in_a_channel().map(|orphans| {
        let mut group = card();
        for (index, holder) in orphans.iter().enumerate() {
            group = group.child(holder_row(&draw, index, "running-orphan", holder));
        }
        col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(look.secondary))
                    .child(format!("Not in a channel \u{b7} {}", orphans.len())),
            )
            .child(group)
    });
    (listed.len(), main, orphans)
}

#[cfg(test)]
mod tests {
    use super::*;
    // The gpui glob above brings in its own `test` attribute, which expands
    // to `#[test]` again; the built-in one wins over a glob by name.
    use ::core::prelude::v1::test;

    const MINUTE: i64 = 60_000;

    #[test]
    fn uptime_reads_from_minutes_to_days() {
        assert_eq!(uptime(30_000, 0), "under a minute");
        assert_eq!(uptime(12 * MINUTE, 0), "12 min");
        assert_eq!(uptime(60 * MINUTE, 0), "1 h");
        assert_eq!(uptime(185 * MINUTE, 0), "3 h 5 min");
        assert_eq!(uptime(52 * 60 * MINUTE, 0), "2 d 4 h");
        assert_eq!(uptime(48 * 60 * MINUTE, 0), "2 d");
    }

    #[test]
    fn a_start_in_the_future_reads_as_just_started() {
        assert_eq!(uptime(0, 10 * MINUTE), "under a minute");
    }

    #[test]
    fn end_and_archive_sit_apart_from_the_rest() {
        assert!(separated(RowAction::EndForGood));
        assert!(separated(RowAction::Archive));
        assert!(!separated(RowAction::Stop));
        assert!(!separated(RowAction::Resume));
    }

    #[test]
    fn ending_reads_the_same_for_every_row() {
        assert!(action_label(RowAction::EndForGood).starts_with("End for good"));
    }

    fn sample_holder(stopped: bool, archived: bool) -> RunningHolder {
        RunningHolder {
            tab: Some("tb_shell".to_owned()),
            session: Some(humata2_clients::model::SessionId::new("se_1")),
            title: "Old work".to_owned(),
            channel: None,
            desk: true,
            agent: Agent::Shell,
            pid: (!stopped).then_some(42),
            folder: "/work/old".to_owned(),
            started_unix_ms: (!stopped).then_some(100),
            attached: false,
            machine: humata2_clients::model::MachineId::new("this-mac"),
            stopped,
            agent_tab: false,
            archived,
            paused_for_move: false,
        }
    }

    #[test]
    fn an_archived_holder_is_badged_apart_from_an_active_stopped_row() {
        let archived_stopped = sample_holder(true, true);
        let archived_running = sample_holder(false, true);
        let active_stopped = sample_holder(true, false);

        assert!(row_badge(&archived_stopped).is_some());
        assert!(row_badge(&archived_running).is_some());
        assert!(row_badge(&active_stopped).is_none());
        assert_ne!(row_aria_label(&archived_stopped), row_aria_label(&active_stopped));
    }

    #[test]
    fn clicking_a_session_holder_row_opens_that_session() {
        let running = sample_holder(false, false);
        let stopped = sample_holder(true, false);
        let archived = sample_holder(true, true);
        let mut orphan = sample_holder(false, false);
        orphan.session = None;

        let expected = MachinePageAction::OpenSession(humata2_clients::model::SessionId::new("se_1"));
        assert_eq!(row_click_action(&running), Some(expected.clone()));
        assert_eq!(row_click_action(&stopped), Some(expected));
        assert_eq!(row_click_action(&archived), None);
        assert_eq!(row_click_action(&orphan), None);
    }
}
