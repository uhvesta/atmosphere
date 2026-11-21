//! ATM-8, ATM-9: milestone detection, topological dependency ordering, cycle
//! safety, Done-step omission, and launch-brief generation.

use humata2_clients::model::{Item, ItemDetails, ItemId, ItemKind, ItemStatus, Scope, Workspace};
use humata2_clients::todo::Entry;
use humata2_todo::{Day, State, Todo};
use humata2_todo_feature::{
    MilestoneStep, TodoState, View, is_todo_milestone, is_workspace_milestone, todo_milestone,
    workspace_milestone,
};

fn make_item(id: &str, title: &str, status: ItemStatus, depends_on: &[&str]) -> Item {
    Item {
        id: ItemId::new(id),
        scope: Scope::Global,
        kind: ItemKind::Task,
        title: title.to_owned(),
        status,
        depends_on: depends_on.iter().map(|d| ItemId::new(*d)).collect(),
        session: None,
        parent: None,
        revision: 0,
    }
}

fn make_entry(key: &str, title: &str, state: State, depends_on: &[&str], desc: &str) -> Entry {
    let mut todo = Todo::new(key, title, Day::from_ymd(2026, 11, 20).expect("valid day"));
    todo.state = state;
    todo.depends_on = depends_on.iter().map(|d| (*d).to_owned()).collect();
    todo.description = desc.to_owned();
    Entry { todo, revision: 1, ask_id: None }
}

#[test]
fn milestone_detection_requires_non_empty_depends_on() {
    let plain = make_item("M-1", "Plain task", ItemStatus::Todo, &[]);
    let milestone = make_item("M-2", "Milestone", ItemStatus::Todo, &["S-1"]);
    assert!(!is_workspace_milestone(&plain));
    assert!(is_workspace_milestone(&milestone));

    let plain_todo = make_entry("T-1", "Plain", State::Open, &[], "");
    let milestone_todo = make_entry("T-2", "Milestone", State::Open, &["T-1"], "");
    assert!(!is_todo_milestone(&plain_todo.todo));
    assert!(is_todo_milestone(&milestone_todo.todo));
}

#[test]
fn workspace_milestone_orders_steps_topologically_and_omits_done_in_brief() {
    let mut ws = Workspace::default();
    ws.items = vec![
        make_item("M-1", "Ship checkout", ItemStatus::Todo, &["S-2", "S-3"]),
        make_item("S-1", "Schema migration", ItemStatus::Done, &[]),
        make_item("S-2", "API endpoint", ItemStatus::InProgress, &["S-1"]),
        make_item("S-3", "UI wiring", ItemStatus::Todo, &["S-2"]),
    ];
    ws.item_details.insert(
        ItemId::new("M-1"),
        ItemDetails {
            description: "End-to-end checkout milestone.".to_owned(),
            ..ItemDetails::default()
        },
    );
    ws.item_details.insert(
        ItemId::new("S-2"),
        ItemDetails { description: "Expose POST /checkout".to_owned(), ..ItemDetails::default() },
    );

    let info = workspace_milestone(&ws, &ItemId::new("M-1")).expect("is milestone");
    let step_ids: Vec<&str> = info.steps.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(step_ids, vec!["S-1", "S-2", "S-3"]);
    assert_eq!(info.done_count(), 1);
    assert_eq!(info.total_count(), 3);
    assert_eq!(info.open_steps().len(), 2);

    let brief = info.brief();
    assert!(brief.starts_with("Milestone M-1: Ship checkout\n\nEnd-to-end checkout milestone.\n\n"));
    assert!(brief.contains("Open steps (in dependency order):"));
    assert!(brief.contains("1. S-2 — API endpoint [In progress]\n   Expose POST /checkout"));
    assert!(brief.contains("2. S-3 — UI wiring [To do]"));
    assert!(!brief.contains("S-1"), "Done steps are omitted from brief");
    assert!(brief.contains("humata board"));
}

#[test]
fn cycle_detection_terminates_cleanly() {
    let mut ws = Workspace::default();
    ws.items = vec![
        make_item("M-1", "Cyclic milestone", ItemStatus::Todo, &["A"]),
        make_item("A", "Step A", ItemStatus::Todo, &["B"]),
        make_item("B", "Step B", ItemStatus::Todo, &["A", "M-1"]),
    ];
    let info = workspace_milestone(&ws, &ItemId::new("M-1")).expect("is milestone");
    let step_ids: Vec<&str> = info.steps.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(step_ids, vec!["B", "A"]);
}

#[test]
fn missing_dependency_uses_fallback_step() {
    let mut ws = Workspace::default();
    ws.items = vec![make_item("M-1", "Milestone", ItemStatus::Todo, &["MISSING-1"])];
    let info = workspace_milestone(&ws, &ItemId::new("M-1")).expect("is milestone");
    assert_eq!(
        info.steps,
        vec![MilestoneStep {
            id: "MISSING-1".to_owned(),
            title: String::new(),
            status: "To do".to_owned(),
            description: String::new(),
            done: false,
        }]
    );
    assert!(info.brief().contains("1. MISSING-1 [To do]"));
}

#[test]
fn todo_state_milestone_helpers_resolve_steps_and_all_done_brief() {
    let day = Day::from_ymd(2026, 11, 20).expect("valid day");
    let mut state = TodoState::new(View::default(), day);
    state.items = vec![
        make_entry("M-1", "Done milestone", State::Open, &["S-1"], "Wrap up"),
        make_entry("S-1", "Finished step", State::Closed, &[], ""),
        make_entry("T-1", "Plain task", State::Open, &[], ""),
    ];
    assert!(state.is_milestone("M-1"));
    assert!(!state.is_milestone("T-1"));
    assert!(state.milestone("T-1").is_none());

    let info = state.milestone("M-1").expect("is milestone");
    assert_eq!(info, todo_milestone(&state.items, "M-1").expect("matches"));
    assert_eq!(info.done_count(), 1);
    assert_eq!(info.total_count(), 1);
    assert!(info.open_steps().is_empty());
    assert!(info.brief().contains("All steps are currently done."));
}
