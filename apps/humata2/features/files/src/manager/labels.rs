//! Times the views print, worked out from the read times in state.

use humata2_clients::{Entry, StorageRow, TrashEntry};

use super::when::{age_label, days_left, modified_label};
use super::{FilesState, Side};

impl FilesState {
    /// The latest moment any listing or measurement was taken; the best
    /// "now" the state knows.
    #[must_use]
    pub fn now_unix_s(&self) -> i64 {
        let trash = self
            .trash
            .as_ref()
            .and_then(|trash| trash.listing.as_ref())
            .map(|listing| listing.read_unix_s);
        let storage = self
            .storage
            .as_ref()
            .and_then(|storage| storage.report.as_ref())
            .map(|report| report.measured_unix_s);
        [self.left.read_unix_s, self.right.read_unix_s]
            .into_iter()
            .chain(trash)
            .chain(storage)
            .max()
            .unwrap_or(0)
    }

    /// An entry's modified time in `side`'s pane, as the list shows it.
    #[must_use]
    pub fn modified(&self, side: Side, entry: &Entry) -> String {
        let pane = self.pane(side);
        modified_label(entry.modified_unix_s, pane.read_unix_s, pane.utc_offset_s)
    }

    /// When a Storage row was last used, short (`5m`, `9d`), or a dash.
    #[must_use]
    pub fn used(&self, row: &StorageRow) -> String {
        let now = self
            .storage
            .as_ref()
            .and_then(|storage| storage.report.as_ref())
            .map_or_else(|| self.now_unix_s(), |report| report.measured_unix_s);
        row.last_used_unix_s
            .map_or_else(|| "\u{2014}".to_owned(), |then| age_label(then, now))
    }

    /// How long ago the open Storage page was measured: `now`, `5m`, ...
    #[must_use]
    pub fn measured_ago(&self) -> Option<String> {
        let report = self.storage.as_ref()?.report.as_ref()?;
        Some(age_label(report.measured_unix_s, self.now_unix_s()))
    }

    /// When a trashed thing is emptied: `today`, `in 1 day`, `in 12 days`.
    #[must_use]
    pub fn empties_in(&self, item: &TrashEntry) -> String {
        let now = self
            .trash
            .as_ref()
            .and_then(|trash| trash.listing.as_ref())
            .map_or_else(|| self.now_unix_s(), |listing| listing.read_unix_s);
        match days_left(item.empties_unix_s, now) {
            0 => "today".to_owned(),
            1 => "in 1 day".to_owned(),
            days => format!("in {days} days"),
        }
    }
}
