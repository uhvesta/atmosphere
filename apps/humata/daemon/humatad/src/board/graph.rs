//! What a board's items wait on: which are blocked, the order to take them
//! in, and whether a link between two would close a loop.
//!
//! An item waits on the items it depends on. A milestone's tasks wait on
//! what their milestone depends on too, and depending on a milestone is
//! waiting for its tasks.

use std::collections::{HashMap, HashSet};

use humata_wire::pb;

use super::{Item, Kind, Status};

/// The loop `from` is in, if its links make one: the ids of the items
/// round it, starting and ending with the same one. A milestone counts as
/// waiting on its tasks, and a task on what its milestone depends on, so a
/// milestone and something one of its tasks holds up can't wait on each
/// other.
pub fn cycle(items: &[Item], from: &str) -> Option<Vec<String>> {
    let by_id: HashMap<&str, &Item> = items.iter().map(|item| (item.id.as_str(), item)).collect();
    let mut waits: HashMap<&str, Vec<&str>> = HashMap::new();
    for item in items {
        let on = waits.entry(item.id.as_str()).or_default();
        on.extend(item.fields.depends_on.iter().map(String::as_str));
        let Some(milestone) = item
            .fields
            .parent_id
            .as_deref()
            .and_then(|id| by_id.get(id))
        else {
            continue;
        };
        on.extend(milestone.fields.depends_on.iter().map(String::as_str));
        waits
            .entry(milestone.id.as_str())
            .or_default()
            .push(item.id.as_str());
    }
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let start = by_id.get(from)?.id.as_str();
    round(&waits, start, &mut path, &mut seen)
        .map(|ids| ids.into_iter().map(str::to_owned).collect())
}

/// Follows what `at` waits on, depth first, until it comes back to an item
/// on the way there.
fn round<'a>(
    waits: &HashMap<&'a str, Vec<&'a str>>,
    at: &'a str,
    path: &mut Vec<&'a str>,
    seen: &mut HashSet<&'a str>,
) -> Option<Vec<&'a str>> {
    if let Some(start) = path.iter().position(|id| *id == at) {
        let mut ids = path[start..].to_vec();
        ids.push(at);
        return Some(ids);
    }
    if !seen.insert(at) {
        return None;
    }
    path.push(at);
    for &on in waits.get(at).into_iter().flatten() {
        if let Some(ids) = round(waits, on, path, seen) {
            return Some(ids);
        }
    }
    path.pop();
    None
}

/// Sets, on a project's `items`, what each reads as (a milestone or a
/// task), whether it is blocked and by which items, and where it comes in
/// the plan.
///
/// An item under another is a task; one with tasks under it, or set to be
/// one, is a milestone. An item is finished once it is Done or Won't do,
/// and a milestone also once all its tasks are. An item is blocked while
/// something it depends on is unfinished, and a milestone's tasks are
/// blocked while it is.
///
/// The plan's order puts each item after what it depends on, the earliest
/// added first where the links leave a choice, and each milestone's tasks
/// right under it in the same order. A link between tasks of two
/// milestones orders the milestones.
pub fn plan(items: &mut [pb::Item]) {
    let planned = planned(items);
    for (item, planned) in items.iter_mut().zip(planned) {
        item.kind = planned.kind.into();
        item.blocked = planned.blocked;
        item.waiting_on = planned.waiting_on;
        item.plan_order = planned.order;
    }
}

struct Planned {
    kind: pb::ItemKind,
    blocked: bool,
    waiting_on: Vec<String>,
    order: u32,
}

fn planned(items: &[pb::Item]) -> Vec<Planned> {
    let count = items.len();
    let index: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .map(|(at, item)| (item.id.as_str(), at))
        .collect();
    let parent: Vec<Option<usize>> = items
        .iter()
        .map(|item| index.get(item.parent_id.as_str()).copied())
        .collect();
    let mut tasks: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (at, parent) in parent.iter().enumerate() {
        if let Some(parent) = parent {
            tasks[*parent].push(at);
        }
    }
    let closed: Vec<bool> = items
        .iter()
        .map(|item| Status::from_wire(item.given_status()).closed())
        .collect();
    let finished: Vec<bool> = (0..count)
        .map(|at| {
            closed[at] || (!tasks[at].is_empty() && tasks[at].iter().all(|task| closed[*task]))
        })
        .collect();
    let depends: Vec<Vec<usize>> = items
        .iter()
        .map(|item| {
            item.depends_on
                .iter()
                .filter_map(|id| index.get(id.as_str()).copied())
                .collect()
        })
        .collect();

    // What each is sorted after, among the items it is sorted with: the
    // project's milestones and lone tasks together, and each milestone's
    // tasks among themselves.
    let top = |at: usize| parent[at].unwrap_or(at);
    let mut after: Vec<HashSet<usize>> = vec![HashSet::new(); count];
    for (at, depends) in depends.iter().enumerate() {
        for &on in depends {
            if top(at) != top(on) {
                after[top(at)].insert(top(on));
            } else if parent[at].is_some() && parent[on].is_some() {
                after[at].insert(on);
            }
        }
    }
    let in_order = |mut among: Vec<usize>| -> Vec<usize> {
        among.sort_by_key(|at| (items[*at].created_unix_ms, *at));
        let mut left: HashSet<usize> = among.iter().copied().collect();
        let mut ordered = Vec::with_capacity(among.len());
        while !among.is_empty() {
            // The earliest with nothing before it left; in a loop, which
            // only links between two milestones' tasks can make of the
            // milestones, the earliest.
            let next = among
                .iter()
                .position(|at| after[*at].iter().all(|on| !left.contains(on)))
                .unwrap_or(0);
            let at = among.remove(next);
            left.remove(&at);
            ordered.push(at);
        }
        ordered
    };
    let mut order = vec![0_u32; count];
    let mut next = 0_u32;
    let tops = (0..count).filter(|at| parent[*at].is_none()).collect();
    for at in in_order(tops) {
        next += 1;
        order[at] = next;
        for task in in_order(tasks[at].clone()) {
            next += 1;
            order[task] = next;
        }
    }

    (0..count)
        .map(|at| {
            let mut waiting: Vec<usize> = Vec::new();
            if !finished[at] {
                let milestone = parent[at].map(|parent| depends[parent].as_slice());
                for &on in depends[at].iter().chain(milestone.into_iter().flatten()) {
                    if !finished[on] && !waiting.contains(&on) {
                        waiting.push(on);
                    }
                }
            }
            let kind = if parent[at].is_some() {
                pb::ItemKind::Task
            } else if !tasks[at].is_empty() || Kind::from_wire(items[at].kind()) == Kind::Milestone
            {
                pb::ItemKind::Milestone
            } else {
                pb::ItemKind::Task
            };
            Planned {
                kind,
                blocked: !waiting.is_empty(),
                waiting_on: waiting.into_iter().map(|on| items[on].id.clone()).collect(),
                order: order[at],
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::ItemFields;

    /// An item added at `created`, under `parent` unless that is empty.
    fn item(id: &str, parent: &str, created: i64, depends_on: &[&str]) -> pb::Item {
        pb::Item {
            id: id.into(),
            parent_id: parent.into(),
            created_unix_ms: created,
            given_status: pb::ItemStatus::Todo.into(),
            depends_on: depends_on.iter().map(|id| (*id).to_owned()).collect(),
            ..Default::default()
        }
    }

    fn given(mut item: pb::Item, status: pb::ItemStatus) -> pb::Item {
        item.set_given_status(status);
        item
    }

    fn blocked(items: &[pb::Item]) -> Vec<(&str, Vec<&str>)> {
        items
            .iter()
            .filter(|item| item.blocked)
            .map(|item| {
                let on = item.waiting_on.iter().map(String::as_str).collect();
                (item.id.as_str(), on)
            })
            .collect()
    }

    fn in_plan_order(items: &[pb::Item]) -> Vec<&str> {
        let mut ordered: Vec<&pb::Item> = items.iter().collect();
        ordered.sort_by_key(|item| item.plan_order);
        ordered.iter().map(|item| item.id.as_str()).collect()
    }

    #[test]
    fn an_item_is_blocked_until_what_it_depends_on_is_done_or_wont_do() {
        let mut items = [
            item("design", "", 1, &[]),
            item("build", "", 2, &["design"]),
            item("ship", "", 3, &["build", "design"]),
        ];
        plan(&mut items);
        assert_eq!(
            blocked(&items),
            [("build", vec!["design"]), ("ship", vec!["build", "design"])]
        );

        items[0].set_given_status(pb::ItemStatus::Done);
        plan(&mut items);
        assert_eq!(blocked(&items), [("ship", vec!["build"])]);

        items[1].set_given_status(pb::ItemStatus::WontDo);
        plan(&mut items);
        assert!(blocked(&items).is_empty());
    }

    #[test]
    fn a_milestone_is_finished_once_all_its_tasks_are() {
        let mut items = [
            item("alpha", "", 1, &[]),
            item("a1", "alpha", 2, &[]),
            item("a2", "alpha", 3, &[]),
            item("beta", "", 4, &["alpha"]),
            // A milestone with no tasks is finished only when it is closed.
            item("empty", "", 5, &[]),
            item("after empty", "", 6, &["empty"]),
        ];
        items[4].set_kind(pb::ItemKind::Milestone);
        plan(&mut items);
        assert_eq!(
            blocked(&items),
            [("beta", vec!["alpha"]), ("after empty", vec!["empty"])]
        );

        items[1].set_given_status(pb::ItemStatus::Done);
        plan(&mut items);
        assert!(items[3].blocked, "one of alpha's tasks is still open");

        items[2].set_given_status(pb::ItemStatus::WontDo);
        plan(&mut items);
        assert_eq!(blocked(&items), [("after empty", vec!["empty"])]);
    }

    #[test]
    fn a_blocked_milestone_blocks_its_open_tasks() {
        let mut items = [
            item("first", "", 1, &[]),
            item("milestone", "", 2, &["first"]),
            item("open", "milestone", 3, &[]),
            given(item("done", "milestone", 4, &[]), pb::ItemStatus::Done),
            item("own", "milestone", 5, &["open"]),
        ];
        plan(&mut items);
        assert_eq!(
            blocked(&items),
            [
                ("milestone", vec!["first"]),
                ("open", vec!["first"]),
                ("own", vec!["open", "first"]),
            ]
        );
    }

    #[test]
    fn an_item_reads_as_a_milestone_when_it_has_tasks_or_was_set_to_be_one() {
        let mut items = [
            item("with tasks", "", 1, &[]),
            item("under", "with tasks", 2, &[]),
            item("alone", "", 3, &[]),
            item("set", "", 4, &[]),
        ];
        items[3].set_kind(pb::ItemKind::Milestone);
        // Set to be a task, but it has one under it.
        items[0].set_kind(pb::ItemKind::Task);
        plan(&mut items);
        let kinds: Vec<pb::ItemKind> = items.iter().map(pb::Item::kind).collect();
        assert_eq!(
            kinds,
            [
                pb::ItemKind::Milestone,
                pb::ItemKind::Task,
                pb::ItemKind::Task,
                pb::ItemKind::Milestone
            ]
        );
    }

    #[test]
    fn the_plan_puts_each_item_after_what_it_depends_on_and_the_earliest_first() {
        let mut items = [
            item("ship", "", 1, &["build"]),
            item("docs", "", 2, &[]),
            item("build", "", 3, &["design"]),
            item("design", "", 4, &[]),
        ];
        plan(&mut items);
        assert_eq!(in_plan_order(&items), ["docs", "design", "build", "ship"]);
    }

    #[test]
    fn a_milestones_tasks_follow_it_in_their_own_order() {
        let mut items = [
            item("beta", "", 1, &["alpha"]),
            item("b1", "beta", 2, &[]),
            item("alpha", "", 3, &[]),
            item("a2", "alpha", 4, &["a1"]),
            item("a0", "alpha", 5, &[]),
            item("a1", "alpha", 6, &[]),
            item("lone", "", 7, &[]),
        ];
        plan(&mut items);
        assert_eq!(
            in_plan_order(&items),
            ["alpha", "a0", "a1", "a2", "beta", "b1", "lone"]
        );
    }

    #[test]
    fn a_link_between_two_milestones_tasks_orders_the_milestones() {
        let mut items = [
            item("later", "", 1, &[]),
            item("l1", "later", 2, &["e1"]),
            item("earlier", "", 3, &[]),
            item("e1", "earlier", 4, &[]),
        ];
        plan(&mut items);
        assert_eq!(in_plan_order(&items), ["earlier", "e1", "later", "l1"]);
    }

    #[test]
    fn every_item_gets_a_place_even_when_tasks_link_two_milestones_both_ways() {
        let mut items = [
            item("one", "", 1, &[]),
            item("o1", "one", 2, &["t1"]),
            item("o2", "one", 3, &[]),
            item("two", "", 4, &[]),
            item("t1", "two", 5, &[]),
            item("t2", "two", 6, &["o2"]),
        ];
        plan(&mut items);
        assert_eq!(
            in_plan_order(&items),
            ["one", "o1", "o2", "two", "t1", "t2"]
        );
    }

    fn stored(id: &str, parent: Option<&str>, depends_on: &[&str]) -> Item {
        Item {
            id: id.into(),
            project_id: "project".into(),
            fields: ItemFields {
                parent_id: parent.map(str::to_owned),
                title: id.into(),
                depends_on: depends_on.iter().map(|id| (*id).to_owned()).collect(),
                ..ItemFields::default()
            },
            created_unix_ms: 0,
            updated_unix_ms: 0,
            session_ids: Vec::new(),
        }
    }

    #[test]
    fn links_that_lead_back_to_an_item_are_a_loop_and_others_are_not() {
        let chain = [
            stored("a", None, &["b"]),
            stored("b", None, &["c"]),
            stored("c", None, &[]),
            stored("d", None, &["b", "c"]),
        ];
        for id in ["a", "b", "c", "d"] {
            assert_eq!(cycle(&chain, id), None);
        }

        let looped = [
            stored("a", None, &["b"]),
            stored("b", None, &["c"]),
            stored("c", None, &["a"]),
        ];
        assert_eq!(
            cycle(&looped, "a"),
            Some(vec!["a".into(), "b".into(), "c".into(), "a".into()])
        );
    }

    #[test]
    fn a_milestone_cant_wait_on_what_one_of_its_tasks_holds_up() {
        // `after` waits on the milestone's task, and the milestone, so its
        // tasks, on `after`.
        let items = [
            stored("milestone", None, &["after"]),
            stored("task", Some("milestone"), &[]),
            stored("after", None, &["task"]),
        ];
        let ids = cycle(&items, "milestone").expect("a loop");
        assert_eq!(ids.first(), ids.last());
        assert!(ids.contains(&"task".to_owned()) && ids.contains(&"after".to_owned()));

        // Nor can a task wait on its own milestone, which waits on it.
        let own = [
            stored("milestone", None, &[]),
            stored("task", Some("milestone"), &["milestone"]),
        ];
        assert!(cycle(&own, "task").is_some());

        // Tasks of one milestone may wait on each other.
        let siblings = [
            stored("milestone", None, &[]),
            stored("first", Some("milestone"), &[]),
            stored("second", Some("milestone"), &["first"]),
        ];
        assert_eq!(cycle(&siblings, "second"), None);
    }
}
