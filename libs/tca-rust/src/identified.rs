//! A list whose elements are found by id, and what changed between two of
//! them, so a view can update only the rows that did.

use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;

/// Something with an id that stays the same as it changes.
pub trait Identified {
    type Id: Clone + Eq + Hash + fmt::Debug;

    fn id(&self) -> Self::Id;
}

/// A list in order whose elements are found by id in constant time. Ids
/// are unique: adding an element whose id is there replaces it.
pub struct IdentifiedVec<T: Identified> {
    items: Vec<T>,
    index: HashMap<T::Id, usize>,
}

/// One change between two lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change<Id> {
    /// Gone from the list.
    Removed { id: Id },
    /// New at `index`, or moved there.
    Inserted { index: usize, id: Id },
    /// At `index`, but not as it was.
    Updated { index: usize, id: Id },
}

impl<T: Identified> IdentifiedVec<T> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            index: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    pub fn ids(&self) -> impl Iterator<Item = T::Id> + '_ {
        self.items.iter().map(Identified::id)
    }

    pub fn contains(&self, id: &T::Id) -> bool {
        self.index.contains_key(id)
    }

    /// Where the element with `id` is.
    pub fn position(&self, id: &T::Id) -> Option<usize> {
        self.index.get(id).copied()
    }

    pub fn get(&self, id: &T::Id) -> Option<&T> {
        self.position(id).map(|at| &self.items[at])
    }

    pub fn first(&self) -> Option<&T> {
        self.items.first()
    }

    pub fn last(&self) -> Option<&T> {
        self.items.last()
    }

    /// Changes the element with `id` with `change`, returning what it
    /// returns; nothing if there's no such element. If `change` gives the
    /// element another id, the list follows it.
    pub fn update<R>(&mut self, id: &T::Id, change: impl FnOnce(&mut T) -> R) -> Option<R> {
        let at = self.position(id)?;
        let result = change(&mut self.items[at]);
        let now = self.items[at].id();
        if &now != id {
            self.reindex();
        }
        Some(result)
    }

    /// Adds `element` at the end, or in the place of the element with its
    /// id, which is returned.
    pub fn push(&mut self, element: T) -> Option<T> {
        let id = element.id();
        match self.position(&id) {
            Some(at) => Some(std::mem::replace(&mut self.items[at], element)),
            None => {
                self.index.insert(id, self.items.len());
                self.items.push(element);
                None
            }
        }
    }

    /// Puts `element` at `at` (at most the length), taking out any element
    /// with its id first, which is returned.
    pub fn insert(&mut self, at: usize, element: T) -> Option<T> {
        let old = self.remove(&element.id());
        let at = at.min(self.items.len());
        self.items.insert(at, element);
        self.reindex_from(at);
        old
    }

    /// Takes out the element with `id`.
    pub fn remove(&mut self, id: &T::Id) -> Option<T> {
        let at = self.index.remove(id)?;
        let element = self.items.remove(at);
        self.reindex_from(at);
        Some(element)
    }

    /// Keeps only the elements `keep` says to.
    pub fn retain(&mut self, keep: impl FnMut(&T) -> bool) {
        self.items.retain(keep);
        self.reindex();
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.index.clear();
    }

    /// Sorts the elements by `key`, keeping equal ones in order.
    pub fn sort_by_key<K: Ord>(&mut self, key: impl FnMut(&T) -> K) {
        self.items.sort_by_key(key);
        self.reindex();
    }

    /// What changed from `before` to this list: what went or moves, then,
    /// in order, what came or moved and what changed. Moves are as few as can be:
    /// the elements that kept their order stay, and the rest are taken out
    /// and put back.
    pub fn changes(&self, before: &Self) -> Vec<Change<T::Id>>
    where
        T: PartialEq,
    {
        let mut changes = Vec::new();
        for element in &before.items {
            let id = element.id();
            if !self.contains(&id) {
                changes.push(Change::Removed { id });
            }
        }
        // Of the elements in both, those whose places in `before` rise
        // longest stay; the rest moved.
        let kept: Vec<(usize, usize)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(at, element)| before.position(&element.id()).map(|was| (at, was)))
            .collect();
        let stayed = longest_rising(&kept.iter().map(|&(_, was)| was).collect::<Vec<_>>());
        let mut moved = vec![true; kept.len()];
        for at in stayed {
            moved[at] = false;
        }
        let moved: HashMap<usize, bool> = kept
            .iter()
            .zip(moved)
            .map(|(&(at, _), moved)| (at, moved))
            .collect();
        // All that moves is taken out first, so each is put back where
        // everything before it is already in place.
        for (&at, &moves) in &moved {
            if moves {
                changes.push(Change::Removed {
                    id: self.items[at].id(),
                });
            }
        }
        for (at, element) in self.items.iter().enumerate() {
            let id = element.id();
            match moved.get(&at) {
                None | Some(true) => changes.push(Change::Inserted { index: at, id }),
                Some(false) => {
                    if before.get(&id) != Some(element) {
                        changes.push(Change::Updated { index: at, id });
                    }
                }
            }
        }
        changes
    }

    fn reindex(&mut self) {
        self.index.clear();
        self.reindex_from(0);
    }

    fn reindex_from(&mut self, from: usize) {
        for (at, element) in self.items.iter().enumerate().skip(from) {
            self.index.insert(element.id(), at);
        }
    }
}

/// The positions in `values` of a longest run of rising values, in order.
fn longest_rising(values: &[usize]) -> Vec<usize> {
    // `ends[k]` is the position of the smallest value ending a rising run
    // of k + 1; `previous` links each position to the one before it.
    let mut ends: Vec<usize> = Vec::new();
    let mut previous = vec![usize::MAX; values.len()];
    for (at, &value) in values.iter().enumerate() {
        let k = ends.partition_point(|&end| values[end] < value);
        if k > 0 {
            previous[at] = ends[k - 1];
        }
        if k == ends.len() {
            ends.push(at);
        } else {
            ends[k] = at;
        }
    }
    let mut run = Vec::with_capacity(ends.len());
    let mut at = ends.last().copied().unwrap_or(usize::MAX);
    while at != usize::MAX {
        run.push(at);
        at = previous[at];
    }
    run.reverse();
    run
}

impl<T: Identified> Default for IdentifiedVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Identified + Clone> Clone for IdentifiedVec<T> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            index: self.index.clone(),
        }
    }
}

impl<T: Identified + PartialEq> PartialEq for IdentifiedVec<T> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl<T: Identified + Eq> Eq for IdentifiedVec<T> {}

impl<T: Identified + fmt::Debug> fmt::Debug for IdentifiedVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.items).finish()
    }
}

impl<T: Identified> FromIterator<T> for IdentifiedVec<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut list = Self::new();
        for element in iter {
            list.push(element);
        }
        list
    }
}

impl<T: Identified> std::ops::Index<usize> for IdentifiedVec<T> {
    type Output = T;

    fn index(&self, at: usize) -> &T {
        &self.items[at]
    }
}

impl<'a, T: Identified> IntoIterator for &'a IdentifiedVec<T> {
    type Item = &'a T;
    type IntoIter = std::slice::Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl<T: Identified> IntoIterator for IdentifiedVec<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Row(&'static str, u32);

    impl Identified for Row {
        type Id = &'static str;
        fn id(&self) -> &'static str {
            self.0
        }
    }

    fn list(rows: &[(&'static str, u32)]) -> IdentifiedVec<Row> {
        rows.iter().map(|&(id, n)| Row(id, n)).collect()
    }

    /// Applies `changes` to `before`'s ids as a table would.
    fn apply(before: &IdentifiedVec<Row>, changes: &[Change<&'static str>]) -> Vec<&'static str> {
        let mut ids: Vec<_> = before.ids().collect();
        for change in changes {
            match change {
                Change::Removed { id } => ids.retain(|i| i != id),
                Change::Inserted { index, id } => ids.insert(*index, id),
                Change::Updated { .. } => {}
            }
        }
        ids
    }

    #[test]
    fn an_element_is_found_by_id_after_others_are_taken_out_and_put_in() {
        let mut rows = list(&[("a", 1), ("b", 2), ("c", 3)]);
        rows.remove(&"a");
        rows.insert(0, Row("z", 9));
        assert_eq!(rows.get(&"c"), Some(&Row("c", 3)));
        assert_eq!(rows.position(&"z"), Some(0));
        assert_eq!(rows.ids().collect::<Vec<_>>(), ["z", "b", "c"]);
    }

    #[test]
    fn pushing_an_id_that_is_there_replaces_it_in_place() {
        let mut rows = list(&[("a", 1), ("b", 2)]);
        assert_eq!(rows.push(Row("a", 5)), Some(Row("a", 1)));
        assert_eq!(rows.as_slice(), [Row("a", 5), Row("b", 2)]);
    }

    #[test]
    fn an_update_that_changes_the_id_is_followed() {
        let mut rows = list(&[("a", 1), ("b", 2)]);
        rows.update(&"a", |row| row.0 = "x");
        assert_eq!(rows.get(&"x"), Some(&Row("x", 1)));
        assert!(!rows.contains(&"a"));
    }

    #[test]
    fn changes_turn_the_old_list_into_the_new_one() {
        let before = list(&[("a", 1), ("b", 2), ("c", 3), ("d", 4), ("e", 5)]);
        let after = list(&[("e", 5), ("b", 7), ("c", 3), ("f", 6), ("a", 1)]);
        let moved_later = list(&[("b", 2), ("n", 0), ("c", 3), ("a", 1)]);
        assert_eq!(
            apply(&before, &moved_later.changes(&before)),
            ["b", "n", "c", "a"]
        );
        let changes = after.changes(&before);
        assert_eq!(apply(&before, &changes), after.ids().collect::<Vec<_>>());
        assert!(changes.contains(&Change::Updated { index: 1, id: "b" }));
        // Only what had to move moved: b and c kept their order.
        let moved = changes
            .iter()
            .filter(|c| matches!(c, Change::Inserted { id, .. } if *id != "f"))
            .count();
        assert_eq!(moved, 2);
    }

    #[test]
    fn an_appended_row_is_the_only_change() {
        let before = list(&[("a", 1), ("b", 2)]);
        let after = list(&[("a", 1), ("b", 2), ("c", 3)]);
        assert_eq!(
            after.changes(&before),
            [Change::Inserted { index: 2, id: "c" }]
        );
        assert!(before.changes(&before).is_empty());
    }
}
