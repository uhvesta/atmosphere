//! Settings › General › To-do (TODO2WIRE, D902): what Humata does by itself
//! when pull requests and items change.

/// Whether Humata asks first or acts and shows Undo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TodoMode {
    /// The item asks with a one-click question.
    #[default]
    Ask,
    /// Humata does it and shows Undo.
    Auto,
}

/// Days of inactivity before an In progress item asks about carrying on,
/// until the owner picks another.
pub const DEFAULT_IDLE_NUDGE_DAYS: u32 = 3;

/// The days the idle-nudge pop-up offers, in order.
pub const IDLE_NUDGE_CHOICES: [u32; 4] = [1, 3, 7, 14];

/// The To-do automation section as the daemon holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TodoAutomation {
    /// A merged pull request closes its item: ask, or do it with Undo.
    pub on_pr_merged: TodoMode,
    /// A closed item archives its sessions: ask, or do it with Undo.
    pub on_todo_closed_archive_sessions: TodoMode,
    /// Failing checks go to the item's session when its turn ends.
    pub auto_wake_on_ci_fail: bool,
    /// New review comments go to the item's session the same way.
    pub auto_wake_on_review_comments: bool,
    /// Days without activity before an In progress item asks what next.
    pub idle_nudge_days: u32,
}

impl Default for TodoAutomation {
    fn default() -> Self {
        Self {
            on_pr_merged: TodoMode::Ask,
            on_todo_closed_archive_sessions: TodoMode::Ask,
            auto_wake_on_ci_fail: false,
            auto_wake_on_review_comments: false,
            idle_nudge_days: DEFAULT_IDLE_NUDGE_DAYS,
        }
    }
}

impl TodoAutomation {
    /// The same section with a stored 0 days read as the default.
    #[must_use]
    pub const fn normalized(mut self) -> Self {
        if self.idle_nudge_days == 0 {
            self.idle_nudge_days = DEFAULT_IDLE_NUDGE_DAYS;
        }
        self
    }
}
