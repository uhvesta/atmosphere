//! The decision card (IN-9): what is asked, why, what it would do, the rule
//! behind it, and the answers.
use humata2_ui_core::bridge::Ui;
use humata2_ui_core::theme::Look;
use humata2_ui_core::widgets::{col, row};
use gpui_kit::base::Disableable as _;
use gpui_kit::base::{ObservedElement, TestSupportExt as _, TestTextExt as _};
use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_clients::progress::OpProgress;
use humata2_clients::model::{
    Agent, Answer, BrowserPreview, CardEffect, CopyCard, HolderNote, HolderSession, HoldersMoved,
    LineKind, MigrateCard, MigrateStep, MigrateStepState, Preview, Request, RequestKind, Status,
};
use humata2_inbox::{InboxAction, MigrateStatus, Offer};
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use std::rc::Rc;

use humata2_ui_core::theme::{GREEN, GREY, ORANGE, RED_STATUS as RED};

/// The purple VIP mark the Inbox draws on what asks for attention.
pub fn vip_badge() -> Div {
    div()
        .px(px(4.))
        .rounded(px(3.))
        .bg(rgb(0xbf5af2))
        .text_size(px(9.5))
        .font_weight(FontWeight::BOLD)
        .text_color(rgb(0xffffff))
        .child("VIP")
}

pub use humata2_ui_core::chrome::release_label;

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
        Status::Offline | Status::Stopped | Status::AgentStopped => dot.bg(rgb(GREY)),
        _ => dot.border_1().border_color(rgb(GREY)),
    };
    row()
        .size(px(10.))
        .justify_center()
        .flex_shrink_0()
        .child(dot)
}

/// A button that sends `events` in order when clicked.
pub fn button(
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

/// The request runs a tool with no arguments, so its preview would be an
/// empty command.
fn no_arguments(request: &Request) -> bool {
    matches!(&request.preview, Preview::Command(command) if command.trim().is_empty())
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
        // A Migrate item is drawn whole by `migrate_card`, a copy by
        // `copy_lines`, and a browser card by `browser_facts_box`, never here.
        Preview::Migrate(_) | Preview::CrossMachineCopy(_) | Preview::Browser(_) => col(),
        Preview::Diff(lines) => col().gap(px(1.)).children(lines.iter().map(|line| {
            let color = match line.kind {
                LineKind::Added => GREEN,
                LineKind::Removed => RED,
                LineKind::Hunk | LineKind::Note => look.secondary,
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
    let label = "Always allow for this session";
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
pub fn requester(state: &AppState, request: &Request, tool: bool) -> String {
    let Some(s) = state.workspace.session(&request.session) else {
        return String::new();
    };
    let runs = if humata2_clients::desk::is_desk(s) {
        humata2_ui_core::chrome::runs(s.agent)
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
    placeholder: &'static str,
}

impl RenderOnce for ReplyBox {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = SharedString::from(format!("reply input {}", self.id));
        let ui = self.ui.clone();
        let id = self.id.clone();
        let placeholder = self.placeholder;
        let held = window.use_keyed_state(key.clone(), cx, move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            input.update(cx, |input, cx| input.focus(window, cx));
            let app: &mut App = &mut *cx;
            let _subscription = app.subscribe(
                &input,
                move |input: Entity<InputState>, event: &InputEvent, cx: &mut App| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        ui.send(AppAction::Inbox(InboxAction::ReplyDraft(id.clone(), text)), cx);
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
        div().id(key).w_full().relative().child(
            Input::new(&input)
                .small()
                .min_h(px(26.))
                .text_size(px(12.5))
                .aria_label("Reply"),
        ).child(
            div()
                .absolute()
                .right(px(10.))
                .top(px(5.))
                .text_size(px(13.))
                .child("↵"),
        )
    }
}

/// One request as the decision card. `wide` is the Inbox's detail pane; the
/// session panel leaves out the machine, channel and Open session.
pub fn card(
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
        .child(div().flex_shrink_0().child(humata2_ui_core::chrome::icon(glyph, hex, 12.)))
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
            .child(humata2_ui_core::chrome::icon(
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
        HolderNote::Moves => "moves to the new release and keeps running",
        HolderNote::Moving => "moving now; it keeps running",
    }
}

/// The sessions and shells whose holders a Holders item restarts: those that
/// restart first, then "Stays until closed" (frame 83). While Migrate moves
/// them (`moving`), the first group is what is still to move (frame 125).
fn holders_list(
    ui: &Rc<Ui>,
    state: &AppState,
    request: &Request,
    holders: &[HolderSession],
    moving: bool,
    look: Look,
) -> Div {
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
                    secondary(look, waiting_header(waiting.len(), moving)).font_weight(FontWeight::SEMIBOLD),
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
                        .child(secondary(look, moving_note(holder.note, moving)))
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
                        .child(humata2_ui_core::chrome::icon(IconName::SquareTerminal, look.secondary, 12.))
                        .child(text(holder.title.clone()).font_weight(FontWeight::MEDIUM))
                        .child(secondary(look, holder_note(holder.note)))
                        .child(div().flex_1())
                        .children(open_link(holder))
                })),
        );
    }
    list
}

/// The header over a Holders item's waiting rows: "Still to move · 3
/// sessions" while Migrate moves them (frame 125), else "Sessions waiting · 3".
fn waiting_header(count: usize, moving: bool) -> String {
    if moving {
        format!("Still to move · {count} session{}", if count == 1 { "" } else { "s" })
    } else {
        format!("Sessions waiting · {count}")
    }
}

/// A row's note; while Migrate moves terminals, one queued to move reads
/// "next" and the one moving "moving now" (frame 125).
const fn moving_note(note: HolderNote, moving: bool) -> &'static str {
    match note {
        HolderNote::Moves if moving => "next",
        HolderNote::Moving if moving => "moving now",
        _ => holder_note(note),
    }
}

/// The progress line of a Holders item's move round: names the terminal
/// moving now, with the count on the right (frame 125).
fn moved_progress(moved: &HoldersMoved, holders: &[HolderSession]) -> OpProgress {
    let mut progress = moved.progress();
    if let Some(holder) = holders.iter().find(|holder| holder.note == HolderNote::Moving && !holder.title.is_empty()) {
        progress.title = format!("Moving {}", holder.title);
    }
    progress
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
                .child(humata2_ui_core::chrome::icon(IconName::Server, look.accent, 12.))
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
    // A Holders item that has begun to move terminals shows the count as a
    // progress line in place of the generic line.
    let moved = match (&status, &migrate.moved) {
        (Some(MigrateStatus::Migrating), Some(moved)) => Some(*moved),
        _ => None,
    };
    match &status {
        Some(MigrateStatus::Migrating) if migrate.steps.is_empty() && moved.is_none() => {
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
        body = body.child(holders_list(ui, state, request, &migrate.holders, moved.is_some(), look));
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
    // The count sits where the Migrate button was (frame 125).
    if let Some(moved) = moved {
        body = body.child(
            div()
                .id(SharedString::from(format!("migrate-moved-{}", request.id)))
                .test_support()
                .child(humata2_ui_core::widgets::progress_line(
                    look,
                    &moved_progress(&moved, &migrate.holders),
                    Some(ui.now()),
                )),
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
            .child(humata2_ui_core::chrome::icon(
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

fn is_dark(look: Look) -> bool {
    look.text > look.content
}

fn tone(look: Look, light: u32, dark: u32) -> u32 {
    if is_dark(look) { dark } else { light }
}

/// Text with `bold` set in bold at its start; it wraps like any text.
fn rich(lead: &str, rest: &str) -> StyledText {
    let whole = format!("{lead}{rest}");
    let spans = if lead.is_empty() {
        Vec::new()
    } else {
        vec![(
            0..lead.len(),
            HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() },
        )]
    };
    StyledText::new(whole).with_highlights(spans)
}

/// A small caps section label with an optional small remark.
fn section(look: Look, title: &'static str, small: Option<&'static str>, first: bool) -> Div {
    row()
        .items_baseline()
        .gap(px(4.))
        .mt(px(if first { 0. } else { 10. }))
        .mb(px(4.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(look.secondary))
        .child(title)
        .children(small.map(|small| {
            div().text_size(px(11.5)).font_weight(FontWeight::NORMAL).child(small)
        }))
}

/// The grey box: label, bold lead, rest.
fn facts_box(look: Look, request: &Request) -> ObservedElement<Stateful<Div>> {
    col()
        .id(SharedString::from(format!("facts {}", request.id)))
        .test_support()
        .gap(px(3.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(8.))
        .bg(rgb(look.fill))
        .children(request.facts.iter().map(|fact| {
            row()
                .child(
                    div()
                        .w(px(66.))
                        .flex_shrink_0()
                        .text_color(rgb(look.secondary))
                        .child(fact.label.clone()),
                )
                .child(div().flex_1().min_w_0().child(rich(&fact.lead, &fact.rest)))
        }))
}

fn effect_row(look: Look, effect: &humata2_clients::model::CardEffect) -> Div {
    let hue = if effect.warn {
        tone(look, 0xb25000, 0xffb340)
    } else {
        tone(look, 0x0f766e, 0x35c4b5)
    };
    row()
        .gap(px(8.))
        .py(px(1.5))
        .items_start()
        .child(
            row()
                .w(px(14.))
                .flex_shrink_0()
                .justify_center()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(hue))
                .child(if effect.warn { "!" } else { "✓" }),
        )
        .child(div().flex_1().min_w_0().child(rich(&effect.lead, &effect.rest)))
}

fn quiet(look: Look, value: impl Into<SharedString>) -> Div {
    text(value).text_color(rgb(look.secondary))
}

/// A text link that sends `action`.
fn link(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: &'static str,
    action: InboxAction,
    look: Look,
) -> ObservedElement<Stateful<Div>> {
    let ui = ui.clone();
    row()
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .test_support()
        .flex_shrink_0()
        .whitespace_nowrap()
        .cursor_pointer()
        .text_color(rgb(look.accent))
        .child(text(label))
        .on_click(move |_, _, cx| ui.send(AppAction::Inbox(action.clone()), cx))
}

/// The footer button: the frame's own size, a little larger than the
/// compact `button`.
fn answer_button(
    ui: &Rc<Ui>,
    id: impl Into<ElementId>,
    label: &'static str,
    action: InboxAction,
    look: Look,
    primary: bool,
    muted: bool,
) -> Button {
    button(ui, id, label, vec![AppAction::Inbox(action)], look, primary)
        .h(px(32.))
        .px(px(14.))
        .min_w(px(64.))
        .rounded(px(7.))
        .text_size(px(13.))
        .when(muted, |button| button.text_color(rgb(look.secondary)))
}

/// Default side-effect rows for a browser request when the daemon didn't attach
/// explicit `affects` rows (frames `b2` and `b3`).
fn browser_default_effects(is_write: bool, supports_30m_grant: bool) -> Vec<CardEffect> {
    if is_write {
        vec![
            CardEffect {
                warn: true,
                lead: "The action is made as you:".to_owned(),
                rest: " the page sees your signed-in account, not the agent".to_owned(),
            },
            CardEffect {
                warn: true,
                lead: "Humata can\u{2019}t tell what the page does next.".to_owned(),
                rest: " Check the URL and the target".to_owned(),
            },
            CardEffect {
                warn: false,
                lead: String::new(),
                rest: "Only this one action: the next click, typed text or page is asked again"
                    .to_owned(),
            },
        ]
    } else {
        let mut rows = vec![
            CardEffect {
                warn: false,
                lead: String::new(),
                rest: "Nothing is clicked, typed or opened: this only reads".to_owned(),
            },
            CardEffect {
                warn: true,
                lead: "The page is read as you see it, signed in:".to_owned(),
                rest: " it can hold things only your account can open".to_owned(),
            },
        ];
        if supports_30m_grant {
            rows.push(CardEffect {
                warn: true,
                lead: "Allow 30 min is wider than this tab:".to_owned(),
                rest: " this session can read any of your tabs, and list them, for 30 minutes. No other session can"
                    .to_owned(),
            });
        }
        rows
    }
}

/// Default "CAN IT BE UNDONE?" text for a browser request when the daemon didn't
/// send one (frames `b2` and `b3`).
fn browser_default_undo(is_write: bool, supports_30m_grant: bool) -> &'static str {
    if is_write {
        "Not by Humata. What the action does is up to the page."
    } else if supports_30m_grant {
        "No. Once the agent has the text, it keeps it. Allow 30 min ends after 30 minutes, or sooner if Humata restarts."
    } else {
        "No. Once the agent has the text, it keeps it."
    }
}

/// The grey facts box for a browser read or write card (frames `b2` and `b3`).
/// For a write, the exact URL is highlighted in an amber box and the target
/// preview is shown prominently.
fn browser_facts_box(
    state: &AppState,
    request: &Request,
    browser: &BrowserPreview,
    is_write: bool,
    agent: &str,
    look: Look,
) -> ObservedElement<Stateful<Div>> {
    let session = state.workspace.session(&request.session);
    let machine_name = session
        .and_then(|s| state.workspace.machine(&s.machine))
        .map_or_else(|| "This Mac".to_owned(), |m| m.name.clone());
    let from_rest = match session {
        Some(s) if !agent.is_empty() => format!(" \u{b7} {agent} in {}", s.title),
        Some(s) => format!(" \u{b7} {}", s.title),
        None => String::new(),
    };
    let fact_row = |label: &'static str, value: AnyElement| {
        row()
            .items_start()
            .child(
                div()
                    .w(px(66.))
                    .flex_shrink_0()
                    .text_color(rgb(look.secondary))
                    .child(label),
            )
            .child(div().flex_1().min_w_0().child(value))
    };
    let mut box_col = col()
        .id(SharedString::from(format!("facts {}", request.id)))
        .test_support()
        .gap(px(4.))
        .px(px(12.))
        .py(px(9.))
        .rounded(px(8.))
        .bg(rgb(look.fill));

    if is_write {
        if !browser.action_summary.is_empty() {
            box_col = box_col.child(fact_row(
                "Action",
                div()
                    .child(rich(&browser.action_summary, ""))
                    .into_any_element(),
            ));
        }
        if !browser.url.is_empty() {
            let waiting_border = tone(look, 0xf0b35a, 0xb9812f);
            let url_box = div()
                .id(SharedString::from(format!("browser url {}", request.id)))
                .test_support()
                .px(px(6.))
                .py(px(1.))
                .rounded(px(5.))
                .bg(rgb(look.attention_bg))
                .border_1()
                .border_color(rgb(waiting_border))
                .font_family("Menlo")
                .text_size(px(12.))
                .child(browser.url.clone())
                .test_text(browser.url.clone());
            box_col = box_col.child(fact_row("URL", url_box.into_any_element()));
        }
        if !browser.target_preview.is_empty() {
            let target_box = div()
                .id(SharedString::from(format!("browser target {}", request.id)))
                .test_support()
                .px(px(8.))
                .py(px(2.))
                .rounded(px(6.))
                .bg(rgb(look.card))
                .border_1()
                .border_color(rgb(look.line))
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(12.))
                .child(browser.target_preview.clone())
                .test_text(browser.target_preview.clone());
            box_col = box_col.child(fact_row("Target", target_box.into_any_element()));
        }
    } else {
        if !browser.target_preview.is_empty() {
            box_col = box_col.child(fact_row(
                "Tab",
                div()
                    .child(rich(&browser.target_preview, ""))
                    .into_any_element(),
            ));
        }
        if !browser.url.is_empty() {
            let url_text = text(browser.url.clone())
                .font_family("Menlo")
                .text_size(px(12.));
            box_col = box_col.child(fact_row("URL", url_text.into_any_element()));
        }
        if !browser.action_summary.is_empty() {
            box_col = box_col.child(fact_row(
                "Reads",
                text(browser.action_summary.clone()).into_any_element(),
            ));
        }
    }

    box_col = box_col
        .child(fact_row(
            "Browser",
            text("Your browser on This Mac").into_any_element(),
        ))
        .child(fact_row(
            "From",
            div()
                .child(rich(&machine_name, &from_rest))
                .into_any_element(),
        ));

    box_col
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
    let on_screen = request.on_screen.is_some();
    let is_question = request.kind == RequestKind::Question || on_screen;
    let is_browser_read = request.kind == RequestKind::BrowserRead;
    let is_browser_write = request.kind == RequestKind::BrowserWrite;
    let is_browser = is_browser_read || is_browser_write;
    let copy = request.copy();
    let rich_card = !request.facts.is_empty() || !request.affects.is_empty();
    let id = request.id.clone();
    let agent = session.map_or_else(String::new, |s| {
        if humata2_clients::desk::is_desk(s) {
            humata2_ui_core::chrome::runs(s.agent).to_owned()
        } else {
            agent_name(s.agent).to_owned()
        }
    });
    let channel_name = state
        .workspace
        .channel(&request.channel)
        .map(|c| c.name.clone())
        .or_else(|| (request.channel == humata2_clients::desk::channel()).then(|| "Desk".to_owned()));

    // Title row: dot, title; in the page the requester, channel, age and the
    // link to the session.
    let who = {
        let mut parts = Vec::new();
        if let Some(s) = session {
            parts.push(format!("{agent} in {}", s.title));
        }
        parts.extend(channel_name);
        parts.push(format!("{} ago", request.asked.trim_end_matches(" ago")));
        parts.join(" · ")
    };
    let top = row()
        .gap(px(10.))
        .items_center()
        .child(dot(if waiting {
            Status::NeedsYou
        } else {
            session.map_or(Status::NeedsYou, |s| s.status)
        }))
        .child(
            text(request.title.clone())
                .text_size(px(if wide { 15.5 } else { 14. }))
                .font_weight(FontWeight::BOLD)
                .flex_1()
                .min_w(px(if wide { 240. } else { 0. }))
                .line_clamp(2)
                .text_ellipsis(),
        )
        .when(wide, |top| {
            top.child(
                quiet(look, who)
                    .text_size(px(12.5))
                    .flex_shrink(1.)
                    .min_w_0()
                    .whitespace_nowrap()
                    .overflow_hidden()
                    .text_ellipsis(),
            )
            .child(
                link(
                    ui,
                    SharedString::from(format!("open session {}", request.id)),
                    "Open session ↗",
                    InboxAction::OpenRequest(id.clone()),
                    look,
                )
                .text_size(px(12.5))
                .flex_shrink_0()
                .whitespace_nowrap(),
            )
        });

    let working = (!request.working_on.is_empty())
        .then(|| quiet(look, format!("Working on {}", request.working_on)).mt(px(2.)));

    // The columns.
    let left;
    let right;
    if is_question {
        let asks = if request.reason.is_empty() {
            match &request.preview {
                Preview::Text(value) if request.choices.is_empty() => value.clone(),
                _ => String::new(),
            }
        } else {
            format!("“{}”", request.reason)
        };
        left = col()
            .children((!asks.is_empty()).then(|| {
                col()
                    .child(section(look, "WHAT IT NEEDS TO KNOW", None, true))
                    .child(text(asks).line_height(relative(1.5)))
            }))
            .children(working);
        let busy = pending || !waiting;
        let reply = state
            .inbox
            .reply
            .as_ref()
            .filter(|(reply_id, _)| *reply_id == request.id)
            .map(|(_, draft)| draft.clone())
            .unwrap_or_default();
        right = col()
            .child(section(look, "YOUR ANSWER", None, true))
            .child(
                col()
                    .gap(px(6.))
                    .items_start()
                    .children(request.choices.iter().enumerate().map(|(index, choice)| {
                        button(
                            ui,
                            SharedString::from(format!("choose {} {index}", request.id)),
                            choice.clone(),
                            vec![AppAction::Inbox(InboxAction::Choose(id.clone(), choice.clone()))],
                            look,
                            false,
                        )
                        .h(px(32.))
                        .px(px(14.))
                        .text_size(px(13.))
                        .disabled(busy)
                    }))
                    .when(!on_screen && waiting, |c| {
                        c.child(
                            div().w_full().mt(px(4.)).child(ReplyBox {
                                ui: ui.clone(),
                                id: request.id.clone(),
                                draft: reply,
                                placeholder: "Or type an answer…",
                            }),
                        )
                    }),
            );
    } else if is_browser {
        let mut l = col();
        let mut r = col();
        let fallback_preview = BrowserPreview {
            url: String::new(),
            action_summary: request.title.clone(),
            target_preview: String::new(),
            supports_30m_grant: is_browser_read,
            action: String::new(),
            action_key: String::new(),
            reason: request.reason.clone(),
        };
        let browser = request.browser().unwrap_or(&fallback_preview);
        if !request.facts.is_empty() && !is_browser_write {
            l = l.child(facts_box(look, request));
        } else {
            l = l.child(browser_facts_box(
                state,
                request,
                browser,
                is_browser_write,
                &agent,
                look,
            ));
        }
        let why_text = if !request.reason.is_empty() {
            request.reason.as_str()
        } else {
            browser.reason.as_str()
        };
        if !why_text.is_empty() {
            l = l
                .child(section(look, "WHY", Some("the agent's reason"), false))
                .child(text(format!("“{why_text}”")).line_height(relative(1.5)));
        }
        l = l.children(working);

        let effects = if request.affects.is_empty() {
            browser_default_effects(is_browser_write, browser.supports_30m_grant)
        } else {
            request.affects.clone()
        };
        r = r
            .child(section(look, "WHAT ELSE IT AFFECTS", Some("worked out by Humata"), true))
            .children(effects.iter().map(|effect| effect_row(look, effect)));

        let undo = if request.undo_text.is_empty() {
            browser_default_undo(is_browser_write, browser.supports_30m_grant).to_owned()
        } else {
            request.undo_text.clone()
        };
        r = r
            .child(section(look, "CAN IT BE UNDONE?", None, false))
            .child(text(undo));

        left = l;
        right = r;
    } else {
        let mut l = col();
        let mut r = col();
        if rich_card {
            l = l.child(facts_box(look, request));
            if !request.reason.is_empty() {
                l = l
                    .child(section(look, "WHY", Some("the agent's reason"), false))
                    .child(text(format!("“{}”", request.reason)).line_height(relative(1.5)));
            }
            l = l.children(working);
            r = r.child(section(look, "WHAT ELSE IT AFFECTS", Some("worked out by Humata"), true))
                .children(request.affects.iter().map(|effect| effect_row(look, effect)));
            if !request.undo_text.is_empty() {
                r = r
                    .child(section(look, "CAN IT BE UNDONE?", None, false))
                    .child(text(request.undo_text.clone()));
            }
        } else {
            // From an older daemon: the reason and the preview alone.
            if !request.reason.is_empty() {
                l = l.child(text(request.reason.clone()).line_height(relative(1.5)));
            }
            if copy.is_some() || !no_arguments(request) {
                l = l.child(
                    match copy {
                        Some(copy) => copy_lines(ui, state, request, copy, look),
                        None => preview(request, look),
                    }
                    .mt(px(6.)),
                );
            }
            l = l.children(working);
            if wide && !request.steps.is_empty() {
                let count = request.steps.len();
                r = r.child(section(look, "WHAT IT WAS DOING", None, true)).children(
                    request.steps.iter().enumerate().map(|(index, step)| {
                        let color = if waiting && index + 1 == count { ORANGE } else { GREEN };
                        row()
                            .gap(px(6.))
                            .child(div().size(px(6.)).rounded_full().flex_shrink_0().bg(rgb(color)))
                            .child(text(step.clone()).text_size(px(11.5)).font_family("Menlo"))
                    }),
                );
            }
            if let Some(note) = request.undo_note.clone() {
                r = r.child(text(note).text_size(px(11.5)).text_color(rgb(RED)));
            }
            if !request.why_you.is_empty() {
                r = r
                    .child(section(look, "WHY YOU", None, false))
                    .child(text(request.why_you.clone()));
            }
        }
        left = l;
        right = r;
    }
    let columns = if wide {
        row().gap(px(28.)).items_start().mt(px(10.))
            .child(left.flex_1().min_w_0())
            .child(right.flex_1().min_w_0())
    } else {
        col().gap(px(4.)).mt(px(6.)).child(left).child(right)
    };

    // You asked why: the agent's reply, or a waiting line before it comes.
    let asked = request.asked_why.as_ref().map(|why| {
        let body = match &why.reply {
            Some(reply) => format!("{agent}, {} ago: “{reply}”", why.replied.clone().unwrap_or_else(|| why.asked.clone())),
            None => format!("Waiting for {agent} to answer. Asked {} ago.", why.asked),
        };
        let dim = why.reply.as_ref().map(|_| {
            let name = body.find(':').map_or(0, |end| end + 1);
            (0..name, HighlightStyle { color: Some(rgb(look.secondary).into()), ..Default::default() })
        });
        col()
            .id(SharedString::from(format!("asked why {}", request.id)))
            .test_support()
            .mt(px(12.))
            .px(px(13.))
            .py(px(10.))
            .rounded(px(9.))
            .bg(rgb(tone(look, 0xeef4ff, 0x20304d)))
            .child(
                text("You asked why · the request is on hold")
                    .mb(px(3.))
                    .font_weight(FontWeight::SEMIBOLD),
            )
            .child(
                div()
                    .line_height(relative(1.5))
                    .child(StyledText::new(body).with_highlights(dim)),
            )
    });

    // The footer.
    let reply = state
        .inbox
        .reply
        .as_ref()
        .filter(|(reply_id, _)| *reply_id == request.id)
        .map(|(_, draft)| draft.clone());
    let rule_label = if request.rule_label.is_empty() {
        request.asked_by.as_ref().map(|asked| asked.label.clone()).unwrap_or_default()
    } else {
        request.rule_label.clone()
    };
    let rule_note = (!rule_label.is_empty()).then(|| {
        let change = link(
            ui,
            SharedString::from(format!("change rule {}", request.id)),
            "Change this rule",
            InboxAction::ChangeRule,
            look,
        );
        if wide {
            row()
                .flex_wrap()
                .gap(px(4.))
                .text_size(px(12.5))
                .text_color(rgb(look.secondary))
                .child(text("Asked because"))
                .child(text(rule_label).font_weight(FontWeight::MEDIUM))
                .child(text("is set to Ask me ·"))
                .child(change)
        } else {
            // Panel width: the rule wraps on its own line, then the switch.
            col()
                .w_full()
                .min_w_0()
                .gap(px(2.))
                .text_size(px(12.5))
                .text_color(rgb(look.secondary))
                .child(
                    row()
                        .w_full()
                        .flex_wrap()
                        .items_start()
                        .gap(px(4.))
                        .child(text("Asked because"))
                        .child(text(rule_label).font_weight(FontWeight::MEDIUM).min_w_0()),
                )
                .child(
                    row()
                        .w_full()
                        .flex_wrap()
                        .gap(px(4.))
                        .child(text("is set to Ask me ·"))
                        .child(change),
                )
        }
    });
    let plain_decision = waiting
        && !pending
        && reply.is_none()
        && !is_question
        && !is_browser
        && !request.dismiss_only
        && !(copy.is_some() && !rich_card);
    let footer = if !waiting {
        row().child(quiet(look, answered_text(request.answer.as_ref())))
    } else if pending {
        row().child(quiet(look, "Answering…"))
    } else if let (Some(draft), false) = (reply, is_question) {
        // No, with a note: the note box, then Send and Cancel.
        row()
            .gap(px(8.))
            .items_center()
            .child(div().flex_1().child(ReplyBox {
                ui: ui.clone(),
                id: request.id.clone(),
                draft,
                placeholder: "Add a note for the agent…",
            }))
            .child(button(
                ui,
                SharedString::from(format!("send reply {}", request.id)),
                "No, send note",
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
            ))
    } else if request.dismiss_only {
        // D798: nothing an answer does would type into the agent's terminal.
        row().child(button(
            ui,
            SharedString::from(format!("dismiss {}", request.id)),
            "Dismiss",
            vec![AppAction::Inbox(InboxAction::Dismiss(id.clone()))],
            look,
            true,
        ))
    } else if is_question {
        let stopped = if agent.is_empty() {
            "The agent is stopped until you answer".to_owned()
        } else {
            format!("{agent} is stopped until you answer")
        };
        row()
            .flex_wrap()
            .gap(px(8.))
            .items_center()
            .child(
                quiet(look, "A question changes nothing by itself; the answer goes to the agent as you give it.")
                    .flex_1()
                    .min_w_0(),
            )
            .child(quiet(look, stopped).text_size(px(12.5)).flex_shrink_0().ml_auto())
            .when(on_screen && !wide, |r| {
                r.child(button(
                    ui,
                    SharedString::from(format!("dismiss {}", request.id)),
                    "Dismiss",
                    vec![AppAction::Inbox(InboxAction::Dismiss(id.clone()))],
                    look,
                    false,
                ))
            })
    } else if is_browser {
        // Frames b2 & b3:
        // - Browser read: Allow once, Allow 30 min (only if supports_30m_grant), Deny.
        // - Browser write: Allow once, Deny ONLY.
        // No Ask why, no Always allow box, no rule link.
        let offers = state.inbox.offers(&request.id);
        let mut buttons = row().flex_wrap().gap(px(8.)).items_center();
        for offer in offers {
            match offer {
                Offer::Allow => {
                    buttons = buttons.child(answer_button(
                        ui,
                        SharedString::from(format!("allow {}", request.id)),
                        "Allow once",
                        InboxAction::Answer(id.clone(), Answer::Approved),
                        look,
                        true,
                        false,
                    ));
                }
                Offer::AllowFor30Min => {
                    buttons = buttons.child(answer_button(
                        ui,
                        SharedString::from(format!("allow 30m {}", request.id)),
                        "Allow 30 min",
                        InboxAction::Answer(id.clone(), Answer::AllowFor30Min),
                        look,
                        false,
                        false,
                    ));
                }
                Offer::Deny => {
                    buttons = buttons.child(answer_button(
                        ui,
                        SharedString::from(format!("deny {}", request.id)),
                        "Deny",
                        InboxAction::Answer(id.clone(), Answer::Denied { reply: None }),
                        look,
                        false,
                        false,
                    ));
                }
                _ => {}
            }
        }
        buttons
    } else if copy.is_some() && !rich_card {
        row()
            .gap(px(8.))
            .flex_wrap()
            .child(answer_button(ui, SharedString::from(format!("approve {}", request.id)), "Approve",
                InboxAction::Answer(id.clone(), Answer::Approved), look, true, false))
            .child(answer_button(ui, SharedString::from(format!("somewhere else {}", request.id)), "Somewhere else…",
                InboxAction::SomewhereElse(id.clone()), look, false, false))
            .child(answer_button(ui, SharedString::from(format!("deny {}", request.id)), "Deny",
                InboxAction::Answer(id.clone(), Answer::Denied { reply: None }), look, false, false))
    } else {
        let asking = state.inbox.asking.contains(&request.id);
        let (why_label, why_muted) = if request.asked_why.is_some() {
            ("Asked why ✓", true)
        } else if asking {
            ("Asking…", true)
        } else {
            ("Ask why", false)
        };
        row()
            .flex_wrap()
            .gap(px(8.))
            .items_center()
            .child(answer_button(ui, SharedString::from(format!("allow {}", request.id)), "Yes",
                InboxAction::Answer(id.clone(), Answer::Approved), look, true, false))
            .child(answer_button(ui, SharedString::from(format!("deny {}", request.id)), "No",
                InboxAction::Answer(id.clone(), Answer::Denied { reply: None }), look, false, false))
            .child(
                answer_button(ui, SharedString::from(format!("ask why {}", request.id)), why_label,
                    InboxAction::AskWhy(id.clone()), look, false, why_muted)
                    .ml(px(8.))
                    .disabled(why_muted),
            )
            .when(!request.rule.is_empty(), |r| r.child(tick(ui, request, ticked, look)))
    };
    let note = if is_browser && waiting {
        if is_browser_write {
            Some(
                col()
                    .items_end()
                    .text_size(px(12.5))
                    .text_color(rgb(look.secondary))
                    .child(text(
                        "Asked every time: clicks, typing and opening pages can\u{2019}t be allowed ahead",
                    ))
                    .child(text("Not part of Allow all")),
            )
        } else {
            let stopped = if agent.is_empty() {
                "The agent is stopped until you answer".to_owned()
            } else {
                format!("{agent} is stopped until you answer")
            };
            Some(
                col()
                    .items_end()
                    .text_size(px(12.5))
                    .text_color(rgb(look.secondary))
                    .child(text(stopped)),
            )
        }
    } else {
        rule_note.filter(|_| waiting && !is_question)
    };
    let reply_link = plain_decision.then(|| {
        link(
            ui,
            SharedString::from(format!("reply {}", request.id)),
            "No, with a note…",
            InboxAction::StartReply(id.clone()),
            look,
        )
        .text_size(px(12.))
        .text_color(rgb(look.secondary))
    });
    let footer = if wide {
        row()
            .flex_wrap()
            .gap(px(8.))
            .items_center()
            .mt(px(14.))
            .pt(px(12.))
            .border_t_1()
            .border_color(rgb(look.line))
            .child(footer.flex_1().min_w_0())
            .when(note.is_some() || reply_link.is_some(), |r| {
                r.child(
                    col()
                        .ml_auto()
                        .flex_shrink_0()
                        .items_end()
                        .children(note)
                        .children(reply_link),
                )
            })
    } else {
        // Panel width: everything stacks and wraps inside the card.
        col()
            .gap(px(8.))
            .items_start()
            .mt(px(14.))
            .pt(px(12.))
            .border_t_1()
            .border_color(rgb(look.line))
            .child(footer.w_full().min_w_0())
            .child(
                col()
                    .w_full()
                    .min_w_0()
                    .items_start()
                    .gap(px(4.))
                    .children(reply_link)
                    .children(note),
            )
    };

    col()
        .id(SharedString::from(format!("request {}", request.id)))
        .role(Role::Group)
        .aria_label(request.title.clone())
        .test_support()
        .rounded(px(12.))
        .bg(rgb(look.card))
        .border_1()
        .border_color(rgb(if waiting { tone(look, 0xf0b35a, 0xb9812f) } else { look.line }))
        .px(px(if wide { 18. } else { 14. }))
        .pt(px(16.))
        .pb(px(14.))
        .text_size(px(if wide { 13. } else { 12.5 }))
        .child(top)
        .child(columns)
        .children(asked)
        .child(footer)
}

/// How an answered request was answered, in a line.
fn answered_text(answer: Option<&Answer>) -> String {
    match answer {
        None => String::new(),
        Some(Answer::Approved) => "Answered: yes".to_owned(),
        Some(Answer::AllowFor30Min) => "Answered: allowed for 30 min".to_owned(),
        Some(Answer::Always { .. }) => "Answered: yes, always for this session".to_owned(),
        Some(Answer::Denied { reply: None }) => "Answered: no".to_owned(),
        Some(Answer::Denied { reply: Some(note) }) => format!("Answered: no · {note}"),
        Some(Answer::Replied { text }) => format!("Answered: {text}"),
    }
}

#[cfg(test)]
mod browser_card_tests {
    use super::*;

    #[core::prelude::v1::test]
    fn answered_text_formats_allow_for_30_min() {
        assert_eq!(
            answered_text(Some(&Answer::AllowFor30Min)),
            "Answered: allowed for 30 min"
        );
    }

    #[core::prelude::v1::test]
    fn browser_default_effects_distinguish_read_and_write() {
        let read_30m = browser_default_effects(false, true);
        assert_eq!(read_30m.len(), 3);
        assert!(read_30m.iter().any(|e| e.lead.contains("Allow 30 min")));

        let read_once = browser_default_effects(false, false);
        assert_eq!(read_once.len(), 2);
        assert!(!read_once.iter().any(|e| e.lead.contains("Allow 30 min")));

        let write = browser_default_effects(true, false);
        assert_eq!(write.len(), 3);
        assert!(write[0].warn && write[1].warn && !write[2].warn);
        assert!(write[1].rest.contains("URL"));
    }
}
