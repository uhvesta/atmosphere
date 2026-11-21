//! Milestone detection, dependency-order resolution, and launch-brief
//! generation (ATM-8, ATM-9, ATM-11).
//!
//! Any item with a non-empty `depends_on` list is a milestone. Its steps are
//! the transitive closure of `depends_on` in topological dependency order
//! (dependencies before dependents), with cycle protection.

use std::collections::{BTreeSet, HashMap};

use humata2_clients::model::{Item, ItemId, ItemStatus, Workspace};
use humata2_clients::todo::Entry;
use humata2_todo::{State, Todo};

/// One step of a milestone, resolved from the workspace board or To-do list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MilestoneStep {
    /// Step identifier (board `ItemId` or To-do key).
    pub id: String,
    /// Step title; empty when the referenced item is missing.
    pub title: String,
    /// Human-readable status label.
    pub status: String,
    /// Optional longer description.
    pub description: String,
    /// Whether the step is finished (`ItemStatus::Done` or `State::Closed`).
    pub done: bool,
}

/// A resolved milestone and its transitive dependency steps in topological order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MilestoneInfo {
    /// Milestone identifier (board `ItemId` or To-do key).
    pub id: String,
    /// Milestone title.
    pub title: String,
    /// Milestone description.
    pub description: String,
    /// Steps in dependency order (dependencies before dependents).
    pub steps: Vec<MilestoneStep>,
}

impl MilestoneInfo {
    /// Open (not done) steps in dependency order.
    #[must_use]
    pub fn open_steps(&self) -> Vec<&MilestoneStep> {
        self.steps.iter().filter(|s| !s.done).collect()
    }

    /// Number of steps currently marked done.
    #[must_use]
    pub fn done_count(&self) -> usize {
        self.steps.iter().filter(|s| s.done).count()
    }

    /// Total number of steps in this milestone.
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.steps.len()
    }

    /// Generates the pre-filled launch brief for starting a session on this
    /// milestone (ATM-9).
    #[must_use]
    pub fn brief(&self) -> String {
        let mut sections = Vec::new();
        sections.push(format!("Milestone {}: {}", self.id, self.title));

        let desc = self.description.trim();
        if !desc.is_empty() {
            sections.push(desc.to_owned());
        }

        let open = self.open_steps();
        if open.is_empty() {
            sections.push("All steps are currently done.".to_owned());
        } else {
            let mut lines = Vec::with_capacity(open.len() + 1);
            lines.push("Open steps (in dependency order):".to_owned());
            for (idx, step) in open.iter().enumerate() {
                let num = idx + 1;
                if step.title.is_empty() {
                    lines.push(format!("{num}. {} [{}]", step.id, step.status));
                } else {
                    lines.push(format!("{num}. {} — {} [{}]", step.id, step.title, step.status));
                }
                let step_desc = step.description.trim();
                if !step_desc.is_empty() {
                    for line in step_desc.lines() {
                        lines.push(format!("   {line}"));
                    }
                }
            }
            sections.push(lines.join("\n"));
        }

        sections.push(
            "Work the steps in order, move each with humata board as you go. The milestone is done when every step is."
                .to_owned(),
        );
        sections.join("\n\n")
    }
}

/// Returns `true` when `item` is a milestone (has at least one dependency, ATM-8).
#[must_use]
pub fn is_workspace_milestone(item: &Item) -> bool {
    !item.depends_on.is_empty()
}

/// Resolves milestone metadata and dependency-ordered steps for a workspace
/// board item. Returns `None` if `id` is not found or is not a milestone.
#[must_use]
pub fn workspace_milestone(workspace: &Workspace, id: &ItemId) -> Option<MilestoneInfo> {
    let by_id: HashMap<&str, &Item> =
        workspace.items.iter().map(|item| (item.id.as_str(), item)).collect();
    let root = by_id.get(id.as_str()).copied()?;
    if !is_workspace_milestone(root) {
        return None;
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut order = Vec::new();
    visiting.insert(root.id.as_str().to_owned());

    for dep in &root.depends_on {
        dfs_workspace(dep.as_str(), &by_id, &mut visiting, &mut visited, &mut order);
    }

    let steps = order
        .into_iter()
        .map(|step_id| {
            if let Some(item) = by_id.get(step_id.as_str()) {
                let description = workspace
                    .item_details
                    .get(&item.id)
                    .map(|d| d.description.clone())
                    .unwrap_or_default();
                MilestoneStep {
                    id: item.id.as_str().to_owned(),
                    title: item.title.clone(),
                    status: item.status.name().to_owned(),
                    description,
                    done: item.status == ItemStatus::Done,
                }
            } else {
                MilestoneStep {
                    id: step_id,
                    title: String::new(),
                    status: "To do".to_owned(),
                    description: String::new(),
                    done: false,
                }
            }
        })
        .collect();

    let description = workspace
        .item_details
        .get(&root.id)
        .map(|d| d.description.clone())
        .unwrap_or_default();

    Some(MilestoneInfo {
        id: root.id.as_str().to_owned(),
        title: root.title.clone(),
        description,
        steps,
    })
}

fn dfs_workspace(
    id: &str,
    by_id: &HashMap<&str, &Item>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) {
    if visited.contains(id) || visiting.contains(id) {
        return;
    }
    visiting.insert(id.to_owned());
    if let Some(item) = by_id.get(id) {
        for dep in &item.depends_on {
            dfs_workspace(dep.as_str(), by_id, visiting, visited, order);
        }
    }
    visiting.remove(id);
    visited.insert(id.to_owned());
    order.push(id.to_owned());
}

/// Returns `true` when `todo` is a milestone (has at least one dependency, ATM-8).
#[must_use]
pub fn is_todo_milestone(todo: &Todo) -> bool {
    !todo.depends_on.is_empty()
}

/// Resolves milestone metadata and dependency-ordered steps for a To-do entry.
/// Returns `None` if `key` is not found or is not a milestone.
#[must_use]
pub fn todo_milestone(items: &[Entry], key: &str) -> Option<MilestoneInfo> {
    let by_key: HashMap<&str, &Todo> =
        items.iter().map(|entry| (entry.todo.key.as_str(), &entry.todo)).collect();
    let root = by_key.get(key).copied()?;
    if !is_todo_milestone(root) {
        return None;
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    let mut order = Vec::new();
    visiting.insert(root.key.clone());

    for dep in &root.depends_on {
        dfs_todo(dep.as_str(), &by_key, &mut visiting, &mut visited, &mut order);
    }

    let steps = order
        .into_iter()
        .map(|step_key| {
            if let Some(todo) = by_key.get(step_key.as_str()) {
                MilestoneStep {
                    id: todo.key.clone(),
                    title: todo.title.clone(),
                    status: todo_state_label(todo.state).to_owned(),
                    description: todo.description.clone(),
                    done: todo.state == State::Closed,
                }
            } else {
                MilestoneStep {
                    id: step_key,
                    title: String::new(),
                    status: "Open".to_owned(),
                    description: String::new(),
                    done: false,
                }
            }
        })
        .collect();

    Some(MilestoneInfo {
        id: root.key.clone(),
        title: root.title.clone(),
        description: root.description.clone(),
        steps,
    })
}

fn dfs_todo(
    key: &str,
    by_key: &HashMap<&str, &Todo>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) {
    if visited.contains(key) || visiting.contains(key) {
        return;
    }
    visiting.insert(key.to_owned());
    if let Some(todo) = by_key.get(key) {
        for dep in &todo.depends_on {
            dfs_todo(dep.as_str(), by_key, visiting, visited, order);
        }
    }
    visiting.remove(key);
    visited.insert(key.to_owned());
    order.push(key.to_owned());
}

const fn todo_state_label(state: State) -> &'static str {
    match state {
        State::Open => "Open",
        State::InProgress => "In progress",
        State::Closed => "Closed",
        State::WontDo => "Won't do",
        State::Became => "Became",
        State::WatchingPr => "Watching PR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use humata2_clients::model::{ItemDetails, ItemKind, Scope};
    use humata2_todo::Day;

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
        Entry {
            todo,
            revision: 1,
            ask_id: None,
        }
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
    fn workspace_milestone_topological_order_and_brief() {
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
            ItemDetails {
                description: "Expose POST /checkout".to_owned(),
                ..ItemDetails::default()
            },
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
        assert!(brief.ends_with(
            "Work the steps in order, move each with humata board as you go. The milestone is done when every step is."
        ));
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
    fn all_done_brief_says_all_steps_done() {
        let entries = vec![
            make_entry("M-1", "Done milestone", State::Open, &["S-1"], ""),
            make_entry("S-1", "Finished step", State::Closed, &[], ""),
        ];
        let info = todo_milestone(&entries, "M-1").expect("is milestone");
        assert_eq!(info.done_count(), 1);
        assert_eq!(info.total_count(), 1);
        assert!(info.open_steps().is_empty());
        assert!(info.brief().contains("All steps are currently done."));
    }
}
