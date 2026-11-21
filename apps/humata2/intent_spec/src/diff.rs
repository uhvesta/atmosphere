use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{IntentSpec, IntentStatus};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentDiff {
    pub outcome_changed: bool,
    pub status_changed: Option<(IntentStatus, IntentStatus)>,
    pub added_non_goals: Vec<String>,
    pub removed_non_goals: Vec<String>,
    pub added_invariants: Vec<String>,
    pub removed_invariants: Vec<String>,
    pub added_forbidden_actions: Vec<String>,
    pub removed_forbidden_actions: Vec<String>,
    pub added_approval_boundaries: Vec<String>,
    pub removed_approval_boundaries: Vec<String>,
    pub added_steps: Vec<String>,
    pub removed_steps: Vec<String>,
    pub modified_steps: Vec<String>,
    pub added_acceptance: Vec<String>,
    pub removed_acceptance: Vec<String>,
    pub modified_acceptance: Vec<String>,
    pub resolved_questions: Vec<String>,
    pub added_questions: Vec<String>,
}

impl IntentDiff {
    pub fn compute(before: &IntentSpec, after: &IntentSpec) -> Self {
        let outcome_changed = before.outcome != after.outcome;
        let status_changed = if before.metadata.status != after.metadata.status {
            Some((before.metadata.status, after.metadata.status))
        } else {
            None
        };

        let (added_non_goals, removed_non_goals) =
            diff_string_lists(&before.non_goals, &after.non_goals);
        let (added_invariants, removed_invariants) =
            diff_string_lists(&before.invariants, &after.invariants);
        let (added_forbidden_actions, removed_forbidden_actions) = diff_string_lists(
            &before.constraints.forbidden_actions,
            &after.constraints.forbidden_actions,
        );
        let (added_approval_boundaries, removed_approval_boundaries) = diff_string_lists(
            &before.constraints.approval_boundaries,
            &after.constraints.approval_boundaries,
        );

        let before_steps: HashMap<&str, _> = before
            .steps
            .iter()
            .map(|s| (s.id.as_str(), s))
            .collect();
        let after_steps: HashMap<&str, _> = after
            .steps
            .iter()
            .map(|s| (s.id.as_str(), s))
            .collect();

        let mut added_steps = Vec::new();
        let mut modified_steps = Vec::new();
        for s in &after.steps {
            match before_steps.get(s.id.as_str()) {
                None => added_steps.push(s.id.clone()),
                Some(&prev) if prev != s => modified_steps.push(s.id.clone()),
                Some(_) => {}
            }
        }
        let mut removed_steps = Vec::new();
        for s in &before.steps {
            if !after_steps.contains_key(s.id.as_str()) {
                removed_steps.push(s.id.clone());
            }
        }

        let before_acc: HashMap<&str, _> = before
            .acceptance
            .iter()
            .map(|a| (a.id.as_str(), a))
            .collect();
        let after_acc: HashMap<&str, _> = after
            .acceptance
            .iter()
            .map(|a| (a.id.as_str(), a))
            .collect();

        let mut added_acceptance = Vec::new();
        let mut modified_acceptance = Vec::new();
        for a in &after.acceptance {
            match before_acc.get(a.id.as_str()) {
                None => added_acceptance.push(a.id.clone()),
                Some(&prev) if prev != a => modified_acceptance.push(a.id.clone()),
                Some(_) => {}
            }
        }
        let mut removed_acceptance = Vec::new();
        for a in &before.acceptance {
            if !after_acc.contains_key(a.id.as_str()) {
                removed_acceptance.push(a.id.clone());
            }
        }

        let before_q: HashMap<&str, _> = before
            .open_questions
            .iter()
            .map(|q| (q.id.as_str(), q))
            .collect();
        let mut added_questions = Vec::new();
        let mut resolved_questions = Vec::new();
        for q in &after.open_questions {
            match before_q.get(q.id.as_str()) {
                None => added_questions.push(q.id.clone()),
                Some(&prev) if !prev.resolved && q.resolved => {
                    resolved_questions.push(q.id.clone());
                }
                Some(_) => {}
            }
        }

        Self {
            outcome_changed,
            status_changed,
            added_non_goals,
            removed_non_goals,
            added_invariants,
            removed_invariants,
            added_forbidden_actions,
            removed_forbidden_actions,
            added_approval_boundaries,
            removed_approval_boundaries,
            added_steps,
            removed_steps,
            modified_steps,
            added_acceptance,
            removed_acceptance,
            modified_acceptance,
            resolved_questions,
            added_questions,
        }
    }

    pub fn is_empty(&self) -> bool {
        !self.outcome_changed
            && self.status_changed.is_none()
            && self.added_non_goals.is_empty()
            && self.removed_non_goals.is_empty()
            && self.added_invariants.is_empty()
            && self.removed_invariants.is_empty()
            && self.added_forbidden_actions.is_empty()
            && self.removed_forbidden_actions.is_empty()
            && self.added_approval_boundaries.is_empty()
            && self.removed_approval_boundaries.is_empty()
            && self.added_steps.is_empty()
            && self.removed_steps.is_empty()
            && self.modified_steps.is_empty()
            && self.added_acceptance.is_empty()
            && self.removed_acceptance.is_empty()
            && self.modified_acceptance.is_empty()
            && self.resolved_questions.is_empty()
            && self.added_questions.is_empty()
    }

    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if let Some((from, to)) = self.status_changed {
            lines.push(format!("Status: {} -> {}", from.label(), to.label()));
        }
        if self.outcome_changed {
            lines.push("Outcome updated".to_string());
        }
        push_count(&mut lines, "Added non-goals", &self.added_non_goals);
        push_count(&mut lines, "Removed non-goals", &self.removed_non_goals);
        push_count(&mut lines, "Added invariants", &self.added_invariants);
        push_count(&mut lines, "Removed invariants", &self.removed_invariants);
        push_count(
            &mut lines,
            "Added forbidden actions",
            &self.added_forbidden_actions,
        );
        push_count(
            &mut lines,
            "Removed forbidden actions",
            &self.removed_forbidden_actions,
        );
        push_count(
            &mut lines,
            "Added approval boundaries",
            &self.added_approval_boundaries,
        );
        push_count(
            &mut lines,
            "Removed approval boundaries",
            &self.removed_approval_boundaries,
        );
        push_ids(&mut lines, "Added steps", &self.added_steps);
        push_ids(&mut lines, "Removed steps", &self.removed_steps);
        push_ids(&mut lines, "Modified steps", &self.modified_steps);
        push_ids(&mut lines, "Added acceptance", &self.added_acceptance);
        push_ids(&mut lines, "Removed acceptance", &self.removed_acceptance);
        push_ids(&mut lines, "Modified acceptance", &self.modified_acceptance);
        push_ids(&mut lines, "Resolved questions", &self.resolved_questions);
        push_ids(&mut lines, "Added questions", &self.added_questions);
        lines
    }
}

fn diff_string_lists(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>) {
    let before_set: HashSet<&str> = before.iter().map(String::as_str).collect();
    let after_set: HashSet<&str> = after.iter().map(String::as_str).collect();

    let added = after
        .iter()
        .filter(|s| !before_set.contains(s.as_str()))
        .cloned()
        .collect();
    let removed = before
        .iter()
        .filter(|s| !after_set.contains(s.as_str()))
        .cloned()
        .collect();
    (added, removed)
}

fn push_count(lines: &mut Vec<String>, label: &str, items: &[String]) {
    if !items.is_empty() {
        lines.push(format!("{label}: {}", items.len()));
    }
}

fn push_ids(lines: &mut Vec<String>, label: &str, ids: &[String]) {
    if !ids.is_empty() {
        lines.push(format!("{label}: {}", ids.join(", ")));
    }
}
