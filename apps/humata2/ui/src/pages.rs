//! The screens in the middle of the window, one per route.

use std::rc::Rc;

use crate::{chrome, theme::Look};
use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Sizable as _, input::InputState};
use gpui_kit::{prelude::*, *};
use humata2_app::{AppAction, AppState, Route};
use humata2_app::{
    PaneAction,
    layout::{FullScreen, PaneId},
};
use humata2_clients::model::{Agent, ForkKind, Session, Status};
use humata2_session::{MenuItem, SessionAction};

use crate::bridge::Ui;
use humata2_ui_core::clock::{ticked_now, until_next};
use humata2_ui_core::full_text::full_text;
use crate::widgets::{self, col, row};

/// The screen's title, for the top bar.
#[must_use]
pub fn title(state: &AppState) -> String {
    match &state.route {
        Route::Home => "Home".to_owned(),
        Route::Inbox => "Agent inbox".to_owned(),
        Route::Todo => "To-do".to_owned(),
        Route::Channel(id) => state
            .workspace
            .channels
            .iter()
            .find(|channel| &channel.id == id)
            .map_or_else(
                || "Channel".to_owned(),
                |channel| format!("#{}", channel.name),
            ),
        Route::Session(id) => state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map_or_else(|| "Session".to_owned(), |session| session.title.clone()),
        Route::Hosts => "Hosts".to_owned(),
        Route::Files => "Files".to_owned(),
        Route::Views => "Views".to_owned(),
        Route::Desk => "Desk".to_owned(),
        Route::Settings => "Settings".to_owned(),
        Route::Machine(id) => state
            .workspace
            .machine(id)
            .map_or_else(|| "Machine".to_owned(), |machine| machine.name.clone()),
    }
}

/// The screen for the current route.
pub fn page(
    ui: &Rc<Ui>,
    state: &AppState,
    channel_filter: Entity<InputState>,
    todo_inputs: &crate::todo::Inputs,
    files_name: Entity<InputState>,
    terminals: &crate::terminals::Terminals,
    documents: &crate::document_tab::Documents,
    machine_inputs: &crate::machine_page::MachinePageInputs,
    providers_inputs: &crate::settings::ProvidersInputs,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let look = Look::of(cx);
    match &state.route {
        Route::Home => home(ui, state, Look::of(cx)).into_any_element(),
        Route::Desk => crate::desk::page(ui, state, Look::of(cx)).into_any_element(),
        Route::Files => crate::file_manager::page(ui, &state.files, &files_name, Look::of(cx)),
        Route::Settings => {
            crate::settings::page(ui, state, providers_inputs, Look::of(cx), window, cx).into_any_element()
        }
        Route::Machine(id) => {
            crate::machine_page::route(ui, state, id, machine_inputs, Look::of(cx), window, cx)
        }
        Route::Hosts => crate::hosts::page(
            ui,
            state,
            f32::from(window.viewport_size().width) - state.layout.sidebar.width,
            Look::of(cx),
        )
        .into_any_element(),
        Route::Todo => todo_page(ui, state, todo_inputs, Look::of(cx)).into_any_element(),
        Route::Inbox => crate::lists::inbox(ui, state, Look::of(cx)).into_any_element(),
        Route::Channel(id) => state.workspace.channel(id).map_or_else(
            || {
                widgets::empty_in(look, "This channel is gone.", "Pick another in the sidebar.")
                    .into_any_element()
            },
            |channel| {
                // The channel's To-do tab is the To-do list, narrowed to it (D874).
                let todo = || todo_page(ui, state, todo_inputs, look).into_any_element();
                crate::channel::page(ui, state, channel, channel_filter, &todo, look)
            },
        ),
        Route::Session(id) => state
            .workspace
            .sessions
            .iter()
            .find(|session| &session.id == id)
            .map_or_else(
                || {
                    widgets::empty_in(look, "This session is gone.", "Pick another in the sidebar.")
                        .into_any_element()
                },
                |session| {
                    session_body(ui, state, session, Look::of(cx), terminals, documents, cx).into_any_element()
                },
            ),
        _ => widgets::empty_in(look, "Nothing here yet.", "This screen is being built.")
            .into_any_element(),
    }
}

pub(crate) fn home(ui: &Rc<Ui>, state: &AppState, look: Look) -> impl IntoElement {
    crate::lists::home(ui, state, look)
}

pub(crate) fn session_header(
    ui: &Rc<Ui>,
    state: &AppState,
    collapsed: bool,
    width: f32,
    look: Look,
) -> Div {
    let Some(session) = state
        .session
        .open
        .as_ref()
        .and_then(|id| state.workspace.session(id))
    else {
        return div();
    };
    let channel = state
        .workspace
        .channels
        .iter()
        .find(|x| x.id == session.channel);
    let machine = state
        .workspace
        .machines
        .iter()
        .find(|x| x.id == session.machine);
    let terminal_full = state.full_screen() == Some(FullScreen::Terminal);
    let compact = width < 560.;
    // P8: a Working session's run time counts up from its start.
    let working_since = session
        .status_since_ms
        .filter(|_| session.status == Status::Working);
    let mut crumbs = row()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .gap(px(5.))
        .text_size(px(11.5))
        .text_color(rgb(look.secondary));
    let desk = state.is_desk_session(&session.id);
    let diff_shows = state.panel() == Some(humata2_session::Panel::Changes)
        && !state.layout.panel.collapsed
        && state.full_screen() != Some(FullScreen::Terminal);
    // A channel's own Desk session names that channel and its Desk, and the
    // link goes to the Desk tab (frame 93).
    let owner = state
        .desk
        .channel_record(&session.id)
        .map(|found| found.channel.clone());
    let event = match &owner {
        Some(owner) => AppAction::OpenChannelDesk(owner.clone()),
        None => AppAction::Switch(if desk {
            Route::Desk
        } else {
            Route::Channel(session.channel.clone())
        }),
    };
    let ui2 = ui.clone();
    crumbs = crumbs
        .child(
            div()
                .id("session-channel")
                .role(Role::Link)
                .aria_label(if desk { "Open Desk" } else { "Open channel" })
                .text_color(rgb(look.accent))
                .child(match &owner {
                    Some(owner) => format!(
                        "# {} · Desk",
                        state
                            .workspace
                            .channel(owner)
                            .map_or_else(|| owner.to_string(), |x| x.name.clone())
                    ),
                    None if desk => "Desk".to_owned(),
                    None => format!("# {}", channel.map_or("", |x| x.name.as_str())),
                })
                .on_click(move |_, _, cx| ui2.send(event.clone(), cx))
                .test_support(),
        )
        .child("›");
    let mut heading = row()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .gap(px(7.))
        .child(chrome::status(session.status))
        .child({
            let title = session.shown_title();
            full_text(
                div()
                    .id("session-title")
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(14.))
                    .child(title.clone()),
                title,
            )
            .test_support()
        });
    let note = match session.status {
        Status::NeedsYou => Some(("Needs you".into(), 0xff9f0a)),
        Status::Exited { .. } => Some(("Agent exited".into(), 0xff453a)),
        // D784 A3: stopped by the person, frame 73's muted "Stopped".
        Status::AgentStopped => Some(("Stopped".into(), 0x8e8e93)),
        // Frame 64: "Moving to <machine>…" in orange.
        Status::Moving => Some((
            format!(
                "Moving to {}…",
                session
                    .moving
                    .as_ref()
                    .and_then(|moving| state.workspace.machines.iter().find(|x| x.id == moving.to))
                    .map_or("another machine", |x| x.name.as_str())
            ),
            0xff9f0a,
        )),
        Status::Offline => Some((
            format!(
                "{} is offline",
                machine.map_or("Machine", |x| x.name.as_str())
            ),
            0x8e8e93,
        )),
        _ => None,
    };
    if let Some((words, color)) = note.filter(|_| !compact) {
        heading = heading.child(
            div()
                .text_size(px(11.5))
                .text_color(rgb(color))
                .child(words),
        );
    }
    if let Some(since_ms) = working_since.filter(|_| !compact) {
        heading = heading.child(WorkingNote {
            ui: ui.clone(),
            session: session.id.to_string(),
            since_ms,
            look,
        });
    }
    row()
        .h(px(52.))
        .px(px(16.))
        .gap(px(if compact { 8. } else { 12. }))
        .flex_shrink_0()
        .border_b_1()
        .border_color(rgb(look.line))
        .bg(rgb(look.content))
        .when(collapsed || terminal_full, |x| {
            x.child(chrome::lights()).child(div().w(px(4.)))
        })
        // DS-58: the same ‹ Back as every other page's top bar.
        // A true ghost button: no fill, no border, only the ‹.
        .child({
            let back = ui.clone();
            Button::new("session-back")
                .ghost()
                .small()
                .w(px(24.))
                .h(px(24.))
                .p_0()
                .flex_shrink_0()
                .icon(chrome::icon(IconName::ChevronLeft, look.secondary, 14.))
                .accessibility_label("Back")
                .tooltip("Back  ⌘[")
                .disabled(state.trail.is_empty())
                .on_click(move |_, _, cx| {
                    cx.stop_propagation();
                    back.send(AppAction::Back, cx);
                })
        })
        .child(
            col()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .child(crumbs)
                .child(heading),
        )
        .when(!compact, |bar| {
            bar.child(
                div()
                    .text_size(px(11.5))
                    .text_color(rgb(look.secondary))
                    .child(format!(
                        "{} · {}",
                        if desk {
                            crate::desk::runs(session.agent)
                        } else if session.agent == Agent::Claude {
                            "Claude Code"
                        } else {
                            session.agent.name()
                        },
                        machine.map_or("", |x| x.name.as_str())
                    )),
            )
        })
        // D784 A3: a session the person stopped offers a plain Resume, as
        // frame 73's stopped rows do; an exited one keeps the blue button.
        .when(
            crate::session_tabs::offers_resume(session) && session.status == Status::AgentStopped,
            |bar| {
                bar.child(
                    widgets::send(ui, "resume-agent", "Resume", AppAction::Session(SessionAction::Resume))
                        .outline()
                        .h(px(24.))
                        .px(px(11.)),
                )
            },
        )
        .when(crate::session_tabs::offers_resume(session) && session.status != Status::AgentStopped, |bar| {
            bar.child(
                widgets::primary_in(look,
                    ui,
                    "resume-agent",
                    "Resume agent",
                    AppAction::Session(SessionAction::Resume),
                )
                .small()
                .h(px(24.))
                .bg(rgb(look.accent))
                .text_color(rgb(0xffffff)),
            )
        })
        // Frame 68: a pull request review is submitted from its header,
        // unless the Diff panel shows: its footer holds Submit review then
        // (DS-55), and one blue action is enough (DS-11).
        .when(session.pull_request_review && !desk && !diff_shows, |bar| {
            bar.child(
                widgets::primary_in(
                    look,
                    ui,
                    "submit-review",
                    "Submit review",
                    AppAction::PrSubmit(humata2_pr_review::submit::SubmitAction::Open(
                        session.id.clone(),
                    )),
                )
                .small()
                .h(px(24.))
                .bg(rgb(look.accent))
                .text_color(rgb(0xffffff)),
            )
        })
        .when(!desk, |bar| {
            bar.child(chrome::action(
                ui,
                "session-fork",
                "Fork",
                IconName::GitFork,
                AppAction::Session(SessionAction::StartFork(ForkKind::Worktree)),
                look.secondary,
            ))
        })
        .child({
            // Like `chrome::action`, with the shortcut in a tooltip so the
            // two arrows do not read as duplicates.
            let ui = ui.clone();
            let hint: SharedString = if terminal_full {
                "Return to columns  ⇧⌘↩".into()
            } else {
                "Expand terminal  ⇧⌘↩".into()
            };
            let shown = hint.clone();
            div()
                .id("terminal-full-screen")
                .role(Role::Button)
                .aria_label(if terminal_full {
                    "Return to columns"
                } else {
                    "Expand terminal"
                })
                .aria_description(hint)
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(shown.clone()).build(window, cx)
                })
                .size(px(15.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .child(chrome::icon(
                    if terminal_full {
                        IconName::Minimize2
                    } else {
                        IconName::Maximize2
                    },
                    look.secondary,
                    15.,
                ))
                .on_click(move |_, _, cx| {
                    ui.send(AppAction::ToggleFullScreen(FullScreen::Terminal), cx)
                })
                .test_support()
        })
        .child(chrome::action(
            ui,
            "toggle-side-panel",
            "Toggle side panel",
            IconName::PanelRight,
            AppAction::Pane(PaneAction::Toggle(PaneId::Panel)),
            if state.panel().is_some() {
                look.accent
            } else {
                look.secondary
            },
        ))
        .child(session_menu(ui, state, session, desk, look))
}

/// The header's ⋯ button and, when open, its menu (SE-7). No frame draws the
/// menu; it is the machine page's menu card (D295).
/// The words the header shows for a session that has been Working since
/// `since_ms` (unix ms), once the run is at least [`widgets::ELAPSED_AFTER_MS`]:
/// "Working · 2m 10s". `None` for any other status, when the start is not
/// known, or while the run is still short.
fn working_for(status: Status, since_ms: Option<i64>, now_ms: i64) -> Option<String> {
    if status != Status::Working {
        return None;
    }
    let run = now_ms.saturating_sub(since_ms?);
    (run >= widgets::ELAPSED_AFTER_MS)
        .then(|| format!("Working · {}", widgets::elapsed_text(run)))
}

/// The Working run time in the header (P8): its words once the run is long
/// enough, else an empty box. It reads the app's clock when it ticks (at 10 s,
/// then each second of the run), never when the header is drawn
/// ([`humata2_ui_core::clock`]).
#[derive(IntoElement)]
struct WorkingNote {
    ui: Rc<Ui>,
    session: String,
    since_ms: i64,
    look: Look,
}

impl RenderOnce for WorkingNote {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let since_ms = self.since_ms;
        let key = SharedString::from(format!("session-working-{}-{since_ms}", self.session));
        let now_ms = ticked_now(window, cx, key.clone(), &self.ui.now(), |at| {
            Some(working_next_change(since_ms, at))
        });
        let words = working_for(Status::Working, Some(since_ms), now_ms);
        // Before there is anything to say it is an empty box out of the
        // row's flow, so the heading keeps its spacing.
        let note = if words.is_none() { div().absolute() } else { div() };
        note.id(key)
            .text_size(px(11.5))
            .text_color(rgb(self.look.secondary))
            .children(words)
    }
}

/// How many ms after `at` the Working note of a run since `since_ms` next
/// reads differently: when its words appear, then each second.
fn working_next_change(since_ms: i64, at: i64) -> i64 {
    let run = at.saturating_sub(since_ms);
    if run < widgets::ELAPSED_AFTER_MS {
        widgets::ELAPSED_AFTER_MS - run
    } else {
        until_next(run, 1_000)
    }
}

fn session_menu(ui: &Rc<Ui>, state: &AppState, session: &Session, desk: bool, look: Look) -> Div {
    let open = state.session.menu;
    let pick = |item| AppAction::Session(SessionAction::MenuPick(item));
    div()
        .relative()
        .child(chrome::action(
            ui,
            "session-menu",
            "Session actions",
            IconName::Ellipsis,
            AppAction::Session(SessionAction::Menu(!open)),
            if open { look.accent } else { look.secondary },
        ))
        .when(open, |menu| {
            // Deferred, so both paint over the session under the header and
            // take the clicks there; the scrim first, the card over it.
            menu.child(deferred(crate::action_menu::scrim(
                ui,
                "session-menu-scrim",
                AppAction::Session(SessionAction::Menu(false)),
            )))
            .child(deferred(
                // Wide enough for its longest item, "New desk session for this
                // session…", with the items' own padding.
                crate::action_menu::card("session-menu-card", 256., look)
                    .test_support()
                    .top(px(26.))
                    .right(px(0.))
                    .child(crate::action_menu::item(
                        ui,
                        "session-menu-rename",
                        "Rename…",
                        pick(MenuItem::Rename),
                        look,
                    ))
                    .when(!desk, |card| {
                        card.child(crate::action_menu::item(
                            ui,
                            "session-menu-move",
                            "Move to another machine…",
                            pick(MenuItem::Move),
                            look,
                        ))
                    })
                    // CV-8: the Desk isn't a channel, so its sessions have none.
                    .when(!desk, |card| {
                        card.child(crate::action_menu::item(
                            ui,
                            "session-menu-continue",
                            "Continue in another channel…",
                            pick(MenuItem::ContinueInChannel),
                            look,
                        ))
                    })
                    // REPOS (CH-21): Add repo to this session.
                    .when(!desk, |card| {
                        card.child(crate::action_menu::item(
                            ui,
                            "session-menu-add-repo",
                            "Add repo to this session…",
                            AppAction::SessionAddRepo(humata2_app::SessionAddRepoAction::Open(
                                session.id.clone(),
                            )),
                            look,
                        ))
                    })
                    // CD-6: a Desk session has no channel to look after.
                    .when(!desk, |card| {
                        card.child(crate::action_menu::item(
                            ui,
                            "session-menu-new-desk",
                            "New desk session for this session…",
                            AppAction::NewDeskSessionFor(session.id.clone()),
                            look,
                        ))
                    })
                    // A stopped session has no agent to stop, so the item
                    // goes rather than greying out (no frame draws it, D295).
                    .when(!session.status.agent_ended(), |card| {
                        card.child(crate::action_menu::item(
                            ui,
                            "session-menu-stop",
                            "Stop the agent",
                            pick(MenuItem::Stop),
                            look,
                        ))
                    })
                    .child(crate::action_menu::item(
                        ui,
                        "session-menu-push",
                        "Push and open a pull request",
                        pick(MenuItem::PushAndPullRequest),
                        look,
                    ))
                    .child(crate::action_menu::item(
                        ui,
                        "session-menu-archive",
                        if desk { "Close session" } else { "Archive session…" },
                        pick(MenuItem::Archive),
                        look,
                    ))
            ))
        })
}

fn session_body(
    ui: &Rc<Ui>,
    state: &AppState,
    session: &Session,
    look: Look,
    terminals: &crate::terminals::Terminals,
    documents: &crate::document_tab::Documents,
    cx: &mut App,
) -> Div {
    let body = crate::session_tabs::body(ui, state, session, look, terminals, documents, cx);
    match review_notices(ui, state, session, look) {
        Some(bar) => col().size_full().child(body.flex_1().min_h_0()).child(bar),
        None => body,
    }
}

/// What the daemon's feed says moved on a review's pull requests (PR-15 to
/// PR-17), under the terminal: new commits with Update the clone, and a
/// merged or closed pull request with Archive (frame 68).
fn review_notices(ui: &Rc<Ui>, state: &AppState, session: &Session, look: Look) -> Option<Div> {
    use humata2_clients::pull_requests::PullRequestState;
    use humata2_pr_review::ask::AskAction;
    use humata2_pr_review::heads::HeadsAction;
    if !session.pull_request_review {
        return None;
    }
    let moved = state.pr_heads.notice(&session.id);
    let commits: u32 = moved
        .iter()
        .filter(|(_, moved)| moved.state == PullRequestState::Open)
        .map(|(_, moved)| moved.new_commits)
        .sum();
    let rewritten = moved
        .iter()
        .any(|(_, moved)| moved.state == PullRequestState::Open && moved.force_pushed);
    let closed = state.pr_heads.archivable(&session.id);
    let updating = state.pr_heads.updating.contains(&session.id);
    let bar = |id: &'static str, glyph: IconName, color: u32, words: String, button: Option<AnyElement>| {
        row()
            .id(id)
            // A notice: assistive technology reads what changed.
            .role(Role::Status)
            .aria_label(words.clone())
            .px(px(10.))
            .py(px(7.))
            .gap(px(8.))
            .items_center()
            .rounded(px(7.))
            .bg(rgb(look.fill))
            .text_color(rgb(look.text))
            .text_size(px(12.))
            .child(chrome::icon(glyph, color, 12.))
            .child(div().flex_1().min_w_0().child(words))
            .children(button)
            .test_support()
    };
    let mut bars = Vec::new();
    if commits > 0 || rewritten {
        let words = match (commits, rewritten) {
            (0, _) => "The PR branch was force-pushed".to_owned(),
            (1, false) => "1 new commit on the PR branch".to_owned(),
            (n, false) => format!("{n} new commits on the PR branch"),
            // A rewrite with new commits names both (case 12): the old
            // commits are gone, not only added to.
            (1, true) => "The PR branch was force-pushed with 1 new commit".to_owned(),
            (n, true) => format!("The PR branch was force-pushed with {n} new commits"),
        };
        let update = (!updating).then(|| {
            widgets::push_quiet(
                ui,
                "pr-update-clone",
                "Update the clone",
                AppAction::PrHeads(HeadsAction::UpdateClone(session.id.clone())),
                look,
            )
            .into_any_element()
        });
        bars.push(bar("pr-new-commits", IconName::Info, 0xff9f0a, words, update));
    }
    if let Some(pr) = closed.first() {
        let ended = moved
            .iter()
            .find(|(held, _)| held == pr)
            .map_or("closed", |(_, moved)| {
                if moved.state == PullRequestState::Merged { "merged" } else { "closed" }
            });
        bars.push(bar(
            "pr-ended",
            IconName::Info,
            0x8e8e93,
            format!("{}#{} was {ended}. Archive this review when you are done with it.", pr.repo, pr.number),
            Some(
                widgets::push_quiet(
                    ui,
                    "pr-archive",
                    "Archive",
                    AppAction::PrHeads(HeadsAction::Archive(session.id.clone())),
                    look,
                )
                .into_any_element(),
            ),
        ));
    }
    // PR-19: a question that could not be typed into the review chat says
    // why here, under that chat, until it is dismissed or asked again.
    if let Some(error) = state.pr_ask.error_for(&session.id) {
        bars.push(bar(
            "pr-ask-error",
            IconName::Info,
            0xff3b30,
            format!("The question didn't reach the review chat: {error}. Ask again once the agent is running."),
            Some(
                widgets::push_quiet(
                    ui,
                    "pr-ask-dismiss",
                    "Dismiss",
                    AppAction::PrAsk(AskAction::DismissError),
                    look,
                )
                .into_any_element(),
            ),
        ));
    }
    if bars.is_empty() {
        return None;
    }
    Some(
        col()
            .px(px(10.))
            .pb(px(10.))
            .gap(px(6.))
            .flex_shrink_0()
            .bg(rgb(look.term))
            .children(bars),
    )
}

#[cfg(test)]
mod tests {
    // Named, not `super::*`: the module's gpui glob brings in gpui's own
    // `test` attribute, which would shadow the built-in one.
    use ::core::prelude::v1::test;

    use super::{working_for, working_next_change};
    use crate::widgets::elapsed_text;
    use humata2_clients::model::Status;

    #[test]
    fn a_working_session_shows_how_long_it_has_run() {
        let text = working_for(Status::Working, Some(0), 130_000);
        assert!(text.is_some_and(|text| text.ends_with(elapsed_text(130_000).as_str())));
    }

    #[test]
    fn a_working_run_under_the_threshold_shows_nothing_yet() {
        assert_eq!(working_for(Status::Working, Some(0), 9_000), None);
    }

    #[test]
    fn a_session_that_is_not_working_shows_no_run_time() {
        assert_eq!(working_for(Status::Idle, Some(0), 130_000), None);
    }

    #[test]
    fn a_working_session_with_no_known_start_shows_no_run_time() {
        assert_eq!(working_for(Status::Working, None, 130_000), None);
    }

    #[test]
    fn the_run_time_ticks_when_it_first_shows_then_each_second() {
        assert_eq!(working_next_change(500, 4_500), 6_000);
        assert_eq!(working_next_change(500, 10_700), 800);
        assert_eq!(working_next_change(500, 10_500), 1_000);
    }
}

/// The To-do page, sending the list's actions into the app. A board view
/// being shown opens here (D874), so its Showing bar (VW-8) leads the page.
fn todo_page(ui: &Rc<Ui>, state: &AppState, inputs: &crate::todo::Inputs, look: Look) -> Div {
    let showing = state
        .views
        .showing
        .as_ref()
        .and_then(|showing| state.views.record(&showing.id))
        .filter(|record| {
            matches!(
                record.content,
                humata2_clients::views::ViewContent::Board(_)
                    | humata2_clients::views::ViewContent::SessionBoard { .. }
            )
        });
    let bar = showing.map(|record| crate::view_tab::showing_bar(ui, state, record, look));
    let send: crate::todo::Send = {
        let ui = ui.clone();
        Rc::new(move |action, cx| ui.send(AppAction::Todo(action), cx))
    };
    let sessions = |id: &str| session_chip(state, id);
    col()
        .size_full()
        .min_h_0()
        .children(bar)
        .child(crate::todo::page(&send, &state.todo, inputs, &sessions, look).flex_1().min_h_0())
}

/// A session as the To-do list's chips show it.
pub(crate) fn session_chip(state: &AppState, id: &str) -> Option<crate::todo::SessionChip> {
    state
        .workspace
        .sessions
        .iter()
        .find(|s| s.id.as_str() == id)
        .map(|s| crate::todo::SessionChip {
            name: s.title.clone(),
            dot: match s.status {
                Status::Working => crate::theme::GREEN,
                Status::NeedsYou => crate::theme::ORANGE,
                Status::Exited { .. } => crate::theme::RED_STATUS,
                _ => crate::theme::GREY,
            },
        })
}
