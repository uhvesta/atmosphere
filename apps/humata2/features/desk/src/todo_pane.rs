//! Per-channel To-do List-only split pane state beside Channel Desk (DESKTODO).

/// Minimum width of the To-do split pane in pixels.
pub const TODO_PANE_MIN_WIDTH: f32 = 320.0;

/// Maximum width of the To-do split pane in pixels.
pub const TODO_PANE_MAX_WIDTH: f32 = 460.0;

/// Default width of the To-do split pane in pixels.
pub const TODO_PANE_DEFAULT_WIDTH: f32 = 360.0;

/// Per-channel state for the live To-do List-only split pane beside Channel Desk.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelTodoPane {
    /// Whether the split pane is currently shown.
    pub visible: bool,
    /// Clamped width in pixels (`320.0..=460.0`), remembered per channel.
    pub width: f32,
    /// Number of agent board changes observed while the pane was hidden.
    pub unseen_agent_changes: usize,
    /// Whether the inline search box in the pane header is open.
    pub search_open: bool,
    /// Current search filter text.
    pub search_query: String,
    /// Whether the folded `Done` section is expanded.
    pub done_expanded: bool,
}

impl Default for ChannelTodoPane {
    fn default() -> Self {
        Self {
            visible: false,
            width: TODO_PANE_DEFAULT_WIDTH,
            unseen_agent_changes: 0,
            search_open: false,
            search_query: String::new(),
            done_expanded: false,
        }
    }
}

/// Clamps a To-do split pane width to [`TODO_PANE_MIN_WIDTH`]..=[`TODO_PANE_MAX_WIDTH`].
#[must_use]
pub fn clamp_todo_pane_width(width: f32) -> f32 {
    width.clamp(TODO_PANE_MIN_WIDTH, TODO_PANE_MAX_WIDTH)
}
