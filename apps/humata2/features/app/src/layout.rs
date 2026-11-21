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
    /// The open session's side panel, on the right.
    Panel,
}

impl PaneId {
    /// Its width before anyone drags it.
    #[must_use]
    pub const fn default_width(self) -> f32 {
        match self {
            Self::Sidebar => 236.0,
            Self::Panel => 340.0,
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
            Self::Panel => 2400.0,
        }
    }

    /// The name people see, such as on its divider.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Panel => "side panel",
        }
    }
}

/// Dragging this far under a pane's smallest width collapses it.
pub const COLLAPSE_SLACK: f32 = 60.0;

/// The narrowest the main area gets before panes shrink.
pub const MAIN_MIN: f32 = 360.0;

/// The widest fraction of the window width the right-hand side panel can take.
pub const PANEL_MAX_FRACTION: f32 = 0.5;

/// The widest the right-hand side panel can be in a window of `viewport` px:
/// up to half the window width, never less than its minimum width.
#[must_use]
pub fn panel_max_width(viewport: f32) -> f32 {
    (viewport * PANEL_MAX_FRACTION)
        .max(PaneId::Panel.min_width())
        .min(PaneId::Panel.max_width())
}

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
}

/// A session area that can fill the window without changing saved pane sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FullScreen {
    /// Only the terminal and its header/tabs.
    Terminal,
    /// Only the selected side panel.
    Panel,
}

/// Column widths for this viewport. Automatic folds never alter saved sizes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Columns {
    /// Width of an open sidebar; zero when folded.
    pub sidebar: f32,
    /// Width of an open panel; zero when folded.
    pub panel: f32,
    /// Sidebar is manually or automatically folded to its 14px strip.
    pub sidebar_hidden: bool,
    /// The panel cannot fit or was manually collapsed.
    pub panel_hidden: bool,
}

impl Layout {
    /// Resolve the columns for a live window size. At narrow widths, fold
    /// navigation before the panel; keep the main area useful and restore
    /// both columns automatically when space returns.
    #[must_use]
    pub fn columns(&self, viewport: f32, in_session: bool, panel_shown: bool) -> Columns {
        let right_strip = if !in_session {
            0.
        } else if self.panel.collapsed {
            14.
        } else {
            64.
        };
        let available = (viewport - right_strip).max(0.);
        let panel_wanted = in_session && panel_shown && !self.panel.collapsed;
        let sidebar_hidden = self.sidebar.collapsed
            || available
                < MAIN_MIN
                    + PaneId::Sidebar.min_width()
                    + 1.
                    + if panel_wanted {
                        PaneId::Panel.min_width() + 1.
                    } else {
                        0.
                    };
        let left_strip = if sidebar_hidden { 14. } else { 0. };
        let panel_hidden =
            !panel_wanted || available - left_strip < MAIN_MIN + PaneId::Panel.min_width() + 1.;
        let sidebar = if sidebar_hidden {
            0.
        } else {
            self.sidebar.width
        };
        let panel = if panel_hidden {
            0.
        } else {
            self.panel.width.min(panel_max_width(viewport))
        };
        let dividers = if !sidebar_hidden { 1. } else { 0. } + if !panel_hidden { 1. } else { 0. };
        let room = (available - left_strip - dividers - MAIN_MIN).max(0.);
        let wanted = sidebar + panel;
        let scale = if wanted > room && wanted > 0. {
            room / wanted
        } else {
            1.
        };
        Columns {
            sidebar: sidebar * scale,
            panel: panel * scale,
            sidebar_hidden,
            panel_hidden,
        }
    }

    /// How the sidebar comes back when ⌘0 or ⌘L asks for it, or `None`
    /// when it shows already (NV-6). Folded with room to spare, it docks
    /// again; hidden for want of room, it shows over the page.
    #[must_use]
    pub fn reveal_sidebar(
        &self,
        viewport: f32,
        in_session: bool,
        panel_shown: bool,
    ) -> Option<Reveal> {
        if !self.columns(viewport, in_session, panel_shown).sidebar_hidden {
            return None;
        }
        let mut docked = *self;
        docked.sidebar.collapsed = false;
        Some(if docked.columns(viewport, in_session, panel_shown).sidebar_hidden {
            Reveal::Overlay
        } else {
            Reveal::Dock
        })
    }
}

/// How a hidden sidebar is shown again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reveal {
    /// Unfold it: the window has room for it beside the page.
    Dock,
    /// Show it over the page: the window is too narrow to dock it.
    Overlay,
}
