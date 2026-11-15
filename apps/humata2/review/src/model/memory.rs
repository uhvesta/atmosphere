//! What the pane remembers per session: viewed marks, collapsed files and
//! repos, filters and each file's display mode. Saved with the session.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::agent::{AgentView, NoteId};
use super::prepared::{Base, Layout, PreparedReview, Showing};
use super::review::FileKey;

/// Which files the file list shows. Filters that are on narrow the list
/// together; with none on, every changed file shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filters {
    /// Files changed again since you marked them viewed.
    pub changed: bool,
    /// Files with comments that aren't resolved.
    pub commented: bool,
    /// Files you haven't marked viewed at their current version.
    pub unviewed: bool,
}

impl Filters {
    pub fn any(self) -> bool {
        self.changed || self.commented || self.unviewed
    }
}

/// How an image change is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageMode {
    #[default]
    SideBySide,
    /// The new image over the old, cut at `position` (per mille of its width).
    Swipe { position: u16 },
    /// The new image over the old at `opacity` (per mille).
    Onion { opacity: u16 },
}

/// One image file's display: its mode and whether changed pixels are marked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageView {
    pub mode: ImageMode,
    pub highlight: bool,
}

/// What the pane remembers for one session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewMemory {
    /// Each viewed file, with the blob it was viewed at.
    #[serde(with = "pairs")]
    pub viewed: BTreeMap<FileKey, String>,
    pub collapsed: BTreeSet<FileKey>,
    pub collapsed_repos: BTreeSet<String>,
    pub filters: Filters,
    /// Markdown files shown rendered rather than as a source diff.
    pub rendered: BTreeSet<FileKey>,
    #[serde(with = "pairs")]
    pub images: BTreeMap<FileKey, ImageView>,
    /// Whether resolved comments are shown.
    pub show_resolved: bool,
    /// The file the pane was on.
    pub current: Option<FileKey>,
    /// What the changes are compared against (RV-30).
    #[serde(default)]
    pub base: Base,
    /// The agent-prepared review being shown, if any (RV-32).
    #[serde(default)]
    pub prepared: Option<Showing>,
    /// The review chat's notes, highlights and changes (PR-23–PR-26).
    #[serde(default)]
    pub agent: AgentView,
}

/// Whether a file is viewed, and if not, whether it was before it changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Viewed {
    Yes,
    No,
    /// Marked viewed, then changed again.
    ChangedSince,
}

impl ReviewMemory {
    pub fn viewed(&self, key: &FileKey, blob: &str) -> Viewed {
        match self.viewed.get(key) {
            Some(seen) if seen == blob => Viewed::Yes,
            Some(_) => Viewed::ChangedSince,
            None => Viewed::No,
        }
    }

    /// Marks a file viewed at `blob`, or unmarks it.
    pub fn set_viewed(&mut self, key: &FileKey, blob: &str, viewed: bool) {
        if viewed {
            self.viewed.insert(key.clone(), blob.to_owned());
            self.collapsed.insert(key.clone());
        } else {
            self.viewed.remove(key);
            self.collapsed.remove(key);
        }
    }

    pub fn toggle_collapsed(&mut self, key: &FileKey) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.clone());
        }
    }

    pub fn toggle_repo(&mut self, repo: &str) {
        if !self.collapsed_repos.remove(repo) {
            self.collapsed_repos.insert(repo.to_owned());
        }
    }

    /// The layout Revert would put back.
    pub fn layout(&self) -> Layout {
        Layout {
            collapsed: self.collapsed.clone(),
            collapsed_repos: self.collapsed_repos.clone(),
            filters: self.filters,
            rendered: self.rendered.clone(),
            current: self.current.clone(),
            base: self.base,
        }
    }

    /// Shows `view` as an offer the user opened (VW-8), remembering the
    /// layout before so Revert can restore it. Opening another view over one
    /// keeps the first layout. `notes` are the notes it added.
    pub(crate) fn open_prepared(&mut self, view: PreparedReview, notes: Vec<NoteId>) {
        let before = match self.prepared.take() {
            Some(showing) => showing.before.unwrap_or_else(|| self.layout()),
            None => self.layout(),
        };
        self.show_prepared(view, Some(before), notes);
    }

    /// Shows `view`. Filters and collapsed repos are cleared, so every file
    /// it orders shows, and the view opens on its first file.
    pub(crate) fn show_prepared(&mut self, mut view: PreparedReview, before: Option<Layout>, notes: Vec<NoteId>) {
        view.notes.clear();
        self.filters = Filters::default();
        self.collapsed_repos.clear();
        self.current = view
            .groups
            .iter()
            .flat_map(|group| group.files.first())
            .next()
            .cloned()
            .or(self.current.take());
        self.prepared = Some(Showing { view, before, notes });
    }

    /// Puts a layout back.
    pub(crate) fn restore(&mut self, layout: Layout) {
        self.collapsed = layout.collapsed;
        self.collapsed_repos = layout.collapsed_repos;
        self.filters = layout.filters;
        self.rendered = layout.rendered;
        self.current = layout.current;
        self.base = layout.base;
    }

    /// Goes back to what the user had before the view opened (VW-9),
    /// taking away the notes it added. The view's id, if one was showing.
    pub fn revert_prepared(&mut self) -> Option<String> {
        let showing = self.prepared.take()?;
        if let Some(before) = showing.before {
            self.restore(before);
        }
        self.agent.notes.retain(|note| !showing.notes.contains(&note.id));
        Some(showing.view.id)
    }

    /// Keeps the view as it is, dropping what Revert would restore.
    pub fn keep_prepared(&mut self) {
        if let Some(showing) = &mut self.prepared {
            showing.before = None;
        }
    }

    pub fn image(&self, key: &FileKey) -> ImageView {
        self.images.get(key).copied().unwrap_or_default()
    }
}

/// Maps keyed by file are saved as lists of pairs, since formats such as
/// JSON only key maps by strings.
mod pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<K: Serialize, V: Serialize, S: Serializer>(
        map: &BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(map.iter())
    }

    pub(super) fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Ok(Vec::<(K, V)>::deserialize(deserializer)?.into_iter().collect())
    }
}
