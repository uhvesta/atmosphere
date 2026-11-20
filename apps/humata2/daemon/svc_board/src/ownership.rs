//! Ownership transfer, stuck-agent recovery, and write-time dependency cycle
//! detection helpers (TODO2).

use std::collections::{BTreeMap, HashSet};

use humata2_todo::OwnerKind;

use super::todo_fields::{Fields, TodoOwnerRow};

/// Detects whether setting `start_key`'s dependencies to `new_deps` against
/// existing `edges` (key -> depends_on keys) would introduce a dependency cycle.
/// Returns the cycle path `[start_key, ..., start_key]` when one is found.
pub(super) fn find_cycle(
    start_key: &str,
    new_deps: &[String],
    edges: &BTreeMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    for dep in new_deps {
        if dep == start_key {
            return Some(vec![start_key.to_owned(), start_key.to_owned()]);
        }
        let mut path = vec![start_key.to_owned(), dep.clone()];
        let mut visited = HashSet::new();
        visited.insert(dep.as_str());
        if dfs_find_target(dep, start_key, edges, &mut visited, &mut path) {
            return Some(path);
        }
    }
    None
}

fn dfs_find_target<'a>(
    node: &'a str,
    target: &str,
    edges: &'a BTreeMap<String, Vec<String>>,
    visited: &mut HashSet<&'a str>,
    path: &mut Vec<String>,
) -> bool {
    let Some(neighbors) = edges.get(node) else {
        return false;
    };
    for next in neighbors {
        if next == target {
            path.push(target.to_owned());
            return true;
        }
        if visited.insert(next.as_str()) {
            path.push(next.clone());
            if dfs_find_target(next, target, edges, visited, path) {
                return true;
            }
            path.pop();
        }
    }
    false
}

/// Transfers ownership back to the user and clears any stuck-agent notice.
pub(super) fn take_back_ownership(fields: &mut Fields, user_label: &str) {
    fields.owner = Some(TodoOwnerRow {
        kind: OwnerKind::User,
        label: user_label.to_owned(),
        session: None,
    });
    fields.stuck_agent = None;
}

/// Clears a stuck-agent notice so the assigned agent can retry; returns true
/// if a stuck-agent notice was present.
pub(super) fn retry_stuck_agent(fields: &mut Fields) -> bool {
    fields.stuck_agent.take().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::todo_fields::StuckAgentRow;

    #[test]
    fn find_cycle_catches_self_direct_and_transitive_loops() {
        let mut edges = BTreeMap::new();
        edges.insert("B".to_owned(), vec!["C".to_owned()]);
        edges.insert("C".to_owned(), vec!["A".to_owned()]);

        // Self loop
        assert_eq!(
            find_cycle("A", &["A".to_owned()], &edges),
            Some(vec!["A".to_owned(), "A".to_owned()])
        );
        // Transitive loop A -> B -> C -> A
        assert_eq!(
            find_cycle("A", &["B".to_owned()], &edges),
            Some(vec![
                "A".to_owned(),
                "B".to_owned(),
                "C".to_owned(),
                "A".to_owned()
            ])
        );
        // Acyclic edge A -> D
        assert_eq!(find_cycle("A", &["D".to_owned()], &edges), None);
    }

    #[test]
    fn take_back_and_retry_clear_stuck_agent() {
        let mut fields = Fields {
            owner: Some(TodoOwnerRow {
                kind: OwnerKind::Agent,
                label: "claude".into(),
                session: Some("se_1".into()),
            }),
            stuck_agent: Some(StuckAgentRow {
                session: "se_1".into(),
                agent_label: "claude".into(),
                reason: "no progress".into(),
                since_ms: 100,
            }),
            ..Fields::default()
        };
        assert!(retry_stuck_agent(&mut fields));
        assert!(fields.stuck_agent.is_none());
        assert!(!retry_stuck_agent(&mut fields));

        fields.stuck_agent = Some(StuckAgentRow::default());
        take_back_ownership(&mut fields, "You");
        assert!(fields.stuck_agent.is_none());
        assert_eq!(fields.owner.as_ref().unwrap().kind, OwnerKind::User);
        assert_eq!(fields.owner.as_ref().unwrap().label, "You");
    }
}
