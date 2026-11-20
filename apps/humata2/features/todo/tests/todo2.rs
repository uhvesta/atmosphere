//! Scoped TDD tests for TODO2 features in `features/todo`.
#![allow(clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use humata2_clients::todo::{BlockerGoneResolution, Call, FakeTodo};
use humata2_todo::{
    Ask, AskKind, BlockerGone, CheckpointState, Day, Effort, FuzzyDue, Impact, LifecyclePrompt,
    Need, OwnerKind, RunEntry, SortBy, SourceAnchor, State, Step, StuckAgent, Todo as Item,
    TodoOwner,
};
use humata2_todo_feature::{
    FilterChip, Todo, TodoAction as A, TodoState, View, ViewFilter,
};
use tca::TestStore;

fn today() -> Day {
    Day::from_ymd(2026, 11, 19).expect("a date")
}

fn store(fake: &FakeTodo) -> TestStore<Todo> {
    TestStore::new(
        TodoState::new(View::default(), today()),
        Todo::new(Arc::new(fake.clone()), today),
    )
    .non_exhaustive()
}

fn loaded(store: &mut TestStore<Todo>) {
    store.send(A::Load, |_| {});
    store.receive_matching("Loaded", |a| matches!(a, A::Loaded(..)), |_| {});
}

fn item<'a>(state: &'a TodoState, key: &str) -> &'a Item {
    &state
        .items
        .iter()
        .find(|e| e.todo.key == key)
        .expect("item listed")
        .todo
}

#[test]
fn sort_by_switcher_reorders_next_temporarily() {
    let fake = FakeTodo::new();
    let mut a = Item::new("A-1", "Small nice", today().add_days(-2));
    a.effort = Some(Effort::S);
    a.impact = Some(Impact::Low);
    a.need = Some(Need::NiceToHave);
    a.due = Some(today().add_days(10));

    let mut b = Item::new("B-1", "Large critical", today().add_days(-2));
    b.effort = Some(Effort::L);
    b.impact = Some(Impact::High);
    b.need = Some(Need::Critical);
    b.due = Some(today().add_days(2));

    fake.insert(a, None);
    fake.insert(b, None);

    let mut store = store(&fake);
    loaded(&mut store);
    assert_eq!(store.state().sort, SortBy::Priority);

    store.send(A::SetSort(SortBy::SmallestEffort), |_| {});
    assert_eq!(store.state().sort, SortBy::SmallestEffort);
    assert_eq!(store.state().ranked().next[0].key, "A-1");

    store.send(A::SetSort(SortBy::MostNeeded), |_| {});
    assert_eq!(store.state().ranked().next[0].key, "B-1");

    store.send(A::SetSort(SortBy::DueSoonest), |_| {});
    assert_eq!(store.state().ranked().next[0].key, "B-1");

    store.send(A::SetSort(SortBy::LargestImpact), |_| {});
    assert_eq!(store.state().ranked().next[0].key, "B-1");
}

#[test]
fn keyboard_triage_focus_drawer_stepping_and_shortcuts() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    let order = store.state().visible_order();
    assert!(order.len() >= 3);

    // MoveFocus initializes to first visible row and steps down/up.
    store.send(A::MoveFocus(1), |_| {});
    assert_eq!(store.state().focused.as_deref(), Some(order[0].as_str()));
    store.send(A::MoveFocus(1), |_| {});
    assert_eq!(store.state().focused.as_deref(), Some(order[1].as_str()));
    store.send(A::MoveFocus(-1), |_| {});
    assert_eq!(store.state().focused.as_deref(), Some(order[0].as_str()));

    // StepDrawer steps through items without closing the drawer.
    store.send(A::Open(order[0].clone()), |_| {});
    assert_eq!(store.state().open.as_deref(), Some(order[0].as_str()));
    store.send(A::StepDrawer(1), |_| {});
    assert_eq!(store.state().open.as_deref(), Some(order[1].as_str()));
    assert_eq!(store.state().focused.as_deref(), Some(order[1].as_str()));
    store.send(A::StepDrawer(-1), |_| {});
    assert_eq!(store.state().open.as_deref(), Some(order[0].as_str()));

    // Focus PAY-5 and start it, then toggle it done.
    store.send(
        A::SelectRow {
            key: "PAY-5".to_owned(),
            shift: false,
            toggle: false,
        },
        |_| {},
    );
    store.send(A::StartOrNextFocused, |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "PAY-5").state, State::InProgress);

    store.send(A::ToggleFocusDone, |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "PAY-5").state, State::Closed);
}

#[test]
fn bulk_selection_and_partial_failure_keeps_only_refused_keys_selected() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    // Select PAY-5 and TD-3.
    store.send(
        A::SelectRow {
            key: "PAY-5".to_owned(),
            shift: false,
            toggle: false,
        },
        |_| {},
    );
    store.send(
        A::SelectRow {
            key: "TD-3".to_owned(),
            shift: false,
            toggle: true,
        },
        |_| {},
    );
    assert!(store.state().selected.contains("PAY-5"));
    assert!(store.state().selected.contains("TD-3"));

    // Make TD-3 stale on the host so bulk_update refuses ONLY TD-3.
    fake.touch("TD-3");

    store.send(A::BulkSetState(State::WontDo), |_| {});
    store.receive_matching("BulkUpdated", |a| matches!(a, A::BulkUpdated { .. }), |_| {});

    // PAY-5 succeeded and is deselected; TD-3 failed and stays selected with error!
    assert_eq!(item(store.state(), "PAY-5").state, State::WontDo);
    assert_eq!(item(store.state(), "TD-3").state, State::Open);
    assert!(!store.state().selected.contains("PAY-5"));
    assert!(store.state().selected.contains("TD-3"));
    assert_eq!(store.state().selected.len(), 1);
    assert!(store.state().bulk_errors.contains_key("TD-3"));
    assert!(!store.state().bulk_errors.contains_key("PAY-5"));

    // ClearSelection empties selection and errors.
    store.send(A::ClearSelection, |_| {});
    assert!(store.state().selected.is_empty());
    assert!(store.state().bulk_errors.is_empty());
}

#[test]
fn bulk_range_selection_and_move_channel() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    let next_keys: Vec<String> = store
        .state()
        .ranked()
        .next
        .iter()
        .map(|r| r.key.clone())
        .collect();
    assert!(next_keys.len() >= 3);

    store.send(
        A::SelectRow {
            key: next_keys[0].clone(),
            shift: false,
            toggle: false,
        },
        |_| {},
    );
    store.send(A::ExtendSelection(2), |_| {});
    assert_eq!(store.state().selected.len(), 3);
    assert!(store.state().selected.contains(&next_keys[0]));
    assert!(store.state().selected.contains(&next_keys[1]));
    assert!(store.state().selected.contains(&next_keys[2]));

    store.send(A::BulkMoveChannel(Some("ops".to_owned())), |_| {});
    store.receive_matching("BulkUpdated", |a| matches!(a, A::BulkUpdated { .. }), |_| {});
    assert!(store.state().selected.is_empty());
    assert_eq!(item(store.state(), &next_keys[0]).channel.as_deref(), Some("ops"));
    assert_eq!(item(store.state(), &next_keys[1]).channel.as_deref(), Some("ops"));
    assert_eq!(item(store.state(), &next_keys[2]).channel.as_deref(), Some("ops"));
}

#[test]
fn undo_offers_start_next_only_when_no_other_item_is_in_progress() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    // WEB-8 is InProgress in frame_b1_1. Closing PAY-5 must NOT offer Start next.
    store.send(A::SetState("PAY-5".to_owned(), State::Closed), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    let toast = store.state().undo.clone().expect("undo toast set");
    assert_eq!(toast.key, "PAY-5");
    assert!(toast.start_next.is_none());

    // Closing WEB-8 (the only InProgress item) MUST offer Start next (top Next = INF-6).
    store.send(A::SetState("WEB-8".to_owned(), State::Closed), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    let toast = store.state().undo.clone().expect("undo toast set");
    assert_eq!(toast.key, "WEB-8");
    let offer = toast.start_next.expect("start_next offered when nothing in progress");
    assert_eq!(offer.key, "INF-6");

    // Undo restores WEB-8 to InProgress.
    store.send(A::Undo, |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "WEB-8").state, State::InProgress);
    assert!(store.state().undo.is_none());

    // Close WEB-8 again and accept StartNextFromUndo -> INF-6 becomes InProgress.
    store.send(A::SetState("WEB-8".to_owned(), State::Closed), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    store.send(A::StartNextFromUndo, |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "INF-6").state, State::InProgress);
    assert!(store.state().undo.is_none());
}

#[test]
fn tag_chip_toggle_and_removable_filter_chips() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    store.send(A::ToggleTagChip("bug".to_owned()), |_| {});
    assert_eq!(store.state().view.filter.tags, vec!["bug"]);
    assert!(store
        .state()
        .view
        .filter
        .active_chips()
        .contains(&FilterChip::Tag("bug".to_owned())));

    // Toggling again removes it.
    store.send(A::ToggleTagChip("BUG".to_owned()), |_| {});
    assert!(store.state().view.filter.tags.is_empty());

    // RemoveFilterChip removes a specific active filter.
    store.send(
        A::SetFilter(ViewFilter {
            need: vec![Need::Critical],
            include_terminal: true,
            search: "backoff".to_owned(),
            ..ViewFilter::default()
        }),
        |_| {},
    );
    store.send(A::RemoveFilterChip(FilterChip::Need(Need::Critical)), |_| {});
    assert!(store.state().view.filter.need.is_empty());
    assert!(store.state().view.filter.include_terminal);

    store.send(A::RemoveFilterChip(FilterChip::IncludeTerminal), |_| {});
    assert!(!store.state().view.filter.include_terminal);

    store.send(A::RemoveFilterChip(FilterChip::Search("backoff".to_owned())), |_| {});
    assert!(store.state().view.filter.search.is_empty());
}

#[test]
fn need_fuzzy_due_external_block_comments_steps_and_checkpoint() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    // Need & FuzzyDue.
    store.send(A::SetNeed("PAY-5".to_owned(), Some(Need::Critical)), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "PAY-5").need, Some(Need::Critical));

    store.send(
        A::SetFuzzyDue("PAY-5".to_owned(), Some(FuzzyDue::NextWeek)),
        |_| {},
    );
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "PAY-5").fuzzy_due, Some(FuzzyDue::NextWeek));
    assert_eq!(item(store.state(), "PAY-5").due, None);

    // External block add / remove.
    store.send(
        A::AddExternalBlock("PAY-5".to_owned(), "Waiting on legal".to_owned()),
        |_| {},
    );
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(
        item(store.state(), "PAY-5")
            .external_block
            .as_ref()
            .map(|b| b.reason.as_str()),
        Some("Waiting on legal")
    );

    store.send(A::RemoveExternalBlock("PAY-5".to_owned()), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert!(item(store.state(), "PAY-5").external_block.is_none());

    // Comment.
    store.send(
        A::AddComment {
            key: "PAY-5".to_owned(),
            body: "Checked with billing".to_owned(),
            step_id: None,
        },
        |_| {},
    );
    store.receive_matching("CommentAdded", |a| matches!(a, A::CommentAdded { .. }), |_| {});
    assert_eq!(item(store.state(), "PAY-5").comments.len(), 1);
    assert_eq!(item(store.state(), "PAY-5").comments[0].body, "Checked with billing");

    // Step markdown, move, drop, restore.
    let mut with_steps = Item::new("STP-1", "Steps item", today());
    with_steps.steps = vec![Step::new("First"), Step::new("Second")];
    fake.insert(with_steps, None);
    loaded(&mut store);

    store.send(
        A::SetStepMarkdown("STP-1".to_owned(), 0, Some("## Details".to_owned())),
        |_| {},
    );
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(
        item(store.state(), "STP-1").steps[0].markdown_context.as_deref(),
        Some("## Details")
    );

    store.send(A::MoveStep("STP-1".to_owned(), 0, 1), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "STP-1").steps[0].text, "Second");
    assert_eq!(item(store.state(), "STP-1").steps[1].text, "First");

    store.send(A::DropStep("STP-1".to_owned(), 0), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert!(item(store.state(), "STP-1").steps[0].dropped);

    store.send(A::RestoreStep("STP-1".to_owned(), 0), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert!(!item(store.state(), "STP-1").steps[0].dropped);

    // Checkpoint.
    store.send(
        A::SetCheckpoint("STP-1".to_owned(), Some(CheckpointState::Defined)),
        |_| {},
    );
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(item(store.state(), "STP-1").checkpoint, Some(CheckpointState::Defined));
}

#[test]
fn split_fold_history_and_runs() {
    let fake = FakeTodo::frame_b1_1();
    fake.add_run(
        "PAY-5",
        RunEntry {
            id: "run-1".to_owned(),
            tool: "claude".to_owned(),
            model: "sonnet".to_owned(),
            outcome: "ok".to_owned(),
            cost_micros: 120_000,
            ..RunEntry::default()
        },
    );
    let mut store = store(&fake);
    loaded(&mut store);

    // Split PAY-5 into two tasks.
    store.send(
        A::SplitItem {
            key: "PAY-5".to_owned(),
            titles: vec!["Part A".to_owned(), "Part B".to_owned()],
            keep_age: true,
        },
        |_| {},
    );
    store.receive_matching("ItemSplit", |a| matches!(a, A::ItemSplit(_)), |_| {});
    assert_eq!(item(store.state(), "PAY-5").state, State::Became);
    assert_eq!(item(store.state(), "PAY-5").became_keys.len(), 2);

    // Fold TD-3 into INF-6.
    store.send(
        A::FoldIntoStep {
            source_key: "TD-3".to_owned(),
            target_key: "INF-6".to_owned(),
        },
        |_| {},
    );
    store.receive_matching("ItemFolded", |a| matches!(a, A::ItemFolded(_)), |_| {});
    assert_eq!(item(store.state(), "TD-3").state, State::Became);
    assert!(item(store.state(), "INF-6").from_keys.contains(&"TD-3".to_owned()));

    // LoadHistory and LoadRuns.
    store.send(A::LoadHistory("TD-3".to_owned()), |_| {});
    store.receive_matching("HistoryLoaded", |a| matches!(a, A::HistoryLoaded { .. }), |_| {});
    assert!(!store.state().history.get("TD-3").expect("history loaded").is_empty());

    store.send(A::LoadRuns("PAY-5".to_owned()), |_| {});
    store.receive_matching("RunsLoaded", |a| matches!(a, A::RunsLoaded { .. }), |_| {});
    let runs = store.state().runs.get("PAY-5").expect("runs loaded");
    assert_eq!(runs.runs.len(), 1);
    assert_eq!(runs.total_cost_micros, 120_000);
}

#[test]
fn structured_asks_stuck_take_back_blocker_gone_and_lifecycle_prompts() {
    let fake = FakeTodo::new();

    let mut q = Item::new("Q-1", "Question item", today());
    q.ask = Some(Ask {
        kind: AskKind::Question {
            question: "Which DB?".to_owned(),
            choices: vec!["Postgres".to_owned(), "SQLite".to_owned()],
            multi_select: false,
            step_id: None,
            details: None,
            editable: true,
        },
        by: "claude".to_owned(),
        reason: "need choice".to_owned(),
    });
    fake.insert(q, Some("ask-q1"));

    let mut act = Item::new("ACT-1", "Action item", today());
    act.ask = Some(Ask {
        kind: AskKind::ActionRequest {
            action: "Rotate prod key".to_owned(),
            step_id: None,
            details: None,
        },
        by: "claude".to_owned(),
        reason: "requires manual access".to_owned(),
    });
    fake.insert(act, Some("ask-act1"));

    let mut rev = Item::new("REV-9", "Review item", today());
    rev.checkpoint = Some(CheckpointState::Defined);
    rev.ask = Some(Ask {
        kind: AskKind::ReviewRequest {
            summary: "Ready for review".to_owned(),
            round: 1,
            step_id: None,
            details: None,
            checkpoint: Some(CheckpointState::Done),
        },
        by: "claude".to_owned(),
        reason: "round 1".to_owned(),
    });
    fake.insert(rev, Some("ask-rev9"));

    let mut stuck = Item::new("STK-1", "Stuck item", today());
    stuck.owner = Some(TodoOwner {
        kind: OwnerKind::Agent,
        label: "claude".to_owned(),
        session: Some("s-1".to_owned()),
    });
    stuck.stuck_agent = Some(StuckAgent {
        session: "s-1".to_owned(),
        agent_label: "claude".to_owned(),
        reason: "rate limited".to_owned(),
        since_minutes: 15,
    });
    stuck.blocker_gone.push(BlockerGone {
        blocker_key: "OLD-1".to_owned(),
        blocker_state: State::WontDo,
        replacement_keys: Vec::new(),
    });
    stuck.depends_on.push("OLD-1".to_owned());
    stuck.lifecycle_prompts.push(LifecyclePrompt {
        id: "lp-1".to_owned(),
        kind: "pr_merged".to_owned(),
        title: "PR merged".to_owned(),
        detail: "Close item?".to_owned(),
        choices: vec!["close".to_owned(), "keep".to_owned()],
        pr_ref: Some("acme/api#99".to_owned()),
        session: None,
    });
    fake.insert(stuck, None);

    let mut store = store(&fake);
    loaded(&mut store);

    // Editable open question draft + submit.
    store.send(
        A::ToggleQuestionChoice {
            ask_id: "ask-q1".to_owned(),
            choice: "Postgres".to_owned(),
            multi_select: false,
        },
        |_| {},
    );
    store.send(
        A::UpdateQuestionDraft {
            ask_id: "ask-q1".to_owned(),
            text: "Use primary".to_owned(),
            selected_choices: vec!["Postgres".to_owned()],
        },
        |_| {},
    );
    assert_eq!(
        store.state().question_drafts.get("ask-q1").map(|d| d.text.as_str()),
        Some("Use primary")
    );
    store.send(
        A::AnswerQuestion {
            ask_id: "ask-q1".to_owned(),
            text: "Use primary".to_owned(),
            selected_choices: vec!["Postgres".to_owned()],
        },
        |_| {},
    );
    store.receive_matching("Answered", |a| matches!(a, A::Answered { .. }), |_| {});
    store.receive_matching("Loaded", |a| matches!(a, A::Loaded(..)), |_| {});
    assert!(item(store.state(), "Q-1").ask.is_none());
    assert!(!store.state().question_drafts.contains_key("ask-q1"));

    // ActionRequest.
    store.send(
        A::AnswerAction {
            ask_id: "ask-act1".to_owned(),
            done: true,
            note: "Rotated".to_owned(),
        },
        |_| {},
    );
    store.receive_matching("Answered", |a| matches!(a, A::Answered { .. }), |_| {});
    store.receive_matching("Loaded", |a| matches!(a, A::Loaded(..)), |_| {});
    assert!(item(store.state(), "ACT-1").ask.is_none());

    // ReviewRequest.
    store.send(
        A::AnswerReview {
            ask_id: "ask-rev9".to_owned(),
            approved: true,
            note: "LGTM".to_owned(),
        },
        |_| {},
    );
    store.receive_matching("Answered", |a| matches!(a, A::Answered { .. }), |_| {});
    store.receive_matching("Loaded", |a| matches!(a, A::Loaded(..)), |_| {});
    assert!(item(store.state(), "REV-9").ask.is_none());
    assert_eq!(item(store.state(), "REV-9").checkpoint, Some(CheckpointState::Done));

    // RetryStuck.
    store.send(A::RetryStuck("STK-1".to_owned()), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert!(item(store.state(), "STK-1").stuck_agent.is_none());

    // RequestTakeBack -> ConfirmTakeBack.
    store.send(A::RequestTakeBack("STK-1".to_owned()), |_| {});
    assert_eq!(store.state().confirm_take_back.as_deref(), Some("STK-1"));
    store.send(A::ConfirmTakeBack("STK-1".to_owned()), |_| {});
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert_eq!(store.state().confirm_take_back, None);
    assert_eq!(
        item(store.state(), "STK-1").owner.as_ref().map(|o| o.kind),
        Some(OwnerKind::User)
    );

    // ResolveBlockerGone.
    store.send(
        A::ResolveBlockerGone {
            key: "STK-1".to_owned(),
            blocker_key: "OLD-1".to_owned(),
            resolution: BlockerGoneResolution::Unblock,
        },
        |_| {},
    );
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert!(item(store.state(), "STK-1").blocker_gone.is_empty());
    assert!(item(store.state(), "STK-1").depends_on.is_empty());

    // ResolveLifecyclePrompt.
    store.send(
        A::ResolveLifecyclePrompt {
            key: "STK-1".to_owned(),
            prompt_id: "lp-1".to_owned(),
            choice: "close".to_owned(),
        },
        |_| {},
    );
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    assert!(item(store.state(), "STK-1").lifecycle_prompts.is_empty());
    assert_eq!(item(store.state(), "STK-1").state, State::Closed);
}

#[test]
fn diff_and_markdown_comment_hooks() {
    let fake = FakeTodo::frame_b1_1();
    let mut store = store(&fake);
    loaded(&mut store);

    let anchor = SourceAnchor {
        repo: "acme/api".to_owned(),
        path: "src/ledger.rs".to_owned(),
        line: 42,
        end_line: Some(48),
        comment_id: Some("c-9".to_owned()),
    };

    store.send(
        A::AddStepFromComment {
            key: "PAY-5".to_owned(),
            text: "Handle negative amounts".to_owned(),
            anchor: anchor.clone(),
        },
        |_| {},
    );
    store.receive_matching("Written", |a| matches!(a, A::Written { .. }), |_| {});
    let last_step = item(store.state(), "PAY-5").steps.last().expect("step added");
    assert_eq!(last_step.text, "Handle negative amounts");
    assert_eq!(last_step.source_anchor.as_ref(), Some(&anchor));

    store.send(
        A::SpinOffFromComment {
            from_key: Some("PAY-5".to_owned()),
            title: "Audit rounding mode".to_owned(),
            anchor: anchor.clone(),
            channel: Some("payments-rewrite".to_owned()),
        },
        |_| {},
    );
    store.receive_matching("Created", |a| matches!(a, A::Created(Ok(_))), |_| {});
    let created = store.state().items.last().expect("created").todo.clone();
    assert_eq!(created.title, "Audit rounding mode");
    assert_eq!(created.from_keys, vec!["PAY-5"]);
    assert_eq!(created.source_anchor.as_ref(), Some(&anchor));
    assert!(fake
        .calls()
        .iter()
        .any(|c| matches!(c, Call::Create(d) if d.title == "Audit rounding mode")));

    // NavigateAnchor is a delegate action (no state mutation).
    let before = store.state().items.clone();
    store.send(A::NavigateAnchor(anchor), |_| {});
    assert_eq!(store.state().items, before);
}
