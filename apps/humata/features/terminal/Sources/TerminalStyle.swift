import Foundation

/// How terminals look: the font and the colors for a light and a dark
/// window. Ghostty reads only the config written from this, never its own
/// files.
package struct TerminalStyle: Equatable, Sendable {
    /// The colors for one appearance, each as `#rrggbb`.
    package struct Palette: Equatable, Sendable {
        package var background: String
        package var foreground: String
        /// The ANSI colors from 0 on; those not given stay Ghostty's.
        package var colors: [String]

        package init(background: String, foreground: String, colors: [String] = []) {
            self.background = background
            self.foreground = foreground
            self.colors = colors
        }
    }

    package var fontFamily: String
    package var fontSize: Double
    package var light: Palette
    package var dark: Palette

    /// Menlo at 12 points, which every Mac has, dark on white or light on
    /// near black.
    package init(
        fontFamily: String = "Menlo", fontSize: Double = 12,
        light: Palette = Palette(background: "#ffffff", foreground: "#1d1d1f"),
        dark: Palette = Palette(background: "#1e1e1e", foreground: "#e6e6e6")
    ) {
        self.fontFamily = fontFamily
        self.fontSize = fontSize
        self.light = light
        self.dark = dark
    }

    /// The whole Ghostty config for a `dark` or light window: the font and
    /// colors, only ⌘C, ⌘V and ⌘A bound so Humata keeps its own shortcuts,
    /// and no other file included.
    package func config(dark: Bool) -> String {
        let palette = dark ? self.dark : light
        var lines = [
            // An empty family first drops Ghostty's own, so this is the
            // only one.
            "font-family = \"\"",
            "font-family = \"\(Self.value(fontFamily))\"",
            "font-size = \(fontSize)",
            "background = \(Self.value(palette.background))",
            "foreground = \(Self.value(palette.foreground))",
        ]
        for (index, color) in palette.colors.prefix(256).enumerated() {
            lines.append("palette = \(index)=\(Self.value(color))")
        }
        lines += [
            "keybind = clear",
            "keybind = super+c=copy_to_clipboard",
            "keybind = super+v=paste_from_clipboard",
            "keybind = super+a=select_all",
            "shell-integration = none",
            "confirm-close-surface = false",
            "macos-option-as-alt = true",
            "clipboard-read = deny",
            "clipboard-write = allow",
            "copy-on-select = false",
            "window-padding-x = 4",
            "window-padding-y = 4",
        ]
        return lines.joined(separator: "\n") + "\n"
    }

    /// `text` as a value on one config line: without line breaks, which
    /// would start another setting, or quotes.
    private static func value(_ text: String) -> String {
        var kept = String.UnicodeScalarView()
        kept.append(contentsOf: text.unicodeScalars.filter {
            !CharacterSet.newlines.contains($0) && $0 != "\"" && $0 != "\\"
        })
        return String(kept).trimmingCharacters(in: .whitespaces)
    }
}
