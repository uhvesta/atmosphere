import HumataTerminal
import XCTest

final class TerminalStyleTests: XCTestCase {
    /// Each setting of a Ghostty config with every value given for it, in
    /// order.
    private func settings(_ config: String) -> [String: [String]] {
        var settings: [String: [String]] = [:]
        for line in config.split(separator: "\n") {
            let parts = line.split(separator: "=", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            guard let key = parts.first else { continue }
            settings[key, default: []].append(parts.count > 1 ? parts[1] : "")
        }
        return settings
    }

    private let style = TerminalStyle(
        fontFamily: "Iosevka Term", fontSize: 14,
        theme: TerminalTheme(
            id: "test", name: "Test",
            light: .init(
                background: "#fafafa", foreground: "#101010", colors: ["#000001", "#aa0002"], cursor: "#0000c3",
                selection: "#d0d0d4"),
            dark: .init(background: "#0b0b0b", foreground: "#efefef")))

    /// Defaults of this test's own, emptied before and after.
    private func emptyDefaults() throws -> UserDefaults {
        let suite = "TerminalStyleTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        addTeardownBlock { UserDefaults().removePersistentDomain(forName: suite) }
        return defaults
    }

    func testTheFontAndTheColorsOfTheWindowsAppearanceAreSet() throws {
        let light = settings(style.config(dark: false))
        let dark = settings(style.config(dark: true))

        XCTAssertEqual(light["font-family"]?.last, "\"Iosevka Term\"")
        XCTAssertEqual(light["font-size"]?.last.flatMap { Double($0) }, 14)
        XCTAssertEqual(light["background"], ["#fafafa"])
        XCTAssertEqual(light["foreground"], ["#101010"])
        XCTAssertEqual(light["palette"], ["0=#000001", "1=#aa0002"])
        XCTAssertEqual(light["cursor-color"], ["#0000c3"])
        XCTAssertEqual(light["selection-background"], ["#d0d0d4"])
        XCTAssertEqual(dark["background"], ["#0b0b0b"])
        XCTAssertEqual(dark["foreground"], ["#efefef"])
        XCTAssertNil(dark["palette"])
        XCTAssertNil(dark["cursor-color"])
        XCTAssertNil(dark["selection-background"])
    }

    func testOnlyCopyPasteSelectAllAndFindAreBoundAfterClearingGhosttysOwn() throws {
        let binds = try XCTUnwrap(settings(style.config(dark: true))["keybind"])

        XCTAssertEqual(binds.first, "clear")
        XCTAssertEqual(
            Set(binds.dropFirst()),
            [
                "super+c=copy_to_clipboard", "super+v=paste_from_clipboard", "super+a=select_all",
                "super+f=start_search",
            ])
    }

    func testNoOtherFileIsIncludedWhateverAFontIsNamed() throws {
        var sneaky = style
        sneaky.fontFamily = "Menlo\"\nconfig-file = /tmp/other\nkeybind = super+q=quit"

        let plain = settings(style.config(dark: true))
        let config = settings(sneaky.config(dark: true))
        XCTAssertNil(plain["config-file"])
        XCTAssertNil(config["config-file"])
        XCTAssertEqual(config["keybind"], plain["keybind"])
        XCTAssertEqual(Set(config.keys), Set(plain.keys))
    }

    func testNoBuiltInThemeIncludesAnotherFileOrNamesAThemeOfGhosttys() throws {
        for theme in TerminalTheme.all {
            for dark in [false, true] {
                let config = settings(TerminalStyle(theme: theme).config(dark: dark))
                XCTAssertNil(config["config-file"], theme.id)
                // `theme` would have Ghostty read a file of that name.
                XCTAssertNil(config["theme"], theme.id)
                XCTAssertEqual(config["palette"]?.count, 16, theme.id)
            }
        }
    }

    func testAThemesColorCannotStartAnotherSetting() throws {
        var sneaky = style
        sneaky.theme.dark.background = "#000000\nconfig-file = /tmp/other"
        sneaky.theme.dark.colors = ["#111111\ntheme = other"]
        sneaky.theme.dark.cursor = "#222222\rconfig-file = /tmp/other"

        let config = settings(sneaky.config(dark: true))
        XCTAssertNil(config["config-file"])
        XCTAssertNil(config["theme"])
    }

    func testThemesAreToldApartByTheirIDs() {
        XCTAssertEqual(Set(TerminalTheme.all.map(\.id)).count, TerminalTheme.all.count)
        XCTAssertEqual(TerminalStyle().theme, TerminalTheme.all.first)
    }

    func testASavedStyleIsReadBack() throws {
        let defaults = try emptyDefaults()
        XCTAssertEqual(TerminalStyle(defaults: defaults), TerminalStyle())

        let saved = TerminalStyle(fontFamily: "Monaco", fontSize: 16, theme: .highContrast)
        saved.save(to: defaults)
        XCTAssertEqual(TerminalStyle(defaults: defaults), saved)
    }

    func testTheFontIsScaledWithTheWindowsText() throws {
        var bigger = style
        bigger.textPercent = 150

        let config = settings(bigger.config(dark: false))
        XCTAssertEqual(config["font-size"]?.last.flatMap { Double($0) }, 21)
    }

    func testTheKeptTextScaleIsReadWithTheStyleAndSavingTheStyleLeavesIt() throws {
        let defaults = try emptyDefaults()
        XCTAssertEqual(TerminalStyle(defaults: defaults).textPercent, 100)

        defaults.set(130, forKey: TerminalStyle.textPercentKey)
        XCTAssertEqual(TerminalStyle(defaults: defaults).textPercent, 130)

        TerminalStyle(fontFamily: "Monaco", fontSize: 16, theme: .highContrast).save(to: defaults)
        let read = TerminalStyle(defaults: defaults)
        XCTAssertEqual(read.textPercent, 130)
        XCTAssertEqual(read.fontSize, 16)
    }

    func testWhatWasSavedWrongFallsBackToTheDefault() throws {
        let defaults = try emptyDefaults()
        TerminalStyle(
            fontFamily: "", fontSize: 300,
            theme: TerminalTheme(
                id: "gone", name: "Gone", light: .init(background: "#ffffff", foreground: "#000000"),
                dark: .init(background: "#000000", foreground: "#ffffff"))
        ).save(to: defaults)

        let read = TerminalStyle(defaults: defaults)
        XCTAssertEqual(read.fontFamily, TerminalStyle().fontFamily)
        XCTAssertEqual(read.fontSize, TerminalStyle.fontSizes.upperBound)
        XCTAssertEqual(read.theme, TerminalStyle().theme)
    }
}
