use crate::cell::Color;

/// A 24-bit color.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    /// The color written as `0xRRGGBB`.
    pub const fn hex(value: u32) -> Self {
        Rgb {
            r: (value >> 16) as u8,
            g: (value >> 8) as u8,
            b: value as u8,
        }
    }
}

/// The colors a terminal draws with when a cell doesn't name its own: the
/// default foreground and background, the cursor's color and the 256-entry
/// palette that [`Color::Palette`] indexes. Programs can change any of them
/// (OSC 4, 10, 11, 12), so the grid reports them each frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colors {
    pub foreground: Rgb,
    pub background: Rgb,
    /// The cursor's color, or `None` to draw it in the foreground color of
    /// the cell under it, as `Ghostty` does by default.
    pub cursor: Option<Rgb>,
    pub palette: [Rgb; 256],
}

impl Colors {
    /// The color `color` names, or `None` for [`Color::Default`], which
    /// the caller replaces with the default foreground or background.
    pub fn resolve(&self, color: Color) -> Option<Rgb> {
        match color {
            Color::Default => None,
            Color::Palette(index) => Some(self.palette[usize::from(index)]),
            Color::Rgb { r, g, b } => Some(Rgb { r, g, b }),
        }
    }
}

impl Default for Colors {
    /// `Ghostty`'s built-in theme: its 16 named colors, the 6x6x6 color cube
    /// and the 24-step gray ramp.
    fn default() -> Self {
        Colors {
            foreground: Rgb::hex(0xffffff),
            background: Rgb::hex(0x282c34),
            cursor: None,
            palette: default_palette(),
        }
    }
}

/// The xterm 256-color palette with `Ghostty`'s first 16 entries.
pub fn default_palette() -> [Rgb; 256] {
    const NAMED: [u32; 16] = [
        0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6,
        0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
    ];
    const LEVELS: [u8; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    let mut palette = [Rgb::default(); 256];
    for (entry, value) in palette.iter_mut().zip(NAMED) {
        *entry = Rgb::hex(value);
    }
    for index in 0..216 {
        palette[16 + index] = Rgb {
            r: LEVELS[index / 36],
            g: LEVELS[index / 6 % 6],
            b: LEVELS[index % 6],
        };
    }
    for step in 0..24 {
        let level = 8 + 10 * step as u8;
        palette[232 + step] = Rgb {
            r: level,
            g: level,
            b: level,
        };
    }
    palette
}
