//! Keyboard shortcuts (NV-6, NV-21): the actions they run and where they
//! apply. App-wide ones work from anywhere, a text box included; the
//! sidebar's arrows only when the sidebar has focus.

use gpui_kit::{App, KeyBinding};

gpui_kit::actions!(
    humata,
    [
        /// ⌘K: the command palette.
        OpenPalette,
        /// ⌘N: a new session.
        NewSession,
        /// ⇧⌘N: a new channel.
        NewChannel,
        /// ⌘[: back.
        Back,
        /// Escape: close the top overlay, or cancel what is half-done.
        Dismiss,
        /// ⌘/: the shortcuts.
        ShowShortcuts,
        /// ⌥⌘→: open every channel and fork in the sidebar.
        ExpandAll,
        /// ⌥⌘←: fold them all.
        CollapseAll,
        /// ⌘0: hide or show the sidebar.
        ToggleSidebar,
        /// ⌘1: Home.
        GoHome,
        /// ⌘2: the Inbox.
        GoInbox,
        /// ⌘3: the board.
        GoBoard,
        /// ⌘L: the sidebar takes focus.
        FocusSidebar,
        /// ↑ in the sidebar.
        Up,
        /// ↓ in the sidebar.
        Down,
        /// ← in the sidebar: fold, or go to the parent.
        Out,
        /// → in the sidebar: unfold, or go to the first child.
        In,
        /// Home in the sidebar.
        First,
        /// End in the sidebar.
        Last,
        /// Enter in the sidebar: open the row.
        Open,
        /// ⌘\: split the focused pane side by side.
        SplitPane,
        /// ⇧⌘\: split it stacked.
        SplitPaneDown,
        /// ⌘W: close the focused pane.
        ClosePane,
        /// ⌥⌘←: focus the pane on the left, outside the sidebar.
        FocusLeft,
        /// ⌥⌘→: the pane on the right.
        FocusRight,
        /// ⌥⌘↑: the pane above.
        FocusUp,
        /// ⌥⌘↓: the pane below.
        FocusDown,
        /// ⌘Z outside a text field: undo the last layout change.
        UndoLayout,
        /// ⌥⌘0: hide or show the Panes sidebar.
        TogglePanes,
    ]
);

/// The sidebar's key context.
pub const SIDEBAR: &str = "Sidebar";

/// Every shortcut as people see it, for the Shortcuts sheet: keys, then
/// what they do.
pub const SHORTCUTS: [(&str, &str); 20] = [
    ("⌘K", "Command palette"),
    ("⌘N", "New session"),
    ("⇧⌘N", "New channel"),
    ("⌘[", "Back"),
    ("Esc", "Close or cancel"),
    ("⌘/", "Keyboard shortcuts"),
    ("⌥⌘→", "Expand all, in the sidebar"),
    ("⌥⌘←", "Collapse all, in the sidebar"),
    ("⌘0", "Hide or show the sidebar"),
    ("⌥⌘0", "Hide or show the Panes sidebar"),
    ("⌘\\", "Split the focused pane side by side"),
    ("⇧⌘\\", "Split the focused pane stacked"),
    ("⌘W", "Close the focused pane"),
    ("⌥⌘ ← → ↑ ↓", "Focus the next pane that way"),
    ("⌘Z", "Undo a layout change, outside text fields"),
    ("⌘1 ⌘2 ⌘3", "Home, Inbox, Board"),
    ("⌘L", "Move focus to the sidebar"),
    ("↑ ↓ Home End", "Move in the sidebar"),
    ("← →", "Fold, unfold, parent, child"),
    ("Enter", "Open the sidebar row; send a message"),
];

/// Binds every shortcut. Call once at launch.
pub fn bind(cx: &mut App) {
    let sidebar = Some(SIDEBAR);
    cx.bind_keys([
        KeyBinding::new("cmd-k", OpenPalette, None),
        KeyBinding::new("cmd-n", NewSession, None),
        KeyBinding::new("shift-cmd-n", NewChannel, None),
        KeyBinding::new("cmd-[", Back, None),
        KeyBinding::new("escape", Dismiss, None),
        KeyBinding::new("cmd-/", ShowShortcuts, None),
        KeyBinding::new("alt-cmd-right", ExpandAll, sidebar),
        KeyBinding::new("alt-cmd-left", CollapseAll, sidebar),
        KeyBinding::new("cmd-0", ToggleSidebar, None),
        KeyBinding::new("cmd-1", GoHome, None),
        KeyBinding::new("cmd-2", GoInbox, None),
        KeyBinding::new("cmd-3", GoBoard, None),
        KeyBinding::new("cmd-l", FocusSidebar, None),
        KeyBinding::new("alt-cmd-0", TogglePanes, None),
        KeyBinding::new("cmd-\\", SplitPane, None),
        KeyBinding::new("shift-cmd-\\", SplitPaneDown, None),
        KeyBinding::new("cmd-w", ClosePane, None),
        KeyBinding::new("alt-cmd-left", FocusLeft, None),
        KeyBinding::new("alt-cmd-right", FocusRight, None),
        KeyBinding::new("alt-cmd-up", FocusUp, None),
        KeyBinding::new("alt-cmd-down", FocusDown, None),
        // Text fields bind ⌘Z in their own, deeper context, so typing
        // undoes typing there and this only runs outside them.
        KeyBinding::new("cmd-z", UndoLayout, None),
        KeyBinding::new("up", Up, sidebar),
        KeyBinding::new("down", Down, sidebar),
        KeyBinding::new("left", Out, sidebar),
        KeyBinding::new("right", In, sidebar),
        KeyBinding::new("home", First, sidebar),
        KeyBinding::new("end", Last, sidebar),
        KeyBinding::new("enter", Open, sidebar),
    ]);
}
