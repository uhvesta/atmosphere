//! Pane sizes (NV-12–15): the sidebar and the session's side panel can be
//! dragged wider or narrower, dragged shut past their smallest width,
//! reopened, and reset with a double-click. A narrow window shrinks them to
//! fit without changing what is saved.

use serde::{Deserialize, Serialize};

/// A pane that can be resized and collapsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PaneId {
    /// The sidebar, on the left.
    Sidebar,
    /// The Panes sidebar, on the right (NV-29).
    Panel,
}

impl PaneId {
    /// Its width before anyone drags it.
    #[must_use]
    pub const fn default_width(self) -> f32 {
        match self {
            Self::Sidebar => 268.0,
            Self::Panel => 248.0,
        }
    }

    /// Its narrowest width while open.
    #[must_use]
    pub const fn min_width(self) -> f32 {
        match self {
            Self::Sidebar => 180.0,
            Self::Panel => 200.0,
        }
    }

    /// Its widest.
    #[must_use]
    pub const fn max_width(self) -> f32 {
        match self {
            Self::Sidebar => 480.0,
            Self::Panel => 420.0,
        }
    }

    /// The name people see, such as on its divider.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Panel => "Panes sidebar",
        }
    }
}

/// Dragging this far under a pane's smallest width collapses it.
pub const COLLAPSE_SLACK: f32 = 60.0;

/// The narrowest the main area gets before panes shrink.
pub const MAIN_MIN: f32 = 360.0;

/// One pane's saved size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pane {
    /// Its width while open.
    pub width: f32,
    /// Whether it is collapsed to a strip.
    pub collapsed: bool,
}

/// Both panes' saved sizes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// The sidebar.
    pub sidebar: Pane,
    /// The side panel.
    pub panel: Pane,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            sidebar: Pane {
                width: PaneId::Sidebar.default_width(),
                collapsed: false,
            },
            panel: Pane {
                width: PaneId::Panel.default_width(),
                collapsed: false,
            },
        }
    }
}

impl Layout {
    /// The pane `id`.
    #[must_use]
    pub const fn pane(&self, id: PaneId) -> Pane {
        match id {
            PaneId::Sidebar => self.sidebar,
            PaneId::Panel => self.panel,
        }
    }

    fn pane_mut(&mut self, id: PaneId) -> &mut Pane {
        match id {
            PaneId::Sidebar => &mut self.sidebar,
            PaneId::Panel => &mut self.panel,
        }
    }

    /// A divider was dragged so the pane would be `width` wide.
    pub fn drag(&mut self, id: PaneId, width: f32) {
        let pane = self.pane_mut(id);
        if width < id.min_width() - COLLAPSE_SLACK {
            pane.collapsed = true;
        } else {
            pane.collapsed = false;
            pane.width = width.clamp(id.min_width(), id.max_width());
        }
    }

    /// The pane's collapse button was clicked.
    pub fn toggle(&mut self, id: PaneId) {
        let pane = self.pane_mut(id);
        pane.collapsed = !pane.collapsed;
    }

    /// The divider was double-clicked.
    pub fn reset(&mut self, id: PaneId) {
        *self.pane_mut(id) = Pane {
            width: id.default_width(),
            collapsed: false,
        };
    }

    /// How wide each pane draws in a window `window` wide, with the panel
    /// shown or not: saved widths, shrunk in proportion when the main area
    /// would get narrower than [`MAIN_MIN`]. Collapsed panes are 0.
    #[must_use]
    pub fn widths(&self, window: f32, panel_shown: bool) -> (f32, f32) {
        let sidebar = if self.sidebar.collapsed { 0.0 } else { self.sidebar.width };
        let panel = if self.panel.collapsed || !panel_shown {
            0.0
        } else {
            self.panel.width
        };
        let room = (window - MAIN_MIN).max(0.0);
        let wanted = sidebar + panel;
        if wanted <= room || wanted == 0.0 {
            return (sidebar, panel);
        }
        let scale = room / wanted;
        (sidebar * scale, panel * scale)
    }
}
