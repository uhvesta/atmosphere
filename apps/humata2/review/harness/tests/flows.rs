//! The review pane driven the way a person drives it: clicks, drags, keys
//! and typing through the real view, on fakes of the agent's terminal, the
//! store, the review history and the review chat's tool calls. Everything
//! runs headless; no window is shown.
//!
//! Each flow writes a markdown artifact of its steps. Snapshots a flow takes
//! are kept in `tests/__snapshots__/`; record them with
//! `HUMATA2_SNAPSHOT=record` and read every file before keeping it.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AnyWindowHandle, Entity, Role, ScrollDelta, TestAppContext, point, px};
use humata2_flow::{Flow, GpuiDriver, SpecFlow, Target, artifact_root, run};
use humata2_review_harness::{
    Event, Fakes, lib_rs, open_pane, two_repos, pr_review, prepared, record_events, review_focus,
    review_note, small_review,
};
use humata2_review_view::model::{
    AgentState, Base, FileKey, Kind, LineRange, Review, SessionId, Status, Switch,
};
use humata2_review_view::{ActionError, ActionOutcome, PaneEvent, ReviewPane};
use humata2_snapshot::snapshots;

const WIDTH: f32 = 1280.;
const HEIGHT: f32 = 900.;
/// Longer than the pane waits after typing before it saves a draft.
const DRAFT_SAVE: Duration = Duration::from_millis(400);
/// Longer than the pane waits for the user to be still (PR-24).
const QUIET: Duration = Duration::from_millis(900);

fn session(id: &str) -> SessionId {
    SessionId::new(id)
}

fn play(cx: &mut TestAppContext, window: AnyWindowHandle, flow: Flow) {
    let snapshots = snapshots!().unwrap_or_else(|error| panic!("{error}"));
    // Bazel sends artifacts to the test's outputs instead (see artifact_root).
    let out = artifact_root(&std::env::temp_dir());
    if let Err(error) = run(&flow, &mut GpuiDriver::new(cx, window), &snapshots, &out) {
        panic!("{error}");
    }
}

/// Shows `id` with `review`, its agent in `state`, and waits for the load.
fn show(cx: &mut TestAppContext, pane: &Entity<ReviewPane>, id: &str, review: Review, state: AgentState) {
    pane.update(cx, |pane, cx| {
        pane.show_session(session(id), cx);
        pane.set_review(session(id), review, cx);
        pane.set_agent_state(session(id), state, cx);
    });
    cx.run_until_parked();
}

fn read<R>(cx: &mut TestAppContext, pane: &Entity<ReviewPane>, id: &str, f: impl FnOnce(&humata2_review_view::model::SessionReview) -> R) -> R {
    pane.read_with(cx, |pane, _| f(pane.session(&session(id)).unwrap_or_else(|| panic!("session {id} is loaded"))))
}

fn button(label: &str) -> (Role, String) {
    (Role::Button, label.to_owned())
}

fn line(label: &str) -> (Role, String) {
    (Role::ListItem, label.to_owned())
}

trait FlowExt {
    fn press(self, target: (Role, String)) -> Self;
}

impl FlowExt for Flow {
    fn press(self, (role, label): (Role, String)) -> Self {
        self.click(role, label)
    }
}

// 26.3: comments on several files, sent as one message.

fn comment_on_two_files() -> Flow {
    Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments"))
        .press(line("app/src/lib.rs line 3"))
        .key("c")
        .type_text("Keep run() here")
        .key("secondary-enter")
        .drag(
            Target::new(Role::ListItem, "docs/guide.txt line 10"),
            Target::new(Role::ListItem, "docs/guide.txt line 11"),
        )
        .key("c")
        .type_text("Say why")
        .key("secondary-enter")
        .press(button("Send all drafts"))
        .snapshot()
        .press(button("Send"))
        .snapshot()
}

#[gpui_kit::test]
fn comments_on_two_repos_reach_the_agent_as_one_message(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    let events = record_events(cx, &pane);
    show(cx, &pane, "s1", two_repos(), AgentState::Idle);
    play(cx, window, comment_on_two_files());

    let typed = fakes.typed();
    assert_eq!(typed.len(), 1, "one message: {typed:?}");
    assert!(typed[0].contains("Keep run() here") && typed[0].contains("Say why"));
    assert!(typed[0].contains("app/src/lib.rs") && typed[0].contains("docs/guide.txt"));
    read(cx, &pane, "s1", |review| {
        assert!(review.book().comments().iter().all(|c| c.status == Status::Sent));
        let second = &review.book().comments()[1];
        assert_eq!(second.anchor.lines, LineRange::new(10, 11), "the drag selected two lines");
    });
    assert!(events.borrow().iter().any(|e| matches!(e, PaneEvent::Sent { comments: 2, .. })));
}

#[gpui_kit::test]
fn a_busy_agent_gets_the_message_when_its_turn_ends_and_an_exited_one_never(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "s1", two_repos(), AgentState::Working);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments, agent busy"))
            .press(line("app/src/lib.rs line 3"))
            .key("c")
            .type_text("Later")
            .key("secondary-enter")
            .key("shift-s")
            .key("enter"),
    );
    assert!(fakes.typed().is_empty(), "nothing is typed into a busy agent");
    pane.update(cx, |pane, cx| pane.set_agent_state(session("s1"), AgentState::Exited, cx));
    cx.run_until_parked();
    assert!(fakes.typed().is_empty(), "nor into one that exited");
    read(cx, &pane, "s1", |review| {
        assert_eq!(review.book().comments()[0].status, Status::Failed);
    });
    pane.update(cx, |pane, cx| pane.set_agent_state(session("s1"), AgentState::Idle, cx));
    cx.run_until_parked();
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments, retry"))
            .press(button("Retry send #1")),
    );
    assert_eq!(fakes.typed().len(), 1);
}

// Restarts: comments, drafts and the queue survive a kill at any instant.

#[gpui_kit::test]
fn a_kill_at_any_step_of_sending_neither_loses_nor_repeats_a_comment(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "s1", two_repos(), AgentState::Idle);
    play(cx, window, comment_on_two_files());
    let log = fakes.log();
    assert!(log.iter().any(|e| matches!(e, Event::Typed { .. })));

    for events in 0..=log.len() {
        let world = fakes.killed_at(events);
        let before = world.saved(&session("s1"));
        let typed_before = world.typed().len();
        let (_, pane) = open_pane(cx, &world, WIDTH, HEIGHT);
        show(cx, &pane, "s1", two_repos(), AgentState::Idle);

        let typed = world.typed();
        assert!(typed.len() <= 1, "killed after {events} events, typed twice: {typed:?}");
        let Some(before) = before else {
            continue;
        };
        let saved = humata2_review_view::model::SessionReview::restore(before);
        let had: Vec<_> = saved.book().comments().iter().map(|c| c.body.clone()).collect();
        read(cx, &pane, "s1", |review| {
            let now: Vec<_> = review.book().comments().iter().map(|c| c.body.clone()).collect();
            assert_eq!(now, had, "killed after {events} events, the saved comments come back");
            if !saved.book().deliveries().is_empty() {
                assert_eq!(world.typed().len(), 1, "killed after {events} events, a queued message goes once");
                assert!(review.book().comments().iter().all(|c| c.status == Status::Sent));
            }
        });
        if typed_before == 1 {
            assert_eq!(world.typed().len(), 1, "killed after {events} events, it isn't typed again");
        }
    }
}

#[gpui_kit::test]
fn a_comment_being_written_survives_a_session_switch_and_a_restart(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "s1", two_repos(), AgentState::Idle);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments, unfinished"))
            .press(line("app/src/lib.rs line 4"))
            .key("c")
            .type_text("Half a tho"),
    );
    cx.executor().advance_clock(DRAFT_SAVE);
    cx.run_until_parked();
    show(cx, &pane, "s2", two_repos(), AgentState::Idle);
    read(cx, &pane, "s2", |review| assert!(review.composer().is_none(), "sessions share no drafts"));
    show(cx, &pane, "s1", two_repos(), AgentState::Idle);
    read(cx, &pane, "s1", |review| {
        assert_eq!(review.composer().map(|d| d.text.as_str()), Some("Half a tho"));
    });

    let world = fakes.killed_at(fakes.log().len());
    let (window, pane) = open_pane(cx, &world, WIDTH, HEIGHT);
    show(cx, &pane, "s1", two_repos(), AgentState::Idle);
    read(cx, &pane, "s1", |review| {
        assert_eq!(review.composer().map(|d| d.text.as_str()), Some("Half a tho"));
    });
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments, finished after a restart"))
            .press((Role::TextInput, "Comment for the agent".to_owned()))
            .type_text("ught")
            .key("secondary-enter"),
    );
    read(cx, &pane, "s1", |review| {
        assert_eq!(review.book().comments()[0].body, "Half a thought");
        assert_eq!(review.book().comments()[0].anchor.lines, LineRange::line(4));
    });
}

#[gpui_kit::test]
fn a_review_that_moved_on_keeps_comments_on_their_lines_or_marks_them_outdated(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "s1", Review::new(vec![lib_rs("lib-2")]), AgentState::Working);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments, then the agent edits"))
            .press(line("app/src/lib.rs line 4"))
            .key("c")
            .type_text("server.run() can fail")
            .key("secondary-enter"),
    );
    // The agent rewrote the line.
    let mut moved = lib_rs("lib-3");
    if let humata2_review_view::model::Content::Text(diff) = &mut moved.content {
        diff.hunks[0].lines[4].text = "    server.run()?;".into();
    }
    pane.update(cx, |pane, cx| pane.set_review(session("s1"), Review::new(vec![moved]), cx));
    cx.run_until_parked();
    read(cx, &pane, "s1", |review| assert!(review.book().comments()[0].outdated));
}

// 26.17: a PR review with the review chat.

fn ask_about_refunds() -> Flow {
    Flow::new(SpecFlow::new("26.17", "Review a pull request, ask"))
        .press(button("Ask about hunk 1 of app/src/refund.rs"))
        .type_text("Where does a partial capture get refunded?")
        .key("secondary-enter")
        .key("shift-s")
        .snapshot()
        .key("enter")
}

#[gpui_kit::test]
fn asking_about_a_hunk_types_the_question_and_its_code_into_the_review_chat_once(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    play(cx, window, ask_about_refunds());
    let typed = fakes.typed();
    assert_eq!(typed.len(), 1, "{typed:?}");
    assert!(typed[0].contains("Where does a partial capture get refunded?"));
    assert!(typed[0].contains("step_refund(0)"), "the quoted code goes with it: {}", typed[0]);
    assert_eq!(fakes.asked(), 1);
    read(cx, &pane, "pr", |review| assert_eq!(review.book().comments()[0].kind, Kind::Question));
}

#[gpui_kit::test]
fn asking_about_a_line_or_a_file_queues_while_the_review_chat_answers(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "pr", pr_review(), AgentState::Working);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.17", "Review a pull request, ask about a line and a file"))
            .press(line("app/src/ledger.rs line 2"))
            .key("a")
            .type_text("Is this rounded?")
            .key("secondary-enter")
            .press(button("Ask about app/src/util.rs"))
            .type_text("Is this file needed?")
            .key("secondary-enter")
            .key("shift-s")
            .key("enter"),
    );
    assert!(fakes.typed().is_empty(), "the review chat is answering");
    pane.update(cx, |pane, cx| pane.set_agent_state(session("pr"), AgentState::Idle, cx));
    cx.run_until_parked();
    let typed = fakes.typed();
    assert_eq!(typed.len(), 1);
    assert!(typed[0].contains("Is this rounded?") && typed[0].contains("Is this file needed?"));
    read(cx, &pane, "pr", |review| {
        let anchors: Vec<_> = review.book().comments().iter().map(|c| c.anchor.lines).collect();
        assert_eq!(anchors, vec![LineRange::line(2), LineRange::FILE]);
    });
}

#[gpui_kit::test]
fn the_agent_focuses_highlights_and_notes_and_the_user_keeps_the_note_as_a_draft(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    let events = record_events(cx, &pane);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    play(cx, window, ask_about_refunds());
    // The agent thinks for a moment; the user has stopped typing.
    cx.executor().advance_clock(QUIET);

    let focus = review_focus("call-1", "src/refund.rs", 3, 4, "partial capture");
    let outcome = pane.update(cx, |pane, cx| pane.agent_action(session("pr"), focus, cx));
    assert_eq!(outcome, Ok(ActionOutcome::Applied));
    let note = review_note("call-2", "src/refund.rs", 3, "This rounds down; refunds lose a cent.");
    let outcome = pane.update(cx, |pane, cx| pane.agent_action(session("pr"), note, cx));
    assert_eq!(outcome, Ok(ActionOutcome::Applied));
    cx.run_until_parked();
    let spot = pane.read_with(cx, |pane, _| pane.context(&session("pr"))).expect("open");
    assert_eq!(spot.current, Some(FileKey::new("app", "src/refund.rs")));
    assert_eq!(spot.selection.map(|s| s.lines), Some(LineRange::new(3, 4)));
    let highlighted = cx
        .update_window(window, |_, window, cx| {
            window.render_frame(cx);
            humata2_snapshot::view::capture(window)
        })
        .expect("open")
        .expect("readable");
    assert!(highlighted.contains("Highlighted: partial capture"), "the lines are marked");

    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.17", "Review a pull request, focus and note"))
            .snapshot()
            .press(button("Make note 1 a draft"))
            .press(button("Undo review_focus app/src/refund.rs:3-4"))
            .snapshot(),
    );
    read(cx, &pane, "pr", |review| {
        let drafts = review.book().github_drafts();
        assert_eq!(drafts.len(), 1, "the note is a draft for GitHub");
        let draft = review.book().comment(drafts[0]).expect("there");
        assert_eq!(draft.by.as_deref(), Some("Claude Code"));
        assert_eq!(draft.anchor.lines, LineRange::line(3));
        assert!(review.memory().agent.notes.is_empty());
        assert!(review.memory().agent.highlights.is_empty(), "undo took the highlight away");
    });
    assert_eq!(fakes.typed().len(), 1, "drafts for GitHub aren't sent to the chat");
    assert!(!events.borrow().iter().any(|e| matches!(e, PaneEvent::AgentDenied { .. })));
}

#[gpui_kit::test]
fn editing_the_agents_note_makes_it_the_users_draft(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    let note = review_note("call-1", "src/ledger.rs", 2, "Check the sign");
    pane.update(cx, |pane, cx| pane.agent_action(session("pr"), note, cx)).expect("applied");
    cx.run_until_parked();
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.17", "Review a pull request, edit a note"))
            .press(button("Edit note 1"))
            .type_text(" of refunds")
            .key("secondary-enter"),
    );
    read(cx, &pane, "pr", |review| {
        let drafts = review.book().github_drafts();
        assert_eq!(drafts.len(), 1);
        assert_eq!(review.book().comment(drafts[0]).map(|c| c.body.as_str()), Some("Check the sign of refunds"));
        assert!(review.memory().agent.notes.is_empty());
    });
}

#[gpui_kit::test]
fn the_agent_waits_while_the_user_scrolls_and_moves_the_view_once_they_stop(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, 360.);
    let events = record_events(cx, &pane);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    let scroll = |cx: &mut TestAppContext| {
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.scroll("body", ScrollDelta::Pixels(point(px(0.), px(-60.))), cx);
        })
        .expect("open");
        cx.run_until_parked();
    };
    scroll(cx);
    let focus = review_focus("call-1", "src/util.rs", 2, 2, "here");
    let outcome = pane.update(cx, |pane, cx| pane.agent_action(session("pr"), focus, cx));
    assert_eq!(outcome, Ok(ActionOutcome::Waiting));
    let current = |cx: &mut TestAppContext| read(cx, &pane, "pr", |review| review.memory().current.clone());
    assert_ne!(current(cx), Some(FileKey::new("app", "src/util.rs")));

    // Still scrolling, before the quiet window ends: it keeps waiting.
    cx.executor().advance_clock(Duration::from_millis(500));
    scroll(cx);
    cx.executor().advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    assert_ne!(current(cx), Some(FileKey::new("app", "src/util.rs")), "the user is still scrolling");

    cx.executor().advance_clock(QUIET);
    cx.run_until_parked();
    assert_eq!(current(cx), Some(FileKey::new("app", "src/util.rs")));
    assert!(events.borrow().iter().any(|e| matches!(e, PaneEvent::AgentApplied { id, .. } if id == "call-1")));

    // Back returns to where the user had scrolled to.
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.17", "Review a pull request, back"))
            .key("b"),
    );
    assert_ne!(current(cx), Some(FileKey::new("app", "src/util.rs")));
}

#[gpui_kit::test]
fn a_switch_turned_off_answers_the_agent_with_a_denial(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.17", "Review a pull request, turn notes off"))
            .press(button("Agent may Notes"))
            .snapshot(),
    );
    let note = review_note("call-1", "src/refund.rs", 3, "Nope");
    let outcome = pane.update(cx, |pane, cx| pane.agent_action(session("pr"), note, cx));
    assert_eq!(outcome, Err(ActionError::TurnedOff { switch: Switch::Notes }));
    read(cx, &pane, "pr", |review| assert!(review.memory().agent.notes.is_empty()));

    // The switch is the session's, and a restart keeps it.
    let world = fakes.killed_at(fakes.log().len());
    let (_, pane) = open_pane(cx, &world, WIDTH, HEIGHT);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    let note = review_note("call-2", "src/refund.rs", 3, "Still no");
    let outcome = pane.update(cx, |pane, cx| pane.agent_action(session("pr"), note, cx));
    assert_eq!(outcome, Err(ActionError::TurnedOff { switch: Switch::Notes }));
    let focus = review_focus("call-3", "src/refund.rs", 3, 3, "ok");
    let outcome = pane.update(cx, |pane, cx| pane.agent_action(session("pr"), focus, cx));
    assert_eq!(outcome, Ok(ActionOutcome::Applied), "the other switches stay on");
}

#[gpui_kit::test]
fn the_agents_notes_highlights_and_order_survive_a_restart(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (_, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    pane.update(cx, |pane, cx| {
        let focus = review_focus("call-1", "src/ledger.rs", 2, 3, "the write");
        let note = review_note("call-2", "src/ledger.rs", 2, "Twice?");
        let order = humata2_review_harness::action("call-3", humata2_review_view::model::Tool::Prepare(prepared()));
        for action in [focus, note, order] {
            assert_eq!(pane.agent_action(session("pr"), action, cx), Ok(ActionOutcome::Applied));
        }
    });
    cx.run_until_parked();
    let before = read(cx, &pane, "pr", |review| review.memory().clone());

    let world = fakes.killed_at(fakes.log().len());
    let (_, pane) = open_pane(cx, &world, WIDTH, HEIGHT);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    let after = read(cx, &pane, "pr", |review| review.memory().clone());
    assert_eq!(after.agent.notes, before.agent.notes);
    assert_eq!(after.agent.highlights, before.agent.highlights);
    assert_eq!(after.prepared, before.prepared);
    // Undo still works after the restart.
    pane.update(cx, |pane, cx| pane.undo_agent(session("pr"), "call-3", cx)).expect("undone");
    read(cx, &pane, "pr", |review| assert!(review.memory().prepared.is_none()));
}

// 26.12: an agent-prepared review, then Revert.

#[gpui_kit::test]
fn reverting_a_prepared_review_puts_the_layout_back_and_keeps_the_comments(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    let events = record_events(cx, &pane);
    show(cx, &pane, "pr", pr_review(), AgentState::Working);
    pane.update(cx, |pane, cx| pane.open_prepared(session("pr"), prepared(), cx));
    cx.run_until_parked();
    read(cx, &pane, "pr", |review| {
        assert_eq!(review.memory().current, Some(FileKey::new("app", "src/ledger.rs")), "step 1 first");
        assert_eq!(review.memory().agent.notes.len(), 1);
    });
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.12", "Opening an agent-prepared review, then Revert"))
            .snapshot()
            .press(line("app/src/refund.rs line 5"))
            .key("c")
            .type_text("Name this")
            .key("secondary-enter")
            .key("shift-s")
            .key("enter")
            .press(button("Revert"))
            .snapshot(),
    );
    read(cx, &pane, "pr", |review| {
        assert!(review.memory().prepared.is_none());
        assert!(review.memory().agent.notes.is_empty(), "its notes go with it");
        assert_eq!(review.book().comments()[0].status, Status::Queued, "the comment stays queued");
    });
    assert!(events.borrow().iter().any(|e| matches!(e, PaneEvent::Reverted { view, .. } if view == "view-1")));
}

#[gpui_kit::test]
fn keeping_a_prepared_review_leaves_it_as_it_is(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    let events = record_events(cx, &pane);
    show(cx, &pane, "pr", pr_review(), AgentState::Idle);
    pane.update(cx, |pane, cx| pane.open_prepared(session("pr"), prepared(), cx));
    cx.run_until_parked();
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.12", "Opening an agent-prepared review, then Keep")).press(button("Keep")),
    );
    read(cx, &pane, "pr", |review| {
        let showing = review.memory().prepared.as_ref().expect("its order stays");
        assert!(showing.before.is_none(), "there's nothing to revert to any more");
        assert_eq!(review.memory().agent.notes.len(), 1, "its notes stay");
    });
    assert!(events.borrow().iter().any(|e| matches!(e, PaneEvent::Kept { .. })));
}

// RV-28–RV-30: compare with an earlier review, and mark reviewed.

#[gpui_kit::test]
fn comparing_with_a_review_shows_only_what_changed_since_and_mark_reviewed_adds_one(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let then = {
        let now = small_review();
        let files = now.files().iter().map(|file| {
            if file.key == FileKey::new("app", "src/lib.rs") { lib_rs("lib-1") } else { file.clone() }
        });
        Review::new(files.collect())
    };
    fakes.add_reviewed("Tuesday 14:02", then);
    fakes.set_now(small_review());
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "s1", small_review(), AgentState::Idle);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments, since review 1"))
            .press(button("Choose base"))
            .press(button("Since review 1, Tuesday 14:02"))
            .snapshot(),
    );
    read(cx, &pane, "s1", |review| {
        assert_eq!(review.memory().base, Base::Since(1));
        let files: Vec<_> = review.review().files().iter().map(|f| f.key.to_string()).collect();
        assert_eq!(files, vec!["app/src/lib.rs"]);
    });

    // The daemon sends the session's changes again; the pick holds.
    pane.update(cx, |pane, cx| pane.set_review(session("s1"), small_review(), cx));
    cx.run_until_parked();
    read(cx, &pane, "s1", |review| assert_eq!(review.review().files().len(), 1));

    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments, mark reviewed"))
            .press(button("Mark reviewed"))
            .press(button("Choose base"))
            .press(button("Since review 2, Today 14:02")),
    );
    read(cx, &pane, "s1", |review| {
        assert_eq!(review.memory().base, Base::Since(2));
        assert!(review.review().files().is_empty(), "nothing changed since review 2");
    });
    // A restart keeps the pick.
    let world = fakes.killed_at(fakes.log().len());
    let (_, pane) = open_pane(cx, &world, WIDTH, HEIGHT);
    show(cx, &pane, "s1", small_review(), AgentState::Idle);
    read(cx, &pane, "s1", |review| assert_eq!(review.memory().base, Base::Since(2)));
}

// Keys.

#[gpui_kit::test]
fn the_keyboard_reaches_every_step_of_a_review(cx: &mut TestAppContext) {
    let fakes = Fakes::new();
    let (window, pane) = open_pane(cx, &fakes, WIDTH, HEIGHT);
    show(cx, &pane, "s1", two_repos(), AgentState::Idle);
    play(
        cx,
        window,
        Flow::new(SpecFlow::new("26.3", "Review a 3-repo diff and send comments, by keyboard"))
            .key("n")
            .key("j")
            .key("shift-j")
            .key("c")
            .type_text("By keys")
            .key("secondary-enter")
            .key("[")
            .key("s")
            .key("enter")
            .key("v")
            .key("x"),
    );
    assert_eq!(fakes.typed().len(), 1);
    read(cx, &pane, "s1", |review| {
        let comment = &review.book().comments()[0];
        assert_eq!(comment.anchor.lines, LineRange::new(1, 2), "shift-j extended the selection");
        assert_eq!(comment.status, Status::Sent);
        let lib = FileKey::new("app", "src/lib.rs");
        assert_eq!(review.memory().viewed(&lib, "lib-2"), humata2_review_view::model::Viewed::Yes);
    });
}
