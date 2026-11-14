//! The links between a board's items: which items each waits on. They are
//! part of an item's fields in the board's journal, so linking, unlinking
//! and the links a deleted item takes with it are all changes that revert.

use std::collections::HashMap;

use rusqlite::{Connection, Transaction};

use super::{item_row, log, read_item};
use crate::board::{self, Author, Item};

/// The ids of the items `id` depends on, in order of id.
pub(super) fn of_item(connection: &Connection, id: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare_cached(
        "SELECT depends_on_id FROM item_dependencies WHERE item_id = ?1 ORDER BY depends_on_id",
    )?;
    statement.query_map([id], |row| row.get(0))?.collect()
}

/// What each of `project`'s items depends on, as [`of_item`] has it, in one
/// read.
pub(super) fn of_project(
    connection: &Connection,
    project: &str,
) -> rusqlite::Result<HashMap<String, Vec<String>>> {
    let mut statement = connection.prepare_cached(
        "SELECT item_dependencies.item_id, item_dependencies.depends_on_id
         FROM item_dependencies JOIN items ON items.id = item_dependencies.item_id
         WHERE items.project_id = ?1 ORDER BY item_dependencies.depends_on_id",
    )?;
    let rows = statement.query_map([project], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut depends_on: HashMap<String, Vec<String>> = HashMap::new();
    for row in rows {
        let (item, on) = row?;
        depends_on.entry(item).or_default().push(on);
    }
    Ok(depends_on)
}

pub(super) fn add(connection: &Connection, id: &str, on: &str) -> rusqlite::Result<()> {
    connection.execute(
        "INSERT OR IGNORE INTO item_dependencies (item_id, depends_on_id) VALUES (?1, ?2)",
        [id, on],
    )?;
    Ok(())
}

pub(super) fn remove(connection: &Connection, id: &str, on: &str) -> rusqlite::Result<()> {
    connection.execute(
        "DELETE FROM item_dependencies WHERE item_id = ?1 AND depends_on_id = ?2",
        [id, on],
    )?;
    Ok(())
}

/// The loop `from` is in as `project`'s links stand, if they make one: the
/// titles of the items round it, the first again at the end.
pub(super) fn looped(
    connection: &Connection,
    project: &str,
    from: &str,
) -> rusqlite::Result<Option<Vec<String>>> {
    // Without their specs, which a loop has no use for.
    let mut statement = connection.prepare_cached(
        "SELECT id, project_id, parent_id, title, '', status, priority,
             created_unix_ms, updated_unix_ms, kind
         FROM items WHERE project_id = ?1",
    )?;
    let mut items: Vec<Item> = statement
        .query_map([project], item_row)?
        .collect::<Result<_, _>>()?;
    let mut depends_on = of_project(connection, project)?;
    for item in &mut items {
        if let Some(ids) = depends_on.remove(&item.id) {
            item.fields.depends_on = ids;
        }
    }
    let title = |id: &String| {
        items
            .iter()
            .find(|item| item.id == *id)
            .map_or_else(|| id.clone(), |item| item.fields.title.clone())
    };
    Ok(board::cycle(&items, from).map(|ids| ids.iter().map(title).collect()))
}

/// Links `id` to each of `wanted` that is still in `project` and closes no
/// loop, and says which those were.
pub(super) fn relink(
    transaction: &Transaction<'_>,
    project: &str,
    id: &str,
    wanted: &[String],
) -> rusqlite::Result<Vec<String>> {
    let mut linked = Vec::new();
    for on in wanted {
        let there: bool = transaction.query_row(
            "SELECT EXISTS (SELECT 1 FROM items WHERE id = ?1 AND project_id = ?2)",
            [on.as_str(), project],
            |row| row.get(0),
        )?;
        if !there || on == id {
            continue;
        }
        add(transaction, id, on)?;
        if looped(transaction, project, id)?.is_some() {
            remove(transaction, id, on)?;
            continue;
        }
        linked.push(on.clone());
    }
    Ok(linked)
}

/// Removes `item` and every link to and from it. Each item that waited on
/// it gets a change of its own for the link it lost, and the removal, the
/// last change logged, keeps their ids for [`restore_dependents`].
pub(super) fn remove_item(
    transaction: &Transaction<'_>,
    item: &Item,
    reverts: Option<u64>,
    author: &Author,
) -> rusqlite::Result<()> {
    let dependents: Vec<String> = transaction
        .prepare_cached(
            "SELECT item_id FROM item_dependencies WHERE depends_on_id = ?1 ORDER BY item_id",
        )?
        .query_map([&item.id], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    for dependent in &dependents {
        let Some(was) = read_item(transaction, dependent)? else {
            continue;
        };
        let mut now = was.fields.clone();
        now.depends_on.retain(|on| *on != item.id);
        remove(transaction, dependent, &item.id)?;
        log(
            transaction,
            &item.project_id,
            dependent,
            (Some(&was.fields), Some(&now)),
            reverts,
            author,
        )?;
    }
    transaction.execute(
        "DELETE FROM item_dependencies WHERE item_id = ?1",
        [&item.id],
    )?;
    transaction.execute("DELETE FROM items WHERE id = ?1", [&item.id])?;
    log(
        transaction,
        &item.project_id,
        &item.id,
        (Some(&item.fields), None),
        reverts,
        author,
    )?;
    transaction.execute(
        "UPDATE board_changes SET dependents = ?1 WHERE id = last_insert_rowid()",
        [dependents.join("\n")],
    )?;
    Ok(())
}

/// Links the items that waited on `item` when change `removal` removed it
/// to it again, each as a change that reverts `removal`. One that is gone,
/// or whose link would now close a loop, is left out.
pub(super) fn restore_dependents(
    transaction: &Transaction<'_>,
    project: &str,
    item: &str,
    removal: u64,
    author: &Author,
) -> rusqlite::Result<()> {
    let dependents: String = transaction.query_row(
        "SELECT dependents FROM board_changes WHERE id = ?1",
        [i64::try_from(removal).unwrap_or(i64::MAX)],
        |row| row.get(0),
    )?;
    for dependent in dependents.lines() {
        let Some(was) = read_item(transaction, dependent)? else {
            continue;
        };
        if was.fields.depends_on.iter().any(|on| on == item)
            || relink(transaction, project, dependent, &[item.to_owned()])?.is_empty()
        {
            continue;
        }
        let mut now = was.fields.clone();
        now.depends_on.push(item.to_owned());
        now.depends_on.sort();
        log(
            transaction,
            project,
            dependent,
            (Some(&was.fields), Some(&now)),
            Some(removal),
            author,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use humata_agent::Repository;
    use humata_wire::pb;

    use crate::board::{Author, Item, ItemFields, ItemPatch, Kind};
    use crate::error::StoreError;
    use crate::store::Store;

    fn author() -> Author {
        Author::on(pb::ChangeSurface::App)
    }

    fn new_project(store: &Store, name: &str) -> String {
        let repositories = vec![Repository {
            name: "code".into(),
            path: format!("/code/{name}").into(),
        }];
        store
            .add_project(
                name.into(),
                repositories,
                BTreeMap::new(),
                String::new(),
                Vec::new(),
            )
            .expect("added")
            .id
    }

    /// A store with one project, and that project's id.
    fn with_project(dir: &Path) -> (Store, String) {
        let store = Store::open(&dir.join("humata.sqlite3")).expect("the store opens");
        let id = new_project(&store, "one");
        (store, id)
    }

    fn add(store: &Store, project: &str, title: &str, parent: Option<&Item>) -> Item {
        let fields = ItemFields {
            parent_id: parent.map(|parent| parent.id.clone()),
            title: title.into(),
            ..ItemFields::default()
        };
        store.add_item(project, fields, &author()).expect("added")
    }

    fn link(store: &Store, item: &Item, on: &Item) -> Result<Item, StoreError> {
        let patch = ItemPatch {
            link: vec![on.id.clone()],
            ..ItemPatch::default()
        };
        store.update_item(&item.id, patch, &author())
    }

    fn unlink(store: &Store, item: &Item, on: &Item) -> Item {
        let patch = ItemPatch {
            unlink: vec![on.id.clone()],
            ..ItemPatch::default()
        };
        store
            .update_item(&item.id, patch, &author())
            .expect("unlinked")
    }

    fn depends_on(store: &Store, item: &Item) -> Vec<String> {
        store.item(&item.id).expect("read").fields.depends_on
    }

    /// The newest change to `item`.
    fn last_change(store: &Store, project: &str, item: &Item) -> u64 {
        store.board_log(project, 0, 1, &item.id).expect("read")[0].id
    }

    #[test]
    fn a_link_is_kept_listed_and_journaled_with_who_made_it() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (store, project) = with_project(dir.path());
        let design = add(&store, &project, "design", None);
        let build = add(&store, &project, "build", None);

        let linked = link(&store, &build, &design).expect("linked");
        assert_eq!(linked.fields.depends_on, [design.id.clone()]);
        let listed = store.items(&project).expect("listed");
        for item in &listed {
            assert_eq!(*item, store.item(&item.id).expect("read"));
        }
        assert_eq!(depends_on(&store, &build), [design.id.clone()]);
        assert!(depends_on(&store, &design).is_empty());

        let log = store.board_log(&project, 0, 1, "").expect("read");
        assert_eq!(log[0].item_id, build.id);
        assert_eq!(log[0].before.as_ref(), Some(&build.fields));
        assert_eq!(log[0].after.as_ref(), Some(&linked.fields));
        assert_eq!(log[0].author, author());

        // Linking again is no change.
        let before = store.board_log(&project, 0, 0, "").expect("read").len();
        assert_eq!(link(&store, &build, &design).expect("linked"), linked);
        assert_eq!(
            store.board_log(&project, 0, 0, "").expect("read").len(),
            before
        );
    }

    #[test]
    fn reverting_a_link_takes_it_off_and_reverting_an_unlink_puts_it_back() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (store, project) = with_project(dir.path());
        let design = add(&store, &project, "design", None);
        let review = add(&store, &project, "review", None);
        let build = add(&store, &project, "build", None);

        link(&store, &build, &design).expect("linked");
        let linking = last_change(&store, &project, &build);
        // A link made since stays when the first is reverted.
        link(&store, &build, &review).expect("linked");
        let reverted = store.revert(linking, &author()).expect("reverted");
        assert!(reverted.kept.is_empty());
        assert_eq!(depends_on(&store, &build), [review.id.clone()]);
        let log = store.board_log(&project, 0, 1, "").expect("read");
        assert_eq!(log[0].reverts, Some(linking));

        unlink(&store, &build, &review);
        let unlinking = last_change(&store, &project, &build);
        assert!(depends_on(&store, &build).is_empty());
        store.revert(unlinking, &author()).expect("reverted");
        assert_eq!(depends_on(&store, &build), [review.id.clone()]);
    }

    #[test]
    fn a_link_that_closes_a_loop_is_refused_and_names_the_items_round_it() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (store, project) = with_project(dir.path());
        let a = add(&store, &project, "a", None);
        let b = add(&store, &project, "b", None);
        let c = add(&store, &project, "c", None);
        link(&store, &a, &b).expect("linked");
        link(&store, &b, &c).expect("linked");

        let refused = link(&store, &c, &a);
        let Err(StoreError::DependencyCycle { path, .. }) = refused else {
            panic!("expected a loop, got {refused:?}");
        };
        assert_eq!(path.first(), path.last());
        for title in ["a", "b", "c"] {
            assert!(path.iter().any(|named| named == title), "{path:?}");
        }
        // Nothing of it is kept.
        assert!(depends_on(&store, &c).is_empty());
        assert!(
            store
                .board_log(&project, 0, 0, &c.id)
                .expect("read")
                .iter()
                .all(|change| change.before.is_none())
        );
    }

    #[test]
    fn a_milestone_and_what_its_task_holds_up_cant_wait_on_each_other() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (store, project) = with_project(dir.path());
        let milestone = add(&store, &project, "milestone", None);
        let task = add(&store, &project, "task", Some(&milestone));
        let after = add(&store, &project, "after", None);
        link(&store, &after, &task).expect("linked");

        assert!(matches!(
            link(&store, &milestone, &after),
            Err(StoreError::DependencyCycle { .. })
        ));
        assert!(matches!(
            link(&store, &task, &milestone),
            Err(StoreError::DependencyCycle { .. })
        ));
        // Nor can a new task under the milestone wait on what waits on the
        // milestone, and nothing of it is added.
        let gate = add(&store, &project, "gate", None);
        link(&store, &gate, &milestone).expect("linked");
        let fields = ItemFields {
            parent_id: Some(milestone.id.clone()),
            title: "another".into(),
            depends_on: vec![gate.id.clone()],
            ..ItemFields::default()
        };
        assert!(matches!(
            store.add_item(&project, fields, &author()),
            Err(StoreError::DependencyCycle { .. })
        ));
        assert_eq!(store.items(&project).expect("listed").len(), 4);
    }

    #[test]
    fn a_link_to_itself_or_across_projects_or_to_nothing_is_refused() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (store, project) = with_project(dir.path());
        let other = new_project(&store, "two");
        let here = add(&store, &project, "here", None);
        let there = add(&store, &other, "there", None);

        assert!(matches!(
            link(&store, &here, &here),
            Err(StoreError::DependsOnItself { .. })
        ));
        assert!(matches!(
            link(&store, &here, &there),
            Err(StoreError::DependencyInAnotherProject { .. })
        ));
        let nowhere = ItemPatch {
            link: vec!["nope".into()],
            ..ItemPatch::default()
        };
        assert!(matches!(
            store.update_item(&here.id, nowhere, &author()),
            Err(StoreError::UnknownItem { .. })
        ));
        let across = ItemFields {
            title: "new".into(),
            depends_on: vec![there.id.clone()],
            ..ItemFields::default()
        };
        assert!(matches!(
            store.add_item(&project, across, &author()),
            Err(StoreError::DependencyInAnotherProject { .. })
        ));
        assert!(depends_on(&store, &here).is_empty());
    }

    #[test]
    fn deleting_an_item_takes_its_links_with_it_and_reverting_brings_them_back() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (store, project) = with_project(dir.path());
        let first = add(&store, &project, "first", None);
        let middle = add(&store, &project, "middle", None);
        let last = add(&store, &project, "last", None);
        link(&store, &middle, &first).expect("linked");
        let waiting = link(&store, &last, &middle).expect("linked");

        store
            .delete_item(&middle.id, &author(), &[])
            .expect("deleted");
        assert!(depends_on(&store, &last).is_empty());
        // The link `last` lost is a change of its own, by who deleted.
        let lost = store.board_log(&project, 0, 1, &last.id).expect("read");
        assert_eq!(lost[0].before.as_ref(), Some(&waiting.fields));
        assert_eq!(lost[0].author, author());

        let deletion = last_change(&store, &project, &middle);
        let reverted = store.revert(deletion, &author()).expect("reverted");
        assert!(reverted.kept.is_empty());
        assert_eq!(depends_on(&store, &middle), [first.id.clone()]);
        assert_eq!(depends_on(&store, &last), [middle.id.clone()]);
    }

    #[test]
    fn a_link_that_cant_come_back_is_left_out_and_said_to_be() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (store, project) = with_project(dir.path());
        let a = add(&store, &project, "a", None);
        let b = add(&store, &project, "b", None);
        link(&store, &a, &b).expect("linked");
        unlink(&store, &a, &b);
        let unlinking = last_change(&store, &project, &a);
        // Since then the link runs the other way.
        link(&store, &b, &a).expect("linked");

        let reverted = store.revert(unlinking, &author()).expect("reverted");
        assert_eq!(reverted.kept, [pb::ItemField::DependsOn]);
        assert!(depends_on(&store, &a).is_empty());
        assert_eq!(depends_on(&store, &b), [a.id.clone()]);
    }

    #[test]
    fn an_item_is_added_with_its_kind_and_links_and_a_milestone_goes_under_nothing() {
        let dir = tempfile::tempdir().expect("a temporary folder");
        let (store, project) = with_project(dir.path());
        let first = add(&store, &project, "first", None);
        let fields = ItemFields {
            title: "release".into(),
            kind: Kind::Milestone,
            depends_on: vec![first.id.clone(), first.id.clone()],
            ..ItemFields::default()
        };
        let release = store.add_item(&project, fields, &author()).expect("added");
        assert_eq!(release.fields.kind, Kind::Milestone);
        assert_eq!(release.fields.depends_on, [first.id.clone()]);
        assert_eq!(release, store.item(&release.id).expect("read"));

        let under = ItemFields {
            parent_id: Some(release.id.clone()),
            title: "nested".into(),
            kind: Kind::Milestone,
            ..ItemFields::default()
        };
        assert!(matches!(
            store.add_item(&project, under, &author()),
            Err(StoreError::MilestoneUnderItem { .. })
        ));
        let task = add(&store, &project, "task", Some(&release));
        let patch = ItemPatch {
            kind: Some(Kind::Milestone),
            ..ItemPatch::default()
        };
        assert!(matches!(
            store.update_item(&task.id, patch, &author()),
            Err(StoreError::MilestoneUnderItem { .. })
        ));

        // A kind set by hand is a change, and reverts.
        let patch = ItemPatch {
            kind: Some(Kind::Milestone),
            ..ItemPatch::default()
        };
        let set = store
            .update_item(&first.id, patch, &author())
            .expect("updated");
        assert_eq!(set.fields.kind, Kind::Milestone);
        let setting = last_change(&store, &project, &first);
        store.revert(setting, &author()).expect("reverted");
        assert_eq!(store.item(&first.id).expect("read").fields.kind, Kind::Auto);
    }
}
