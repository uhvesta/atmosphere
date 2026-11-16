//! A session has one or two groups. A terminal has one view, even when an
//! agent is chosen again; closing a group joins its tabs back into the other.

use serde::{Deserialize, Serialize};

/// A terminal, open file, or opted-in prepared view in the session's tab row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tab {
    /// The agent is zero; extra terminals follow it.
    Terminal(usize),
    /// A text file in one of this session's clones.
    File { repo: String, path: String },
    /// An agent-prepared view the person chose to open.
    View(String),
    /// The session's read-only conversation from the host's mirror, one per
    /// session (D186).
    Conversation,
}

/// One of the session's two possible tab groups.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabGroup {
    /// The row's tabs, in order.
    pub tabs: Vec<Tab>,
    /// The selected tab's index in this row.
    pub selected: usize,
}

impl TabGroup {
    pub(super) fn show(&mut self, tab: Tab) {
        self.selected = self.tabs.iter().position(|t| *t == tab).unwrap_or_else(|| {
            self.tabs.push(tab);
            self.tabs.len() - 1
        });
    }
}

/// How wide one tab group is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GroupWidth {
    /// The group shares what is left equally with the other flexible groups.
    Flexible,
    /// The group is this many points wide.
    Fixed(f32),
}

/// How wide the terminal beside a Side-by-side document is (design frame 43).
const BESIDE_DOCUMENT: f32 = 330.;

/// The width of each tab group in a row of `available` points. The group
/// beside a group whose selected tab is a Side-by-side document is
/// `BESIDE_DOCUMENT` wide, or half the row when that is narrower; the
/// document's group takes the rest. Otherwise groups share the row equally.
/// `side_by_side[i]` says whether group `i`'s selected tab is such a document.
#[must_use]
pub fn group_widths(available: f32, side_by_side: &[bool]) -> Vec<GroupWidth> {
    let besides = side_by_side.len() == 2 && side_by_side[0] != side_by_side[1];
    side_by_side
        .iter()
        .map(|document| {
            if besides && !document {
                GroupWidth::Fixed(BESIDE_DOCUMENT.min(available / 2.))
            } else {
                GroupWidth::Flexible
            }
        })
        .collect()
}

impl super::SessionView {
    /// Whether a group's selected tab is a Markdown document shown Side by
    /// side.
    #[must_use]
    pub fn selected_is_side_by_side(&self, group: usize) -> bool {
        match self.selected(group) {
            Some(Tab::File { repo, path }) => {
                super::is_markdown(path)
                    && self.document(repo, path).mode == super::DocumentMode::SideBySide
            }
            _ => false,
        }
    }

    pub(super) fn ensure_groups(&mut self, shells: usize) {
        if self.groups.is_empty() {
            self.groups.push(TabGroup {
                tabs: (0..=shells).map(Tab::Terminal).collect(),
                selected: self.tab.min(shells),
            });
        }
        self.focused_group = self.focused_group.min(self.groups.len() - 1);
    }

    /// The tab selected in a group, if it exists.
    #[must_use]
    pub fn selected(&self, group: usize) -> Option<&Tab> {
        self.groups.get(group).and_then(|g| g.tabs.get(g.selected))
    }

    pub(super) fn show_tab(&mut self, tab: Tab, group: usize, split: bool) {
        // Moving the existing tab avoids duplicate terminal views. A third
        // group is never created; another split reuses the second group.
        let target = if split {
            if self.groups.len() == 1 {
                self.groups.push(TabGroup {
                    tabs: Vec::new(),
                    selected: 0,
                });
            }
            1
        } else {
            group.min(self.groups.len() - 1)
        };
        for (index, g) in self.groups.iter_mut().enumerate() {
            if index != target {
                g.tabs.retain(|t| *t != tab);
                g.selected = g.selected.min(g.tabs.len().saturating_sub(1));
            }
        }
        if let Tab::Terminal(index) = &tab {
            self.tab = *index;
        }
        self.groups[target].show(tab);
        self.focused_group = target;
        self.groups.retain(|g| !g.tabs.is_empty());
        self.focused_group = self.focused_group.min(self.groups.len() - 1);
    }

    pub(super) fn close_group(&mut self, group: usize) {
        if self.groups.len() != 2 || group > 1 {
            return;
        }
        let closed = self.groups.remove(group);
        self.groups[0].tabs.extend(closed.tabs);
        self.focused_group = 0;
        if let Some(Tab::Terminal(tab)) = self.selected(0) {
            self.tab = *tab;
        }
    }
}
