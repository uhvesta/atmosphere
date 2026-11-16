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

impl super::SessionView {
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
