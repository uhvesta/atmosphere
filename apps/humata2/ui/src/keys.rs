//! Keyboard shortcuts (NV-6, NV-21): the actions they run and where they
//! apply. App-wide ones work from anywhere, a text box included; the
//! sidebar's arrows only when the sidebar has focus.

use gpui_kit::{App, KeyBinding, Keystroke, OperatingSystem};

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
        /// ⌘,: Settings.
        OpenSettings,
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
        /// ⌥⌘0: hide or show the session's side panel.
        TogglePanel,
        /// ⇧⌘R: start or stop recording a bug report.
        ToggleRecording,
        /// ⇧⌘S: take a bug report snapshot.
        TakeSnapshot,
        /// ⌥⌘M: move the open session to another machine.
        MoveSession,
        /// ⌘4: Hosts.
        GoHosts,
        /// ⌘5: Files.
        GoFiles,
        /// ⇧⌘↩: the terminal full screen.
        TerminalFullScreen,
        /// ⇧⌘F: fork the open session.
        ForkSession,
    ]
);

/// The sidebar's key context.
pub const SIDEBAR: &str = "Sidebar";

/// Every shortcut as people see it, for the Shortcuts sheet: keys, then
/// what they do.
pub const SHORTCUTS: [(&str, &str); 19] = [
    ("⌘K", "Command palette"),
    ("⌘N", "New session"),
    ("⇧⌘N", "New channel"),
    ("⌘[", "Back"),
    ("Esc", "Close or cancel"),
    ("⌘/", "Keyboard shortcuts"),
    ("⌥⌘→", "Expand all, in the sidebar"),
    ("⌥⌘←", "Collapse all, in the sidebar"),
    ("⌘0", "Hide or show the sidebar"),
    ("⌥⌘0", "Hide or show the side panel"),
    ("⌥⌘M", "Move the session to another machine"),
    ("⌘1 ⌘2 ⌘3", "Home, Inbox, Board"),
    ("⌘,", "Settings"),
    ("⌘L", "Move focus to the sidebar"),
    ("↑ ↓ Home End", "Move in the sidebar"),
    ("← →", "Fold, unfold, parent, child"),
    ("Enter", "Open the sidebar row; send a message"),
    ("⇧⌘R", "Start or stop recording a bug report"),
    ("⇧⌘S", "Take a bug report snapshot"),
];

/// Binds every shortcut. Call once at launch. Off a Mac each ⌘ chord
/// moves off the Super key the desktop keeps for itself (D318): see
/// [`off_mac`].
pub fn bind(cx: &mut App) {
    let info = cx.platform_info();
    if info.is_mac() {
        cx.bind_keys(bindings());
        return;
    }
    let rebound: Vec<KeyBinding> = bindings()
        .into_iter()
        .map(|binding| {
            let keys = binding
                .keystrokes()
                .iter()
                .map(|keystroke| off_mac(keystroke.inner(), info.os))
                .collect::<Vec<_>>()
                .join(" ");
            let action = binding.action().boxed_clone();
            KeyBinding::load(&keys, action, binding.predicate(), false, None, cx)
                .unwrap_or(binding)
        })
        .collect();
    cx.bind_keys(rebound);
}

/// A chord as it is typed off a Mac (D318). ⌘ becomes Ctrl+Shift, and a ⌘
/// chord that already holds Shift or ⌥ becomes Ctrl+⌥+Shift, so ⌘N and
/// ⇧⌘N stay two shortcuts. Plain Ctrl chords stay the shell's: a binding
/// is matched before the terminal sees the key, and Ctrl+K, Ctrl+L and
/// Ctrl+[ mean something there. A chord without ⌘ is left as it is.
fn off_mac(keystroke: &Keystroke, os: OperatingSystem) -> String {
    let mut keystroke = keystroke.clone();
    let modifiers = &mut keystroke.modifiers;
    if modifiers.platform {
        modifiers.platform = false;
        modifiers.control = true;
        if modifiers.shift || modifiers.alt {
            modifiers.alt = true;
        }
        modifiers.shift = true;
    }
    keystroke.unparse(os)
}

/// A shortcut label as the keys are pressed on this machine. `glyphs` is
/// a Mac label such as `⇧⌘N` or `⌘1 ⌘2 ⌘3`; off a Mac each ⌘ part is
/// written as [`off_mac`] binds it, in the Linux style (`Ctrl+Alt+Shift+N`).
/// A part without ⌘ (`Esc`, `↑ ↓`) and any label on a Mac are returned as
/// they are, so text that is not a shortcut passes through.
pub(crate) fn label(glyphs: &str, mac: bool) -> String {
    if mac || !glyphs.contains('⌘') {
        return glyphs.to_owned();
    }
    glyphs
        .split(' ')
        .map(|part| {
            if part.contains('⌘') {
                chord_label(part)
            } else {
                part.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One ⌘ chord off a Mac, the same rule as [`off_mac`].
fn chord_label(part: &str) -> String {
    let mut extra = false;
    let mut key = String::new();
    for glyph in part.chars() {
        match glyph {
            '⌘' | '⌃' => {}
            '⌥' | '⇧' => extra = true,
            other => key.push(other),
        }
    }
    let key = match key.as_str() {
        "→" => "Right".to_owned(),
        "←" => "Left".to_owned(),
        "↑" => "Up".to_owned(),
        "↓" => "Down".to_owned(),
        "↩" => "Enter".to_owned(),
        "⌫" => "Backspace".to_owned(),
        single if single.chars().count() == 1 => single.to_uppercase(),
        named => named.to_owned(),
    };
    let modifiers = if extra { "Ctrl+Alt+Shift" } else { "Ctrl+Shift" };
    format!("{modifiers}+{key}")
}

/// Every shortcut as written, ⌘ being the Mac's Command key.
fn bindings() -> Vec<KeyBinding> {
    let sidebar = Some(SIDEBAR);
    vec![
        KeyBinding::new("cmd-k", OpenPalette, None),
        KeyBinding::new("cmd-n", NewSession, None),
        KeyBinding::new("shift-cmd-n", NewChannel, None),
        KeyBinding::new("cmd-[", Back, None),
        KeyBinding::new("escape", Dismiss, None),
        // In a terminal, Escape is the program's (vim, less): the Shell's
        // key-down capture still closes a menu or sheet that is open first.
        // "HumataTerminal" is the terminal view's key context.
        KeyBinding::new("escape", gpui_kit::NoAction, Some("HumataTerminal")),
        KeyBinding::new("cmd-/", ShowShortcuts, None),
        KeyBinding::new("alt-cmd-right", ExpandAll, sidebar),
        KeyBinding::new("alt-cmd-left", CollapseAll, sidebar),
        KeyBinding::new("cmd-0", ToggleSidebar, None),
        KeyBinding::new("cmd-1", GoHome, None),
        KeyBinding::new("cmd-2", GoInbox, None),
        KeyBinding::new("cmd-3", GoBoard, None),
        KeyBinding::new("cmd-,", OpenSettings, None),
        KeyBinding::new("cmd-l", FocusSidebar, None),
        KeyBinding::new("alt-cmd-0", TogglePanel, None),
        KeyBinding::new("alt-cmd-m", MoveSession, None),
        KeyBinding::new("cmd-4", GoHosts, None),
        KeyBinding::new("cmd-5", GoFiles, None),
        KeyBinding::new("shift-cmd-enter", TerminalFullScreen, None),
        KeyBinding::new("shift-cmd-f", ForkSession, None),
        KeyBinding::new("shift-cmd-r", ToggleRecording, None),
        KeyBinding::new("shift-cmd-s", TakeSnapshot, None),
        KeyBinding::new("up", Up, sidebar),
        KeyBinding::new("down", Down, sidebar),
        KeyBinding::new("left", Out, sidebar),
        KeyBinding::new("right", In, sidebar),
        KeyBinding::new("home", First, sidebar),
        KeyBinding::new("end", Last, sidebar),
        KeyBinding::new("enter", Open, sidebar),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gpui_kit::{Keystroke, OperatingSystem};

    use super::{SHORTCUTS, bindings, label, off_mac};

    /// A chord as a Mac label writes it: ⌃⌥⇧⌘, then the key.
    fn mac_glyphs(keystroke: &Keystroke) -> String {
        let modifiers = &keystroke.modifiers;
        let mut text = String::new();
        for (held, glyph) in [
            (modifiers.control, "⌃"),
            (modifiers.alt, "⌥"),
            (modifiers.shift, "⇧"),
            (modifiers.platform, "⌘"),
        ] {
            if held {
                text.push_str(glyph);
            }
        }
        let key = match keystroke.key.as_str() {
            "right" => "→".to_owned(),
            "left" => "←".to_owned(),
            "up" => "↑".to_owned(),
            "down" => "↓".to_owned(),
            "enter" => "↩".to_owned(),
            other => other.to_uppercase(),
        };
        text + &key
    }

    /// A typed chord in the Linux style, `Ctrl+Alt+Shift+Right`.
    fn linux_style(keystroke: &Keystroke) -> String {
        let modifiers = &keystroke.modifiers;
        let mut text = String::new();
        for (held, name) in [
            (modifiers.control, "Ctrl+"),
            (modifiers.alt, "Alt+"),
            (modifiers.shift, "Shift+"),
        ] {
            if held {
                text.push_str(name);
            }
        }
        let mut key = keystroke.key.chars();
        match key.next() {
            Some(first) => {
                text.extend(first.to_uppercase());
                text.push_str(key.as_str());
            }
            None => {}
        }
        text
    }

    /// Each shortcut off a Mac: its keys, and its context.
    fn on_linux() -> Vec<(Vec<Keystroke>, String)> {
        bindings()
            .iter()
            .map(|binding| {
                let keys = binding
                    .keystrokes()
                    .iter()
                    .map(|keystroke| {
                        let typed = off_mac(keystroke.inner(), OperatingSystem::Linux);
                        Keystroke::parse_with_os(&typed, OperatingSystem::Linux)
                            .unwrap_or_else(|error| panic!("{typed} parses: {error:?}"))
                    })
                    .collect();
                (keys, format!("{:?}", binding.predicate()))
            })
            .collect()
    }

    #[test]
    fn off_a_mac_no_shortcut_needs_the_super_key_and_each_command_chord_uses_ctrl() {
        for (binding, (keys, _)) in bindings().iter().zip(on_linux()) {
            for (written, typed) in binding.keystrokes().iter().zip(&keys) {
                assert!(!typed.modifiers.platform, "{typed:?} needs no Super key");
                if written.inner().modifiers.platform {
                    assert!(typed.modifiers.control, "{typed:?} is typed with Ctrl");
                    assert!(typed.modifiers.shift, "{typed:?} leaves plain Ctrl to the shell");
                }
            }
        }
    }

    #[test]
    fn off_a_mac_no_two_shortcuts_share_a_chord() {
        let mut seen = BTreeSet::new();
        for (keys, context) in on_linux() {
            let chord: Vec<String> = keys
                .iter()
                .map(|keystroke| keystroke.unparse(OperatingSystem::Linux))
                .collect();
            assert!(
                seen.insert((chord.clone(), context.clone())),
                "{chord:?} in {context} is bound twice"
            );
        }
    }

    #[test]
    fn a_chord_without_command_is_typed_as_written() {
        for written in ["ctrl-c", "escape", "up", "enter"] {
            let keystroke = Keystroke::parse_with_os(written, OperatingSystem::Linux)
                .unwrap_or_else(|error| panic!("{written} parses: {error:?}"));
            let typed = off_mac(&keystroke, OperatingSystem::Linux);
            let back = Keystroke::parse_with_os(&typed, OperatingSystem::Linux)
                .unwrap_or_else(|error| panic!("{typed} parses: {error:?}"));
            assert_eq!(back.modifiers, keystroke.modifiers, "{written}");
            assert_eq!(back.key, keystroke.key, "{written}");
        }
    }

    #[test]
    fn off_a_mac_each_command_label_names_the_chord_the_binding_is_typed_with() {
        let mut checked = 0;
        for binding in bindings() {
            for written in binding.keystrokes() {
                if !written.inner().modifiers.platform {
                    continue;
                }
                let typed = off_mac(written.inner(), OperatingSystem::Linux);
                let typed = Keystroke::parse_with_os(&typed, OperatingSystem::Linux)
                    .unwrap_or_else(|error| panic!("{typed} parses: {error:?}"));
                assert_eq!(
                    label(&mac_glyphs(written.inner()), false),
                    linux_style(&typed)
                );
                checked += 1;
            }
        }
        assert!(checked > 10, "the command chords were checked: {checked}");
    }

    #[test]
    fn off_a_mac_every_command_part_of_the_shortcuts_sheet_is_a_bound_chord() {
        let mut bound = BTreeSet::new();
        for binding in bindings() {
            for keystroke in binding.keystrokes() {
                if keystroke.inner().modifiers.platform {
                    bound.insert(mac_glyphs(keystroke.inner()));
                }
            }
        }
        for (keys, _) in SHORTCUTS {
            for part in keys.split(' ').filter(|part| part.contains('⌘')) {
                assert!(bound.contains(part), "{part} on the sheet is bound");
                assert!(
                    !label(part, false).contains('⌘'),
                    "{part} is written without the Mac key"
                );
            }
        }
    }

    #[test]
    fn off_a_mac_a_group_of_chords_maps_each_part() {
        assert_eq!(
            label("⌘1 ⌘2 ⌘3", false),
            "Ctrl+Shift+1 Ctrl+Shift+2 Ctrl+Shift+3"
        );
    }

    #[test]
    fn a_label_without_command_is_unchanged() {
        for glyphs in ["Esc", "↑ ↓ Home End", "Enter", "← →", "This Mac · online"] {
            assert_eq!(label(glyphs, false), glyphs);
        }
    }

    #[test]
    fn on_a_mac_every_label_is_unchanged() {
        for (glyphs, _) in SHORTCUTS {
            assert_eq!(label(glyphs, true), glyphs);
        }
    }
}
