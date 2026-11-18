//! The decision card (IN-9): what is asked, why, what it would do, the rule
//! behind it, and the answers.
use crate::bridge::Ui;
use crate::theme::Look;
use crate::widgets::{col, row};
use gpui_kit::base::{ObservedElement, TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route, channel};
use humata2_clients::model::{
    Agent, Answer, CopyCard, HolderNote, HolderSession, LineKind, MigrateCard, MigrateStep,
    MigrateStepState, Preview, Release, Request, RequestKind, Status,
};
use humata2_inbox::{InboxAction, MigrateStatus, Offer};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use std::rc::Rc;

use crate::theme::{GREEN, GREY, ORANGE, RED_STATUS as RED};

/// The purple VIP mark the Inbox draws on what asks for attention.
pub(crate) fn vip_badge() -> Div {
    div()
        .px(px(4.))
        .rounded(px(3.))
        .bg(rgb(0xbf5af2))
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(0xffffff))
        .child("VIP")
}

/// A release as people read it: "2.0.5 (a1b2c3d)", the bundle shortened.
pub(crate) fn release_label(release: &Release) -> String {
    let short: String = release.bundle.chars().take(7).collect();
    match (release.version.is_empty(), short.is_empty()) {
        (false, false) => format!("{} ({short})", release.version),
        (false, true) => release.version.clone(),
        (true, false) => short,
        (true, true) => "its release".to_owned(),
    }
}

fn text(value: impl Into<SharedString>) -> Div {
    let value = value.into();
    div().child(div().child(value.clone()).test_text(value))
}

fn secondary(look: Look, value: impl Into<SharedString>) -> Div {
    text(value)
        .text_size(px(11.5))
        .text_color(rgb(look.secondary))
}

fn dot(status: Status) -> Div {
    let dot = div().size(px(8.)).rounded_full().flex_shrink_0();
    let dot = match status {
        Status::Working => dot.bg(rgb(GREEN)),
        Status::NeedsYou => dot.bg(rgb(ORANGE)),
        Status::Exited { .. } => dot.bg(rgb(RED)),
        Status::Offline | Status::Stopped => dot.bg(rgb(GREY)),
        _ => dot.border_1().border_color(rgb(GREY)),
    };
    row()
        .size(px(10.))
        .justify_center()
        .flex_shrink_0()
        .child(dot)
}

/// A button that sends `events` in order when clicked.
pub(crate) fn button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    events: Vec<AppAction>,
    look: Look,
    primary: bool,
) -> Button {
    let ui = ui.clone();
    let label = label.into();
    Button::new(id)
        .label(label.clone())
        .accessibility_label(label)
        .small()
        .h(px(24.))
        .px(px(11.))
        .rounded(px(6.))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .when(primary, |button| button.primary())
        .bg(rgb(if primary { look.accent } else { look.fill }))
        .text_color(rgb(if primary { 0xffffff } else { look.text }))
        .when(!primary, |button| {
            button.border_1().border_color(rgb(look.line))
        })
        .on_click(move |_, _, cx| {
            for event in &events {
                ui.send(event.clone(), cx);
            }
        })
}

fn agent_name(agent: Agent) -> &'static str {
    if agent == Agent::Claude {
        "Claude Code"
    } else {
        agent.name()
    }
}

fn labelled(look: Look, label: &'static str, value: impl Into<SharedString>, mono: bool) -> Div {
    row()
        .gap(px(8.))
        .items_start()
        .child(secondary(look, label).w(px(64.)).flex_shrink_0())
        .child(
            text(value)
                .flex_1()
                .min_w_0()
                .text_size(px(11.5))
                .when(mono, |value| value.font_family("Menlo")),
        )
}

/// Splits a text preview into "Key  Value" fields when every line is one.
fn fields(value: &str) -> Option<Vec<(&str, &str)>> {
    let lines: Vec<_> = value.lines().map(|line| line.split_once("  ")).collect::<Option<_>>()?;
    (lines.len() > 1).then_some(lines)
}

/// The IN-10 preview as labelled rows.
fn preview(request: &Request, look: Look) -> Div {
    let body = match &request.preview {
        Preview::Command(command) => col().child(labelled(look, "Command", command.clone(), true)),
        // "Key  Value" lines (two spaces) read as the frame's field rows: a
        // grey key, then its value, with no outer label.
        Preview::Text(value) if fields(value).is_some() => {
            col().gap(px(4.)).children(fields(value).into_iter().flatten().map(|(key, value)| {
                row()
                    .gap(px(8.))
                    .child(secondary(look, key.to_owned()).flex_shrink_0())
                    .child(text(value.to_owned()).min_w_0())
            }))
        }
        Preview::Text(value) => {
            let label = match request.kind {
                RequestKind::Network => "Reach",
                RequestKind::Question => "Asks",
                RequestKind::Merge => "Merge",
                RequestKind::Destroy => "Affects",
                _ => "Detail",
            };
            col().child(labelled(look, label, value.clone(), false))
        }
        // A Migrate item is drawn whole by `migrate_card`, and a copy by
        // `copy_lines`, never through here.
        Preview::Migrate(_) | Preview::CrossMachineCopy(_) => col(),
        Preview::Diff(lines) => col().gap(px(1.)).children(lines.iter().map(|line| {
            let color = match line.kind {
                LineKind::Added => GREEN,
                LineKind::Removed => RED,
                LineKind::Hunk => look.secondary,
                LineKind::Context => look.text,
            };
            text(line.text.clone())
                .text_size(px(11.5))
                .font_family("Menlo")
                .text_color(rgb(color))
        })),
    };
    body.p(px(8.)).rounded(px(6.)).bg(rgb(look.fill))
}

fn tick(ui: &Rc<Ui>, request: &Request, on: bool, look: Look) -> impl IntoElement {
    let ui = ui.clone();
    let id = request.id.clone();
    let label = "Allow this for the rest of the session";
    row()
        .id(SharedString::from(format!("always {}", request.id)))
        .role(Role::CheckBox)
        .aria_label(label)
        .aria_selected(on)
        .test_support()
        .gap(px(8.))
        .cursor_pointer()
        .child(
            row()
                .size(px(14.))
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(rgb(look.line))
                .when(on, |b| b.bg(rgb(look.accent)).text_color(rgb(0xffffff)))
                .text_size(px(10.))
                .child(if on { "✓" } else { "" }),
        )
        .child(text(label))
        .on_click(move |_, _, cx| {
            ui.send(AppAction::Inbox(InboxAction::ToggleAlways(id.clone())), cx);
        })
}

/// Who asks: the agent and its session, then with `tool` what it called (the
/// daemon's operation id, or else the rule's tool name, as the design draws).
pub(crate) fn requester(state: &AppState, request: &Request, tool: bool) -> String {
    let Some(s) = state.workspace.session(&request.session) else {
        return String::new();
    };
    let runs = if humata2_clients::desk::is_desk(s) {
        crate::desk::runs(s.agent)
    } else {
        agent_name(s.agent)
    };
    let mut line = format!("{runs} in {}", s.title);
    let called = if request.operation.is_empty() { &request.rule } else { &request.operation };
    if tool && !called.is_empty() {
        line.push_str(" · ");
        line.push_str(called);
    }
    line
}

/// What the reply box keeps between frames: the text input, and the
/// subscription that turns typing and Enter into Inbox actions.
struct ReplyInput {
    input: Entity<InputState>,
    _subscription: Subscription,
}

/// The reply being written. It owns its text input through keyed element
/// state, so it lives while the box is on screen and is dropped when the
/// reply is sent or cancelled. The draft stays in the Inbox state: typing
/// sends it, Enter sends the reply, and Escape (the global Dismiss) cancels.
#[derive(IntoElement)]
struct ReplyBox {
    ui: Rc<Ui>,
    id: humata2_clients::model::RequestId,
    draft: String,
}

impl RenderOnce for ReplyBox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = SharedString::from(format!("reply input {}", self.id));
        let ui = self.ui.clone();
        let held = window.use_keyed_state(key.clone(), cx, move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Write a reply…"));
            input.update(cx, |input, cx| input.focus(window, cx));
            let app: &mut App = &mut *cx;
            let _subscription = app.subscribe(
                &input,
                move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        ui.send(AppAction::Inbox(InboxAction::ReplyText(text)), cx);
                    }
                    InputEvent::PressEnter { .. } => {
                        ui.send(AppAction::Inbox(InboxAction::SendReply), cx);
                    }
                    InputEvent::Focus | InputEvent::Blur => {}
                },
            );
            ReplyInput { input, _subscription }
        });
        let input = held.read(cx).input.clone();
        // The store owns the draft: show it when it differs from the field.
        if input.read(cx).value().as_ref() != self.draft.as_str() {
            let draft = self.draft.clone();
            input.update(cx, |input, cx| input.set_value(draft, window, cx));
        }
        div().id(key).w_full().child(
            Input::new(&input)
                .small()
                .min_h(px(26.))
                .text_size(px(12.5))
                .aria_label("Reply"),
        )
    }
}

/// One request as the decision card. `wide` is the Inbox's detail pane; the
/// session panel leaves out the machine, channel and Open session.
pub(crate) fn card(
    ui: &Rc<Ui>,
    state: &AppState,
    request: &Request,
    wide: bool,
    look: Look,
) -> AnyElement {
    match request.migrate() {
        Some(migrate) => migrate_card(ui, state, request, migrate, wide, look).into_any_element(),
        None => decision_card(ui, state, request, wide, look).into_any_element(),
    }
}

/// A line with a leading glyph, for what keeps running and what was done.
fn glyph_line(look: Look, glyph: IconName, hex: u32, value: impl Into<SharedString>) -> Div {
    row()
        .gap(px(6.))
        .items_start()
        .child(div().flex_shrink_0().child(crate::chrome::icon(glyph, hex, 12.)))
        .child(text(value).flex_1().min_w_0().text_color(rgb(look.text)))
}

/// A migrate's steps, each with where it is (frame 79).
fn steps_list(look: Look, id: &Request, steps: &[MigrateStep]) -> ObservedElement<Stateful<Div>> {
    col()
        .id(SharedString::from(format!("steps {}", id.id)))
        .test_support()
        .gap(px(6.))
        .p(px(12.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .text_size(px(12.))
        .children(steps.iter().map(|step| {
            let (glyph, hex) = match step.state {
                MigrateStepState::Done => ("✓", GREEN),
                MigrateStepState::Doing => ("●", ORANGE),
                MigrateStepState::Failed => ("✗", RED),
                MigrateStepState::Pending => ("○", look.tertiary),
            };
            let label = text(step.label.clone());
            row()
                .gap(px(8.))
                .items_start()
                .child(div().flex_shrink_0().w(px(12.)).text_color(rgb(hex)).child(glyph))
                .child(match step.state {
                    MigrateStepState::Pending => label.text_color(rgb(look.tertiary)),
                    _ => label,
                })
                .flex_1()
        }))
}

/// "What changes": the lines that show, and "N more" to unfold the rest
/// (frame 78). A list that is not folded has no toggle.
fn changes_fold(ui: &Rc<Ui>, state: &AppState, request: &Request, look: Look) -> Option<ObservedElement<Stateful<Div>>> {
    let fold = state.inbox.changes(&request.id)?;
    if fold.shown.is_empty() {
        return None;
    }
    let total = fold.shown.len() + fold.more;
    let foldable = total > 3;
    let open = fold.more == 0;
    let head = secondary(look, format!("What changes · {total}"));
    let head = if foldable {
        let (ui, id) = (ui.clone(), request.id.clone());
        row()
            .id(SharedString::from(format!("toggle changes {}", request.id)))
            .role(Role::Button)
            .aria_label(format!("What changes · {total}"))
            .test_support()
            .gap(px(4.))
            .cursor_pointer()
            .child(crate::chrome::icon(
                if open { IconName::ChevronDown } else { IconName::ChevronRight },
                look.secondary,
                11.,
            ))
            .child(head)
            .on_click(move |_, _, cx| {
                ui.send(AppAction::Inbox(InboxAction::ToggleChanges(id.clone())), cx);
            })
            .into_any_element()
    } else {
        row().child(head).into_any_element()
    };
    let mut list = col()
        .id(SharedString::from(format!("changes {}", request.id)))
        .test_support()
        .gap(px(4.))
        .p(px(8.))
        .rounded(px(6.))
        .bg(rgb(look.fill))
        .text_size(px(11.5))
        .child(head)
        .children(fold.shown.iter().map(|line| text(line.clone())));
    if fold.more > 0 {
        let (ui, id) = (ui.clone(), request.id.clone());
        list = list.child(
            row()
                .id(SharedString::from(format!("more changes {}", request.id)))
                .role(Role::Button)
                .aria_label(format!("{} more", fold.more))
                .test_support()
                .cursor_pointer()
                .text_color(rgb(look.accent))
                .child(text(format!("{} more", fold.more)))
                .on_click(move |_, _, cx| {
                    ui.send(AppAction::Inbox(InboxAction::ToggleChanges(id.clone())), cx);
                }),
        );
    }
    Some(list)
}

/// What a holder's note says about its session.
const fn holder_note(note: HolderNote) -> &'static str {
    match note {
        HolderNote::RestartsWhenIdle => "restarts when it's idle",
        HolderNote::Restarting => "restarts now and resumes its conversation",
        HolderNote::NoIdleMoment => "can't tell when it's idle",
        HolderNote::StaysUntilClosed => "stays on its old release until you close it",
    }
}

/// The sessions and shells whose holders a Holders item restarts: those that
/// restart first, then "Stays until closed" (frame 83).
fn holders_list(ui: &Rc<Ui>, state: &AppState, request: &Request, holders: &[HolderSession], look: Look) -> Div {
    let (stays, waiting): (Vec<&HolderSession>, Vec<&HolderSession>) =
        holders.iter().partition(|holder| holder.note == HolderNote::StaysUntilClosed);
    let open_link = |holder: &HolderSession| {
        let session = holder.session.clone()?;
        let ui = ui.clone();
        Some(
            row()
                .id(SharedString::from(format!("open holder {} {}", request.id, session)))
                .role(Role::Button)
                .aria_label(format!("Open {} ↗", holder.title))
                .test_support()
                .flex_shrink_0()
                .cursor_pointer()
                .text_size(px(11.5))
                .text_color(rgb(look.accent))
                .child(text("Open session ↗"))
                .on_click(move |_, _, cx| {
                    ui.send(AppAction::Navigate(Route::Session(session.clone())), cx);
                }),
        )
    };
    let mut list = col().gap(px(8.));
    if !waiting.is_empty() {
        list = list.child(
            col()
                .id(SharedString::from(format!("waiting sessions {}", request.id)))
                .test_support()
                .gap(px(5.))
                .child(
                    secondary(look, format!("Sessions waiting · {}", waiting.len()))
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .children(waiting.iter().map(|holder| {
                    let status = holder
                        .session
                        .as_ref()
                        .and_then(|id| state.workspace.session(id))
                        .map_or(Status::Idle, |session| session.status);
                    row()
                        .gap(px(8.))
                        .child(dot(status))
                        .child(text(holder.title.clone()).font_weight(FontWeight::MEDIUM))
                        .child(secondary(look, holder_note(holder.note)))
                        .child(div().flex_1())
                        .children(open_link(holder))
                })),
        );
    }
    if !stays.is_empty() {
        list = list.child(
            col()
                .id(SharedString::from(format!("stays until closed {}", request.id)))
                .test_support()
                .gap(px(5.))
                .child(
                    secondary(look, format!("Stays until closed · {}", stays.len()))
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .children(stays.iter().map(|holder| {
                    row()
                        .gap(px(8.))
                        .child(crate::chrome::icon(IconName::SquareTerminal, look.secondary, 12.))
                        .child(text(holder.title.clone()).font_weight(FontWeight::MEDIUM))
                        .child(secondary(look, holder_note(holder.note)))
                        .child(div().flex_1())
                        .children(open_link(holder))
                })),
        );
    }
    list
}

/// A Migrate item (UP-1, D281): what changes and what keeps running, with
/// one Migrate button. It has no Always, Deny or reply; Migrate is the only
/// answer.
fn migrate_card(
    ui: &Rc<Ui>,
    state: &AppState,
    request: &Request,
    migrate: &MigrateCard,
    wide: bool,
    look: Look,
) -> impl IntoElement {
    let status = state.inbox.migrate_status(&request.id);
    let machine = state
        .workspace
        .machine(&migrate.machine)
        .map_or_else(|| migrate.machine.to_string(), |m| m.name.clone());
    let failed = matches!(status, Some(MigrateStatus::Failed(_)));
    let migrated = matches!(status, Some(MigrateStatus::Migrated { .. }));
    let from = match &status {
        Some(MigrateStatus::Failed(_)) => "From Humata · couldn't migrate".to_owned(),
        Some(MigrateStatus::Migrating) => "From Humata · migrating…".to_owned(),
        Some(MigrateStatus::Migrated { .. }) => "From Humata · migrated".to_owned(),
        _ => format!("From Humata · found when {machine} linked"),
    };
    let go_ui = ui.clone();
    let machine_id = migrate.machine.clone();
    let open_machine = wide.then(|| {
        row()
            .id(SharedString::from(format!("open machine {}", request.id)))
            .role(Role::Button)
            .aria_label(format!("Open {machine} ↗"))
            .test_support()
            .flex_shrink_0()
            .cursor_pointer()
            .text_size(px(11.5))
            .text_color(rgb(look.accent))
            .child(text(format!("Open {machine} ↗")))
            .on_click(move |_, _, cx| {
                go_ui.send(AppAction::Navigate(Route::Machine(machine_id.clone())), cx);
            })
    });

    let mut body = col()
        .id(SharedString::from(format!("request {}", request.id)))
        .role(Role::Group)
        .aria_label(request.title.clone())
        .test_support()
        .rounded(px(9.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(if failed {
            RED
        } else if migrated {
            look.line
        } else {
            ORANGE
        }))
        .p(px(12.))
        .gap(px(8.))
        .text_size(px(12.5))
        .child(
            row()
                .gap(px(6.))
                .when(!migrated, |r| r.child(vip_badge()))
                .child(crate::chrome::icon(IconName::Server, look.accent, 12.))
                .child(
                    text(request.title.clone())
                        .font_weight(FontWeight::SEMIBOLD)
                        .flex_1(),
                )
                .child(secondary(look, request.asked.clone())),
        )
        .child(row().gap(px(8.)).child(
            secondary(look, from)
                .flex_1()
                .min_w_0()
                .when(failed, |s| s.text_color(rgb(RED))),
        ))
        .children(open_machine);

    if let Some(MigrateStatus::Migrated { to, at }) = &status {
        let mut kept = at.clone();
        if !migrate.keeps_running.is_empty() {
            if !kept.is_empty() {
                kept.push_str(" · ");
            }
            kept.push_str(&migrate.keeps_running.join(" · "));
        }
        return body
            .child(
                glyph_line(look, IconName::Check, GREEN, format!("Migrated to {}", release_label(to)))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(secondary(look, kept));
    }

    body = body
        .child(text(request.reason.clone()))
        .child(
            text(format!(
                "{machine} runs {}; this app is {}.",
                release_label(&migrate.from),
                release_label(&migrate.to)
            ))
            .text_size(px(11.5)),
        );
    let running = matches!(status, Some(MigrateStatus::Migrating | MigrateStatus::Failed(_)));
    if running && !migrate.steps.is_empty() {
        body = body.child(steps_list(look, request, &migrate.steps));
    } else {
        body = body.children(changes_fold(ui, state, request, look));
    }
    match &status {
        Some(MigrateStatus::Migrating) if migrate.steps.is_empty() => {
            body = body.child(glyph_line(
                look,
                IconName::Info,
                ORANGE,
                format!(
                    "Migrating {machine} to {}… Sessions keep running; the link drops for a few seconds.",
                    release_label(&migrate.to)
                ),
            ));
        }
        Some(MigrateStatus::Failed(failure)) => {
            body = body.child(
                col()
                    .id(SharedString::from(format!("failure {}", request.id)))
                    .test_support()
                    .gap(px(6.))
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(7.))
                    .bg(rgb(look.fill))
                    .text_size(px(12.))
                    .child(
                        glyph_line(
                            look,
                            IconName::CircleX,
                            RED,
                            format!("Couldn't migrate the {} on {machine}", migrate.thing.label()),
                        )
                        .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(text(failure.message.clone()))
                    .child(text(failure.fix.clone())),
            );
        }
        _ => {}
    }
    if !failed && !migrate.keeps_running.is_empty() {
        body = body.child(
            col()
                .gap(px(3.))
                .child(secondary(look, "What keeps running").font_weight(FontWeight::SEMIBOLD))
                .children(
                    migrate
                        .keeps_running
                        .iter()
                        .map(|line| glyph_line(look, IconName::Check, GREEN, line.clone())),
                ),
        );
    }
    if !migrate.holders.is_empty() {
        body = body.child(holders_list(ui, state, request, &migrate.holders, look));
    } else if !migrate.waiting_sessions.is_empty() {
        body = body.child(
            col()
                .id(SharedString::from(format!("waiting sessions {}", request.id)))
                .test_support()
                .gap(px(4.))
                .child(
                    secondary(look, format!("Sessions waiting · {}", migrate.waiting_sessions.len()))
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .children(migrate.waiting_sessions.iter().map(|id| {
                    let name = state.workspace.session(id).map_or_else(
                        || id.to_string(),
                        |s| format!("{} · {}", s.title, agent_name(s.agent)),
                    );
                    row()
                        .gap(px(8.))
                        .child(dot(state.workspace.session(id).map_or(Status::Idle, |s| s.status)))
                        .child(text(name))
                })),
        );
    }
    if !request.why_you.is_empty() {
        body = body.child(
            row()
                .gap(px(8.))
                .items_start()
                .child(secondary(look, "Asks you because:").flex_shrink_0())
                .child(text(request.why_you.clone()).text_size(px(11.5)).flex_1().min_w_0()),
        );
    }
    let offers = state.inbox.offers(&request.id);
    body.child(row().gap(px(8.)).children(offers.into_iter().filter_map(|offer| {
        let label = match offer {
            Offer::Migrate => "Migrate",
            Offer::MigrateAgain => "Migrate again",
            _ => return None,
        };
        Some(button(
            ui,
            SharedString::from(format!("migrate {}", request.id)),
            label,
            vec![AppAction::Inbox(InboxAction::Migrate(request.id.clone()))],
            look,
            true,
        ))
    })))
}

/// Bytes as people read them: `2.1 GB`.
#[allow(clippy::cast_precision_loss)]
fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

/// One part of a copy card's sentence, findable by its own id.
fn part(request: &Request, name: &str, value: String) -> ObservedElement<Stateful<Div>> {
    div()
        .id(SharedString::from(format!("copy {name} {}", request.id)))
        .test_support()
        .child(value)
}

/// An agent's cross-machine copy (IN-10, D467): "<session> wants to copy
/// <what> (<size>) from <machine> → <machine> · <place>", with the real
/// paths behind Details.
fn copy_lines(ui: &Rc<Ui>, state: &AppState, request: &Request, copy: &CopyCard, look: Look) -> Div {
    let machine = |id: &humata2_clients::model::MachineId| {
        state
            .workspace
            .machine(id)
            .map_or_else(|| id.to_string(), |machine| machine.name.clone())
    };
    let session = state
        .workspace
        .session(&request.session)
        .map_or_else(|| request.session.to_string(), |session| session.title.clone());
    let open = state.inbox.details.contains(&request.id);
    // Reveal in Finder is offered only for a path on This Mac (PL-10).
    let on_this_mac = |id: &humata2_clients::model::MachineId| {
        state
            .workspace
            .machine(id)
            .is_some_and(humata2_clients::is_this_mac)
            .then(|| id.clone())
    };
    let toggle = {
        let (ui, id) = (ui.clone(), request.id.clone());
        row()
            .id(SharedString::from(format!("toggle details {}", request.id)))
            .role(Role::Button)
            .aria_label("Details")
            .test_support()
            .gap(px(4.))
            .cursor_pointer()
            .child(crate::chrome::icon(
                if open { IconName::ChevronDown } else { IconName::ChevronRight },
                look.secondary,
                11.,
            ))
            .child(secondary(look, "Details"))
            .on_click(move |_, _, cx| {
                ui.send(AppAction::Inbox(InboxAction::ToggleDetails(id.clone())), cx);
            })
    };
    let paths = open.then(|| {
        col()
            .id(SharedString::from(format!("details {}", request.id)))
            .test_support()
            .gap(px(4.))
            .p(px(8.))
            .rounded(px(6.))
            .bg(rgb(look.fill))
            .child(copy_side(
                ui,
                request,
                look,
                "From",
                machine(&copy.from),
                copy.source_path.lines().map(str::to_owned).collect(),
                on_this_mac(&copy.from),
            ))
            .child(copy_side(
                ui,
                request,
                look,
                "To",
                machine(&copy.to),
                vec![copy.destination_path.clone()],
                on_this_mac(&copy.to),
            ))
            .child(labelled(look, "Holds", format!("{} files, {}", copy.files, size_text(copy.bytes)), false))
    });
    col()
        .gap(px(6.))
        .child(
            row()
                .gap(px(4.))
                .flex_wrap()
                .font_weight(FontWeight::MEDIUM)
                .child(part(request, "session", format!("{session} wants to copy")))
                .child(part(request, "what", copy.what()))
                .child(part(request, "size", format!("({})", size_text(copy.bytes))))
                .child(div().child("from"))
                .child(part(request, "from", machine(&copy.from)))
                .child(div().child("→"))
                .child(part(request, "to", machine(&copy.to)))
                .children((!copy.place.is_empty()).then(|| {
                    row().gap(px(4.)).child(div().child("·")).child(part(request, "place", copy.place.clone()))
                })),
        )
        .child(toggle)
        .children(paths)
}

/// One side of a copy card's Details (PL-10): each of its paths with Copy
/// path, and Reveal in Finder when `reveal` is set (the side's machine is
/// This Mac).
fn copy_side(
    ui: &Rc<Ui>,
    request: &Request,
    look: Look,
    label: &'static str,
    machine: String,
    paths: Vec<String>,
    reveal: Option<humata2_clients::model::MachineId>,
) -> Div {
    col().gap(px(4.)).children(paths.into_iter().enumerate().map(|(index, path)| {
        let key = format!("{} {path}", request.id);
        let copy = path_action(
            ui,
            look,
            format!("copy path {key}"),
            "Copy path",
            InboxAction::CopyPath(path.clone()),
        );
        // A path Finder can't place (one named relative to a session) gets
        // Copy path only.
        let placeable = std::path::Path::new(&path).is_absolute();
        let finder = reveal.clone().filter(|_| placeable).map(|machine_id| {
            path_action(
                ui,
                look,
                format!("reveal path {key}"),
                "Reveal in Finder",
                InboxAction::Reveal(machine_id, path.clone()),
            )
        });
        // The label sits on the side's first path only.
        let heading = if index == 0 { secondary(look, label) } else { div() };
        row()
            .gap(px(8.))
            .items_start()
            .child(heading.w(px(64.)).flex_shrink_0())
            .child(
                text(format!("{machine}: {path}"))
                    .flex_1()
                    .min_w_0()
                    .text_size(px(11.5))
                    .font_family("Menlo"),
            )
            .child(copy)
            .children(finder)
    }))
}

/// A small ghost button beside a copy card's path, sending `action`.
fn path_action(
    ui: &Rc<Ui>,
    look: Look,
    id: String,
    label: &'static str,
    action: InboxAction,
) -> impl IntoElement {
    let ui = ui.clone();
    Button::new(SharedString::from(id))
        .accessibility_label(label)
        .ghost()
        .small()
        .h(px(23.))
        .px(px(6.))
        .child(text(label).text_color(rgb(look.accent)))
        .on_click(move |_, _, cx| {
            ui.send(AppAction::Inbox(action.clone()), cx);
        })
}

fn decision_card(
    ui: &Rc<Ui>,
    state: &AppState,
    request: &Request,
    wide: bool,
    look: Look,
) -> impl IntoElement {
    let session = state.workspace.session(&request.session);
    let pending = state.inbox.pending.contains(&request.id);
    let waiting = request.answer.is_none();
    let ticked = state.inbox.always.contains(&request.id);
    let channel_name = state
        .workspace
        .channel(&request.channel)
        .map(|c| c.name.clone())
        .or_else(|| (request.channel == humata2_clients::desk::channel()).then(|| "Desk".to_owned()));
    let machine_name = session
        .and_then(|s| state.workspace.machine(&s.machine))
        .map(|m| m.name.clone());

    // 1. Title, the requester's status, the age.
    let top = row()
        .gap(px(6.))
        .child(dot(session.map_or(Status::NeedsYou, |s| s.status)))
        .child(
            text(request.title.clone())
                .font_weight(FontWeight::SEMIBOLD)
                .flex_1(),
        )
        .child(secondary(look, request.asked.clone()));

    // 2. Who asks; in the detail pane, where it runs and a link to its session.
    let by = requester(state, request, true);
    let place = [machine_name, channel_name.map(|name| format!("# {name}"))]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let who = row()
        .gap(px(8.))
        .child(secondary(look, by).flex_1().min_w_0());
    let open_ui = ui.clone();
    let open_id = request.id.clone();
    let meta = wide.then(|| {
        row()
            .gap(px(8.))
            .child(secondary(look, place).flex_1().min_w_0())
            .child(
                row()
                    .id(SharedString::from(format!("open session {}", request.id)))
                    .role(Role::Button)
                    .aria_label("Open session ↗")
                    .test_support()
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(rgb(look.accent))
                    .child(text("Open session ↗"))
                    .on_click(move |_, _, cx| {
                        open_ui.send(
                            AppAction::Inbox(InboxAction::OpenRequest(open_id.clone())),
                            cx,
                        );
                    }),
            )
    });

    // 5. What the agent was doing: its last steps, the pending call last.
    let steps = (wide && !request.steps.is_empty()).then(|| {
        let count = request.steps.len();
        col()
            .id(SharedString::from(format!("steps {}", request.id)))
            .test_support()
            .gap(px(4.))
            .child(secondary(look, "What it was doing").font_weight(FontWeight::SEMIBOLD))
            .children(request.steps.iter().enumerate().map(|(index, step)| {
                let color = if waiting && index + 1 == count {
                    ORANGE
                } else {
                    GREEN
                };
                row()
                    .gap(px(6.))
                    .child(div().size(px(6.)).rounded_full().flex_shrink_0().bg(rgb(color)))
                    .child(
                        text(step.clone())
                            .text_size(px(11.5))
                            .font_family("Menlo"),
                    )
            }))
    });

    // 6. Why it asks you, with the rule's change link when a rule sent it.
    let why = if request.why_you.is_empty() {
        request
            .asked_by
            .as_ref()
            .map_or_else(String::new, |asked| asked.label.clone())
    } else {
        request.why_you.clone()
    };
    let change_rule = request.asked_by.as_ref().map(|_| {
        button(
            ui,
            SharedString::from(format!("change rule {}", request.id)),
            "Change this rule",
            vec![
                AppAction::Navigate(Route::Channel(request.channel.clone())),
                AppAction::Channel(channel::Action::Tab(channel::Tab::Access)),
            ],
            look,
            false,
        )
    });
    let why_line = (!why.is_empty()).then(|| {
        row()
            .gap(px(8.))
            .items_start()
            .child(secondary(look, "Asks you because:").flex_shrink_0())
            .child(text(why).text_size(px(11.5)).flex_1().min_w_0())
            .children(change_rule)
    });

    // 8. The answers. A question on its agent's screen offers its choices
    // and Dismiss, never Deny or Reply…: the wide Inbox puts Dismiss under
    // the terminal, the session's panel here (D781).
    let on_screen = request.on_screen.is_some();
    let is_question = request.kind == RequestKind::Question || on_screen;
    let copy = request.copy();
    let id = request.id.clone();
    let reply = state
        .inbox
        .reply
        .as_ref()
        .filter(|(reply_id, _)| *reply_id == request.id)
        .map(|(_, draft)| draft.clone());
    let answers = if !waiting {
        col().child(secondary(look, "Answered"))
    } else if pending {
        col().child(secondary(look, "Answering…"))
    } else if let Some(draft) = reply {
        col()
            .gap(px(6.))
            .child(ReplyBox {
                ui: ui.clone(),
                id: request.id.clone(),
                draft,
            })
            .child(
                row()
                    .gap(px(8.))
                    .child(button(
                        ui,
                        SharedString::from(format!("send reply {}", request.id)),
                        "Send reply",
                        vec![AppAction::Inbox(InboxAction::SendReply)],
                        look,
                        true,
                    ))
                    .child(button(
                        ui,
                        SharedString::from(format!("cancel reply {}", request.id)),
                        "Cancel",
                        vec![AppAction::Inbox(InboxAction::CancelReply)],
                        look,
                        false,
                    )),
            )
    } else if copy.is_some() {
        col().child(
            row()
                .gap(px(8.))
                .flex_wrap()
                .child(button(
                    ui,
                    SharedString::from(format!("approve {}", request.id)),
                    "Approve",
                    vec![AppAction::Inbox(InboxAction::Answer(id.clone(), Answer::Approved))],
                    look,
                    true,
                ))
                .child(button(
                    ui,
                    SharedString::from(format!("somewhere else {}", request.id)),
                    "Somewhere else…",
                    vec![AppAction::Inbox(InboxAction::SomewhereElse(id.clone()))],
                    look,
                    false,
                ))
                .child(button(
                    ui,
                    SharedString::from(format!("deny {}", request.id)),
                    "Deny",
                    vec![AppAction::Inbox(InboxAction::Answer(
                        id.clone(),
                        Answer::Denied { reply: None },
                    ))],
                    look,
                    false,
                )),
        )
    } else {
        col().child(
            row()
                .gap(px(8.))
                .flex_wrap()
                .when(!is_question, |r| {
                    r.child(button(
                        ui,
                        SharedString::from(format!("allow {}", request.id)),
                        "Allow",
                        vec![AppAction::Inbox(InboxAction::Answer(
                            id.clone(),
                            Answer::Approved,
                        ))],
                        look,
                        true,
                    ))
                    .child(button(
                        ui,
                        SharedString::from(format!("deny {}", request.id)),
                        "Deny",
                        vec![AppAction::Inbox(InboxAction::Answer(
                            id.clone(),
                            Answer::Denied { reply: None },
                        ))],
                        look,
                        false,
                    ))
                })
                .when(is_question, |r| {
                    r.children(request.choices.iter().enumerate().map(|(index, choice)| {
                        button(
                            ui,
                            SharedString::from(format!("choose {} {index}", request.id)),
                            choice.clone(),
                            vec![AppAction::Inbox(InboxAction::Choose(
                                id.clone(),
                                choice.clone(),
                            ))],
                            look,
                            index == 0,
                        )
                    }))
                })
                .child(div().flex_1())
                .when(!on_screen, |r| {
                    r.child(button(
                        ui,
                        SharedString::from(format!("reply {}", request.id)),
                        "Reply…",
                        vec![AppAction::Inbox(InboxAction::StartReply(id.clone()))],
                        look,
                        false,
                    ))
                })
                .when(on_screen && !wide, |r| {
                    r.child(button(
                        ui,
                        SharedString::from(format!("dismiss {}", request.id)),
                        "Dismiss",
                        vec![AppAction::Inbox(InboxAction::Dismiss(id.clone()))],
                        look,
                        false,
                    ))
                }),
        )
    };

    col()
        .id(SharedString::from(format!("request {}", request.id)))
        .role(Role::Group)
        .aria_label(request.title.clone())
        .test_support()
        .rounded(px(9.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(ORANGE))
        .p(px(12.))
        .gap(px(8.))
        .text_size(px(12.5))
        .child(top)
        .child(who)
        .children(meta)
        .child(text(request.reason.clone()))
        .child(match copy {
            Some(copy) => copy_lines(ui, state, request, copy, look),
            None => preview(request, look),
        })
        .children(steps)
        .children(why_line)
        .when_some(request.undo_note.clone(), |c, note| {
            c.child(text(note).text_size(px(11.5)).text_color(rgb(RED)))
        })
        .when(waiting && !request.rule.is_empty() && !is_question, |c| {
            c.child(tick(ui, request, ticked, look))
        })
        .child(answers)
}
