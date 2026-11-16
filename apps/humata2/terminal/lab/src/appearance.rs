//! Host chrome follows the main app's D76 palette (ui/src/theme.rs).
#[derive(Clone, Copy)]
pub(super) struct Look {
    pub sidebar: u32,
    pub panel: u32,
    pub line: u32,
    pub text: u32,
    pub secondary: u32,
    pub accent: u32,
    pub fill: u32,
    pub card: u32,
    pub terminal: u32,
    pub terminal_text: u32,
}
impl Look {
    pub(super) fn new(dark: bool) -> Self {
        if dark {
            Self {
                sidebar: 0x2a2a2d,
                panel: 0x252527,
                line: 0x3a3a3d,
                text: 0xe8e8ea,
                secondary: 0x98989e,
                accent: 0x3b8cff,
                fill: 0x323235,
                card: 0x2a2a2c,
                terminal: 0x1a1a1b,
                terminal_text: 0xdcdcde,
            }
        } else {
            Self {
                sidebar: 0xe9e7ea,
                panel: 0xf6f5f7,
                line: 0xdcdadd,
                text: 0x1d1d1f,
                secondary: 0x6e6e73,
                accent: 0x0a6cff,
                fill: 0xeceaee,
                card: 0xffffff,
                terminal: 0xffffff,
                terminal_text: 0x1d1d1f,
            }
        }
    }
    pub(super) fn terminal(self) -> terminal_embed::Appearance {
        terminal_embed::Appearance {
            background: self.terminal,
            foreground: self.terminal_text,
            font_size: 13.,
            line_height: 19.,
            font_family: "Menlo".into(),
            ansi_colors: (self.terminal == 0xffffff).then_some([
                0x202020, 0xa52626, 0x426619, 0x806000, 0x2457a7, 0x79429c, 0x006b70, 0x666666,
                0x666666, 0xb52828, 0x4c701d, 0x886500, 0x225fc1, 0x8546a8, 0x007a80, 0xdddddd,
            ]),
        }
    }
}
