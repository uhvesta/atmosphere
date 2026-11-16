use super::FakeDaemon;
use crate::daemon::{DaemonError, Event, Operation, Reply};
use crate::model::{Item, ItemId, ItemKind, ItemStatus, Scope, SessionId};

pub(super) fn move_item(daemon: &FakeDaemon, item: &ItemId, status: ItemStatus) -> Reply<()> {
    let id = item.clone();
    daemon.reply(
        Some(Operation::MoveItem),
        format!("move {item} {status:?}"),
        move |inner| {
            let found = inner
                .workspace
                .items
                .iter_mut()
                .find(|found| found.id == id)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Item {id}"),
                })?;
            found.status = status;
            let details = inner.workspace.item_details.entry(id.clone()).or_default();
            details.done_day = (status == ItemStatus::Done).then_some(13);
            details
                .history
                .insert(0, format!("Moved to {} by you · now", status.name()));
            inner.emit(Event::Resync);
            Ok(())
        },
    )
}

pub(super) fn create_item(daemon: &FakeDaemon, scope: Scope, title: &str) -> Reply<ItemId> {
    let title = title.trim().to_owned();
    daemon.reply(
        None,
        format!("create item {scope:?} {title:?}"),
        move |inner| {
            if title.is_empty() {
                return Err(DaemonError::Refused {
                    what: "Adding an item".to_owned(),
                    reason: "it needs a title. Type one and try again".to_owned(),
                });
            }
            let id = ItemId::new(inner.id("N"));
            inner.workspace.items.push(Item {
                id: id.clone(),
                scope,
                kind: ItemKind::Task,
                title,
                status: ItemStatus::Todo,
                depends_on: Vec::new(),
                session: None,
                parent: None,
            });
            inner.emit(Event::Resync);
            Ok(id)
        },
    )
}

pub(super) fn add_dependency(daemon: &FakeDaemon, item: &ItemId, on: &ItemId) -> Reply<()> {
    let item = item.clone();
    let on = on.clone();
    daemon.reply(None, format!("depend {item} on {on}"), move |inner| {
        if item == on || would_cycle(&inner.workspace.items, &item, &on) {
            return Err(DaemonError::Refused {
                what: format!("Making {item} wait for {on}"),
                reason: format!(
                    "{on} already waits for {item}, so neither could start. Remove one of the two dependencies"
                ),
            });
        }
        let found = inner
            .workspace
            .items
            .iter_mut()
            .find(|found| found.id == item)
            .ok_or_else(|| DaemonError::NotFound {
                what: format!("Item {item}"),
            })?;
        if !found.depends_on.contains(&on) {
            found.depends_on.push(on);
        }
        inner.emit(Event::Resync);
        Ok(())
    })
}

pub(super) fn remove_dependency(daemon: &FakeDaemon, item: &ItemId, on: &ItemId) -> Reply<()> {
    let item = item.clone();
    let on = on.clone();
    daemon.reply(None, format!("undepend {item} on {on}"), move |inner| {
        let found = inner
            .workspace
            .items
            .iter_mut()
            .find(|found| found.id == item)
            .ok_or_else(|| DaemonError::NotFound {
                what: format!("Item {item}"),
            })?;
        found.depends_on.retain(|each| each != &on);
        inner.emit(Event::Resync);
        Ok(())
    })
}

pub(super) fn set_item_session(
    daemon: &FakeDaemon,
    item: &ItemId,
    session: &SessionId,
    working: bool,
) -> Reply<()> {
    let item = item.clone();
    let session = session.clone();
    daemon.reply(
        None,
        format!("item {item} session {session} working {working}"),
        move |inner| {
            if inner.workspace.session(&session).is_none() {
                return Err(DaemonError::NotFound {
                    what: format!("Session {session}"),
                });
            }
            let found = inner
                .workspace
                .items
                .iter_mut()
                .find(|found| found.id == item)
                .ok_or_else(|| DaemonError::NotFound {
                    what: format!("Item {item}"),
                })?;
            let details = inner
                .workspace
                .item_details
                .entry(item.clone())
                .or_default();
            if details.sessions.is_empty() {
                details.sessions.extend(found.session.iter().cloned());
            }
            details.sessions.retain(|each| each != &session);
            if working {
                details.sessions.push(session);
            }
            found.session = details.sessions.first().cloned();
            inner.emit(Event::Resync);
            Ok(())
        },
    )
}

pub(super) fn item_history(daemon: &FakeDaemon, item: &ItemId) -> Reply<Vec<String>> {
    let item = item.clone();
    daemon.reply(None, format!("history {item}"), move |inner| {
        if !inner.workspace.items.iter().any(|found| found.id == item) {
            return Err(DaemonError::NotFound {
                what: format!("Item {item}"),
            });
        }
        Ok(inner
            .workspace
            .item_details
            .get(&item)
            .map(|details| details.history.clone())
            .unwrap_or_default())
    })
}

fn would_cycle(items: &[Item], item: &ItemId, on: &ItemId) -> bool {
    // A cycle exists when `item` is already reachable from `on`.
    let mut stack = vec![on.clone()];
    let mut seen = Vec::new();
    while let Some(next) = stack.pop() {
        if &next == item {
            return true;
        }
        if seen.contains(&next) {
            continue;
        }
        if let Some(found) = items.iter().find(|found| found.id == next) {
            stack.extend(found.depends_on.iter().cloned());
        }
        seen.push(next);
    }
    false
}
