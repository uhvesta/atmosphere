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
    /// Cohesive dark or light terminal colors: default foreground/background
    /// and a 256-color palette whose 16 ANSI colors are tuned for the
    /// background (all Light foregrounds meet WCAG AA >= 4.5:1 on `#ffffff`).
    pub fn for_theme(dark: bool) -> Self {
        terminal_colors(dark)
    }

    /// Whether the default background is light (relative luminance >= 0.5).
    pub fn is_light(&self) -> bool {
        relative_luminance(self.background) >= 0.5
    }

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

/// Cohesive Dark or Light terminal colors.
///
/// Light mode uses `#ffffff` background, `#1d1d1f` foreground, and a 16-color
/// ANSI palette where every color meets WCAG AA (>= 4.5:1 contrast) on
/// `#ffffff`. Dark mode uses `#1a1a1b` background, `#dcdcde` foreground, and
/// a cohesive dark ANSI palette.
pub fn terminal_colors(dark: bool) -> Colors {
    const DARK_NAMED: [u32; 16] = [
        0x282c34, 0xe06c75, 0x98c379, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xc8ccd4,
        0x5c6370, 0xf07178, 0xa8d488, 0xf0cf8a, 0x7cc0ff, 0xd68ef0, 0x6cd0dc, 0xeaeaea,
    ];
    const LIGHT_NAMED: [u32; 16] = [
        0x1d1d1f, 0xb82020, 0x2d6a1e, 0x7c5800, 0x1e52a8, 0x7a3b9e, 0x00686e, 0x5c5c62,
        0x595960, 0xc22727, 0x357a24, 0x876000, 0x1a5fd4, 0x8c42b5, 0x00767d, 0x3a3a40,
    ];
    if dark {
        Colors {
            foreground: Rgb::hex(0xdcdcde),
            background: Rgb::hex(0x1a1a1b),
            cursor: None,
            palette: palette_with_named(DARK_NAMED),
        }
    } else {
        Colors {
            foreground: Rgb::hex(0x1d1d1f),
            background: Rgb::hex(0xffffff),
            cursor: None,
            palette: palette_with_named(LIGHT_NAMED),
        }
    }
}

/// The xterm 256-color palette with `Ghostty`'s first 16 entries.
pub fn default_palette() -> [Rgb; 256] {
    const NAMED: [u32; 16] = [
        0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6,
        0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea,
    ];
    palette_with_named(NAMED)
}

fn palette_with_named(named: [u32; 16]) -> [Rgb; 256] {
    const LEVELS: [u8; 6] = [0x00, 0x5f, 0x87, 0xaf, 0xd7, 0xff];
    let mut palette = [Rgb::default(); 256];
    for (entry, value) in palette.iter_mut().zip(named) {
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

fn relative_luminance(rgb: Rgb) -> f64 {
    fn channel(c: u8) -> f64 {
        let s = f64::from(c) / 255.0;
        if s <= 0.03928 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contrast_ratio(a: Rgb, b: Rgb) -> f64 {
        let la = relative_luminance(a);
        let lb = relative_luminance(b);
        let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn light_terminal_colors_meet_wcag_aa_on_white() {
        let light = terminal_colors(false);
        assert_eq!(light, Colors::for_theme(false));
        assert!(light.is_light());
        assert_eq!(light.background, Rgb::hex(0xffffff));
        assert!(
            contrast_ratio(light.foreground, light.background) >= 4.5,
            "light foreground contrast too low"
        );
        for (i, &color) in light.palette[..16].iter().enumerate() {
            let ratio = contrast_ratio(color, light.background);
            assert!(
                ratio >= 4.5,
                "light ANSI palette[{i}] ({color:?}) contrast {ratio:.2} < 4.5 on #ffffff"
            );
        }
    }

    #[test]
    fn dark_terminal_colors_are_cohesive_and_avoid_neon_green() {
        let dark = terminal_colors(true);
        assert_eq!(dark, Colors::for_theme(true));
        assert!(!dark.is_light());
        assert_ne!(dark.palette[2], Rgb::hex(0x30d158));
        assert!(contrast_ratio(dark.foreground, dark.background) >= 10.0);
    }
}
