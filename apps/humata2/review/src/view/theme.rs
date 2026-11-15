use gpui_kit::{Rgba, SharedString, rgb, rgba};

/// How the pane draws. The defaults are the prototype's dark colours.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// The monospace family diff lines are drawn in.
    pub mono: SharedString,
    /// Text size, in pixels.
    pub font_size: f32,
    /// The height of a diff line, in pixels.
    pub line_height: f32,
    pub background: Rgba,
    pub sidebar: Rgba,
    pub panel: Rgba,
    pub panel_raised: Rgba,
    pub line: Rgba,
    pub text: Rgba,
    pub dim: Rgba,
    pub faint: Rgba,
    pub accent: Rgba,
    pub added: Rgba,
    pub removed: Rgba,
    pub warning: Rgba,
    pub added_background: Rgba,
    pub removed_background: Rgba,
    pub selected_background: Rgba,
    /// Lines the agent highlighted (PR-23).
    pub highlight_background: Rgba,
    pub changed_mark: Rgba,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            mono: if cfg!(target_os = "macos") {
                "Menlo".into()
            } else {
                "monospace".into()
            },
            font_size: 12.0,
            line_height: 18.0,
            background: rgb(0x1a1d21),
            sidebar: rgb(0x121417),
            panel: rgb(0x202328),
            panel_raised: rgb(0x262a30),
            line: rgb(0x33373e),
            text: rgb(0xe6e7e9),
            dim: rgb(0x9aa0a8),
            faint: rgb(0x6b7179),
            accent: rgb(0x5b9dff),
            added: rgb(0x3fb950),
            removed: rgb(0xf05d5d),
            warning: rgb(0xe3a13b),
            added_background: rgba(0x3fb9501f),
            removed_background: rgba(0xf05d5d1f),
            selected_background: rgba(0x5b9dff33),
            highlight_background: rgba(0xe0b04a2e),
            changed_mark: rgb(0xb58cff),
        }
    }
}
