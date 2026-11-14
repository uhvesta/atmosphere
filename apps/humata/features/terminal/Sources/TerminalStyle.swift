import Foundation

/// How terminals look: the font and a color theme. Ghostty reads only the
/// config written from this, never its own files.
package struct TerminalStyle: Equatable, Sendable {
    /// The colors for one appearance, each as `#rrggbb`.
    package struct Palette: Equatable, Sendable {
        package var background: String
        package var foreground: String
        /// The ANSI colors from 0 on; those not given stay Ghostty's.
        package var colors: [String]
        /// The cursor's color; nil leaves it Ghostty's.
        package var cursor: String?
        /// What selected text is drawn on; nil leaves it Ghostty's.
        package var selection: String?

        package init(
            background: String, foreground: String, colors: [String] = [], cursor: String? = nil,
            selection: String? = nil
        ) {
            self.background = background
            self.foreground = foreground
            self.colors = colors
            self.cursor = cursor
            self.selection = selection
        }
    }

    /// The sizes Settings offers, in points.
    package static let fontSizes: ClosedRange<Double> = 9...24

    package var fontFamily: String
    package var fontSize: Double
    package var theme: TerminalTheme
    /// The window's text scale, as a percentage: Bigger Text and Smaller
    /// Text in the View menu scale the terminals' font with the chat's.
    /// Settings neither shows nor keeps it.
    package var textPercent = 100

    /// Menlo at 12 points, which every Mac has, in Humata's own colors.
    package init(fontFamily: String = "Menlo", fontSize: Double = 12, theme: TerminalTheme = .humata) {
        self.fontFamily = fontFamily
        self.fontSize = fontSize
        self.theme = theme
    }

    /// The whole Ghostty config for a `dark` or light window: the font and
    /// colors, only ⌘C, ⌘V, ⌘A and ⌘F bound so Humata keeps its own
    /// shortcuts, and no other file included.
    package func config(dark: Bool) -> String {
        let palette = dark ? theme.dark : theme.light
        var lines = [
            // An empty family first drops Ghostty's own, so this is the
            // only one.
            "font-family = \"\"",
            "font-family = \"\(Self.value(fontFamily))\"",
            "font-size = \(fontSize * Double(textPercent) / 100)",
            "background = \(Self.value(palette.background))",
            "foreground = \(Self.value(palette.foreground))",
        ]
        for (index, color) in palette.colors.prefix(256).enumerated() {
            lines.append("palette = \(index)=\(Self.value(color))")
        }
        if let cursor = palette.cursor { lines.append("cursor-color = \(Self.value(cursor))") }
        if let selection = palette.selection { lines.append("selection-background = \(Self.value(selection))") }
        lines += [
            "keybind = clear",
            "keybind = super+c=copy_to_clipboard",
            "keybind = super+v=paste_from_clipboard",
            "keybind = super+a=select_all",
            "keybind = super+f=start_search",
            "shell-integration = none",
            "confirm-close-surface = false",
            "macos-option-as-alt = true",
            "clipboard-read = ask",
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
        kept.append(
            contentsOf: text.unicodeScalars.filter {
                !CharacterSet.newlines.contains($0) && $0 != "\"" && $0 != "\\"
            })
        return String(kept).trimmingCharacters(in: .whitespaces)
    }
}

// MARK: - Kept in UserDefaults

extension TerminalStyle {
    /// Where the window's text scale is kept, as a whole percentage. The
    /// View menu's commands write it; nothing here does.
    package static let textPercentKey = "textScalePercent"
    /// The scales the font follows; one kept outside them is held to them.
    private static let textPercents = 50...300

    private enum Key {
        static let fontFamily = "terminalFontFamily"
        static let fontSize = "terminalFontSize"
        static let theme = "terminalTheme"
    }

    /// The style Settings last saved in `defaults`; whatever is missing, or
    /// no longer makes sense, is the default's.
    package init(defaults: UserDefaults) {
        self.init()
        if let family = defaults.string(forKey: Key.fontFamily), !family.isEmpty { fontFamily = family }
        if let size = defaults.object(forKey: Key.fontSize) as? Double {
            fontSize = min(max(size, Self.fontSizes.lowerBound), Self.fontSizes.upperBound)
        }
        if let id = defaults.string(forKey: Key.theme), let saved = TerminalTheme.all.first(where: { $0.id == id }) {
            theme = saved
        }
        // None kept reads as 0, which is the usual size.
        let percent = defaults.integer(forKey: Self.textPercentKey)
        if percent > 0 {
            textPercent = min(max(percent, Self.textPercents.lowerBound), Self.textPercents.upperBound)
        }
    }

    /// Keeps the style in `defaults`, the theme by its id.
    package func save(to defaults: UserDefaults) {
        defaults.set(fontFamily, forKey: Key.fontFamily)
        defaults.set(fontSize, forKey: Key.fontSize)
        defaults.set(theme.id, forKey: Key.theme)
    }
}

// MARK: - Themes

/// A named pair of palettes, one for a light window and one for a dark.
package struct TerminalTheme: Equatable, Identifiable, Sendable {
    package var id: String
    package var name: String
    package var light: TerminalStyle.Palette
    package var dark: TerminalStyle.Palette

    package init(id: String, name: String, light: TerminalStyle.Palette, dark: TerminalStyle.Palette) {
        self.id = id
        self.name = name
        self.light = light
        self.dark = dark
    }

    /// The themes Settings offers, the default first. Their colors are
    /// Humata's own.
    package static let all = [humata, harbor, highContrast]

    /// Dark on white, or light on near black, with the Mac's blue.
    package static let humata = TerminalTheme(
        id: "humata", name: "Humata",
        light: .init(
            background: "#ffffff", foreground: "#1d1d1f",
            colors: [
                "#1d1d1f", "#c4271c", "#1f7a34", "#9a6700", "#1a5fd0", "#9333b8", "#0f7b8a", "#c9c9ce",
                "#6e6e73", "#e0443a", "#2a9d48", "#b88100", "#3b82f6", "#b455d6", "#1a9cae", "#f2f2f4",
            ],
            cursor: "#1a5fd0", selection: "#cfe0fb"),
        dark: .init(
            background: "#1e1e1e", foreground: "#e6e6e6",
            colors: [
                "#2a2a2c", "#ff6b5e", "#5fd27a", "#f0c24b", "#6aa8ff", "#d38cf0", "#5fd0df", "#d4d4d8",
                "#77777d", "#ff8f85", "#86e39b", "#f7d67a", "#93c0ff", "#e2aef6", "#8be0ec", "#ffffff",
            ],
            cursor: "#6aa8ff", selection: "#33496b"))

    /// Low contrast and warm: slate on cream, or sand on deep teal.
    package static let harbor = TerminalTheme(
        id: "harbor", name: "Harbor",
        light: .init(
            background: "#fbf5e6", foreground: "#4f5b62",
            colors: [
                "#2c3a40", "#cf3f34", "#6f8f12", "#a97a08", "#2f7fc4", "#c2427c", "#2f968c", "#e6dfcc",
                "#6b7a80", "#e05a4f", "#86a626", "#c49213", "#4c97d8", "#d65f94", "#46ada3", "#fffaf0",
            ],
            cursor: "#2f7fc4", selection: "#e9e0c6"),
        dark: .init(
            background: "#0f2a33", foreground: "#a9b9bb",
            colors: [
                "#0a2129", "#e2584c", "#93ad2c", "#c9981f", "#4a9be0", "#d9619a", "#45b3a8", "#dcd5c1",
                "#4f6a73", "#f07a6f", "#abc54a", "#e0b340", "#72b4ee", "#e884b2", "#68cbc0", "#fbf5e6",
            ],
            cursor: "#45b3a8", selection: "#1d4350"))

    /// Black on white, or white on black, with colors as far from the
    /// background as they go.
    package static let highContrast = TerminalTheme(
        id: "high-contrast", name: "High Contrast",
        light: .init(
            background: "#ffffff", foreground: "#000000",
            colors: [
                "#000000", "#a00000", "#006400", "#6b4e00", "#0000b8", "#7a007a", "#005f6b", "#bfbfbf",
                "#3a3a3a", "#c00000", "#007a00", "#7d5c00", "#0020e0", "#9a009a", "#00707e", "#ffffff",
            ],
            cursor: "#000000", selection: "#b3d0ff"),
        dark: .init(
            background: "#000000", foreground: "#ffffff",
            colors: [
                "#000000", "#ff6060", "#40ff70", "#ffe14d", "#70a5ff", "#ff7cff", "#50f0ff", "#e6e6e6",
                "#8c8c8c", "#ff8a8a", "#7dffa0", "#fff08a", "#9cc2ff", "#ffa8ff", "#8cf6ff", "#ffffff",
            ],
            cursor: "#ffffff", selection: "#2f4f9a"))
}
