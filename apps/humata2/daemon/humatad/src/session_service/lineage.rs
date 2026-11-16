//! A session's shown relatives (FK-13): its nearest ancestor that isn't
//! archived and the forks shown under it, from the store's fork origins,
//! and what an archived session shows (ARCHIVED and its summary, MV-7).
//!
//! The store keeps each fork's true parent and never rewrites it, so
//! archiving a session moves its forks up and restoring it puts them back
//! with no bookkeeping.

use std::collections::{HashMap, HashSet};

use humata2_grpc::humata::v1 as pb;
use humata2_store::{ArchiveRow, ForkOrigin, Store, StoreError};
use prost::Message;

use super::{SessionSvc, Tell};
use crate::channel_service::{blocking, timestamp_ms};
use crate::status;

/// Every fork origin and archive row, read once.
pub(super) struct Lineage {
    origins: HashMap<String, ForkOrigin>,
    archives: HashMap<String, ArchiveRow>,
}

impl Lineage {
    pub(super) fn read(store: &Store) -> Result<Self, StoreError> {
        Ok(Self {
            origins: store
                .fork_origins()?
                .into_iter()
                .map(|origin| (origin.session_id.clone(), origin))
                .collect(),
            archives: store
                .archives()?
                .into_iter()
                .map(|row| (row.session_id.clone(), row))
                .collect(),
        })
    }

    fn archived(&self, session: &str) -> bool {
        self.archives.contains_key(session)
    }

    /// The session `session` shows under: its nearest ancestor that isn't
    /// archived.
    fn shown_parent(&self, session: &str) -> Option<&str> {
        let mut at = self.origins.get(session)?.parent.as_str();
        // A walk longer than the origins are many went round a cycle.
        for _ in 0..=self.origins.len() {
            if !self.archived(at) {
                return Some(at);
            }
            at = self.origins.get(at)?.parent.as_str();
        }
        None
    }

    /// The forks shown under `session`, oldest first.
    fn forks_of(&self, session: &str) -> Vec<&ForkOrigin> {
        let mut forks: Vec<&ForkOrigin> = self
            .origins
            .values()
            .filter(|origin| !self.archived(&origin.session_id))
            .filter(|origin| self.shown_parent(&origin.session_id) == Some(session))
            .collect();
        forks.sort_by(|a, b| {
            (a.created_unix_ms, &a.session_id).cmp(&(b.created_unix_ms, &b.session_id))
        });
        forks
    }

    /// How many forks show under `session` now.
    pub(super) fn fork_count(&self, session: &str) -> usize {
        self.forks_of(session).len()
    }

    /// Whether `session` is archived, or being archived.
    pub(super) fn is_archived(&self, session: &str) -> bool {
        self.archived(session)
    }

    /// Puts `session`'s fork origin, forks and archive as they stand.
    pub(super) fn apply(&self, shown: &mut pb::Session) {
        let id = shown
            .id
            .as_ref()
            .map(|id| id.value.clone())
            .unwrap_or_default();
        shown.fork_origin = self.origins.get(&id).and_then(|origin| {
            self.shown_parent(&id).map(|parent| pb::ForkOrigin {
                parent: Some(pb::SessionId {
                    value: parent.to_owned(),
                }),
                kind: origin.kind,
                before_turn: origin.before_turn,
            })
        });
        shown.forks = self
            .forks_of(&id)
            .into_iter()
            .map(|origin| pb::SessionId {
                value: origin.session_id.clone(),
            })
            .collect();
        if let Some(row) = self.archives.get(&id) {
            shown.status = pb::SessionStatus::Archived as i32;
            shown.archive = Some(summary_of(&id, row));
        }
    }

    /// Every session joined to `session` by fork origins, `session` too.
    pub(super) fn family(&self, session: &str) -> Vec<String> {
        let mut near: HashMap<&str, Vec<&str>> = HashMap::new();
        for origin in self.origins.values() {
            near.entry(origin.session_id.as_str())
                .or_default()
                .push(origin.parent.as_str());
            near.entry(origin.parent.as_str())
                .or_default()
                .push(origin.session_id.as_str());
        }
        let mut seen: HashSet<&str> = HashSet::from([session]);
        let mut todo = vec![session];
        while let Some(at) = todo.pop() {
            for next in near.get(at).into_iter().flatten() {
                if seen.insert(next) {
                    todo.push(next);
                }
            }
        }
        let mut family: Vec<String> = seen.into_iter().map(str::to_owned).collect();
        family.sort();
        family
    }
}

/// What the archive row says, for the API; the summary is empty until it is
/// written.
fn summary_of(session: &str, row: &ArchiveRow) -> pb::ArchiveSummary {
    let mut summary = match pb::ArchiveSummary::decode(row.summary.as_slice()) {
        Ok(summary) => summary,
        Err(error) => {
            crate::log::say!(
                "humatad2: session {session}'s archive summary can't be read: {}",
                status::chain(&error)
            );
            pb::ArchiveSummary::default()
        }
    };
    summary.archived_at = Some(timestamp_ms(row.archived_unix_ms));
    summary
}

/// Puts the fork origin, forks and archive on `shown` as the store has them.
/// A store that can't be read leaves it as it was.
pub(super) fn apply(store: &Store, shown: &mut pb::Session) {
    match Lineage::read(store) {
        Ok(lineage) => lineage.apply(shown),
        Err(error) => crate::log::say!(
            "humatad2: couldn't read the fork tree for a session: {}",
            status::chain(&error)
        ),
    }
}

/// Tells the feed of `session` and every session it is a fork of or has
/// forks (a change here moves their shown parents and forks).
pub(super) async fn publish_family(tell: &Tell, session: &str, by: Option<pb::Actor>) {
    let (store, id) = (tell.store.clone(), session.to_owned());
    let family = blocking(move || Lineage::read(&store).map(|lineage| lineage.family(&id))).await;
    let Ok(Ok(family)) = family else {
        return;
    };
    for member in family {
        tell.session(&member, pb::Change::Updated, by.clone()).await;
    }
}

impl SessionSvc {
    /// Puts the fork origin, forks and archive on `shown` (FK-13, MV-7).
    pub(super) fn apply_lineage(&self, shown: &mut pb::Session) {
        apply(&self.starter.store, shown);
    }
}

#[cfg(test)]
mod tests {
    use humata2_store::ArchiveState;

    use super::*;

    fn origin(session: &str, parent: &str, created: i64) -> ForkOrigin {
        ForkOrigin {
            session_id: session.to_owned(),
            parent: parent.to_owned(),
            kind: pb::ForkKind::CodeOnly as i32,
            before_turn: 0,
            created_unix_ms: created,
        }
    }

    fn archive(session: &str) -> ArchiveRow {
        ArchiveRow {
            session_id: session.to_owned(),
            archived_unix_ms: 5,
            state: ArchiveState::Archived,
            summary: Vec::new(),
            free_after_unix_ms: 6,
        }
    }

    fn lineage(origins: &[ForkOrigin], archived: &[&str]) -> Lineage {
        Lineage {
            origins: origins
                .iter()
                .map(|origin| (origin.session_id.clone(), origin.clone()))
                .collect(),
            archives: archived
                .iter()
                .map(|session| ((*session).to_owned(), archive(session)))
                .collect(),
        }
    }

    fn shown(id: &str) -> pb::Session {
        pb::Session {
            id: Some(pb::SessionId {
                value: id.to_owned(),
            }),
            status: pb::SessionStatus::Working as i32,
            ..pb::Session::default()
        }
    }

    fn ids(session: &pb::Session) -> Vec<&str> {
        session.forks.iter().map(|id| id.value.as_str()).collect()
    }

    #[test]
    fn a_fork_shows_under_its_parent_and_the_parent_lists_it() {
        let tree = lineage(&[origin("b", "a", 1), origin("c", "a", 2)], &[]);
        let (mut a, mut b) = (shown("a"), shown("b"));
        tree.apply(&mut a);
        tree.apply(&mut b);
        assert_eq!(ids(&a), ["b", "c"]);
        assert_eq!(b.fork_origin.unwrap().parent.unwrap().value, "a");
        assert!(a.fork_origin.is_none());
    }

    #[test]
    fn forks_move_up_past_an_archived_parent() {
        let tree = lineage(&[origin("b", "a", 1), origin("c", "b", 2)], &["b"]);
        let (mut a, mut b, mut c) = (shown("a"), shown("b"), shown("c"));
        tree.apply(&mut a);
        tree.apply(&mut b);
        tree.apply(&mut c);
        assert_eq!(ids(&a), ["c"]);
        assert_eq!(c.fork_origin.unwrap().parent.unwrap().value, "a");
        // The archived one is shown archived and has no forks left.
        assert_eq!(b.status, pb::SessionStatus::Archived as i32);
        assert!(b.archive.is_some());
        assert!(b.forks.is_empty());
    }

    #[test]
    fn restoring_puts_the_forks_back() {
        let origins = [origin("b", "a", 1), origin("c", "b", 2)];
        let tree = lineage(&origins, &[]);
        let (mut a, mut c) = (shown("a"), shown("c"));
        tree.apply(&mut a);
        tree.apply(&mut c);
        assert_eq!(ids(&a), ["b"]);
        assert_eq!(c.fork_origin.unwrap().parent.unwrap().value, "b");
    }

    #[test]
    fn a_fork_whose_ancestors_are_all_archived_shows_no_parent() {
        let tree = lineage(&[origin("b", "a", 1)], &["a"]);
        let mut b = shown("b");
        tree.apply(&mut b);
        assert!(b.fork_origin.is_none());
    }

    #[test]
    fn the_family_is_everything_joined_by_origins() {
        let tree = lineage(
            &[origin("b", "a", 1), origin("c", "b", 2), origin("y", "x", 3)],
            &[],
        );
        assert_eq!(tree.family("c"), ["a", "b", "c"]);
        assert_eq!(tree.family("lone"), ["lone"]);
    }
}
