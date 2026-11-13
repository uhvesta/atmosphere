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
        light: .init(background: "#fafafa", foreground: "#101010", colors: ["#000001", "#aa0002"]),
        dark: .init(background: "#0b0b0b", foreground: "#efefef"))

    func testTheFontAndTheColorsOfTheWindowsAppearanceAreSet() throws {
        let light = settings(style.config(dark: false))
        let dark = settings(style.config(dark: true))

        XCTAssertEqual(light["font-family"]?.last, "\"Iosevka Term\"")
        XCTAssertEqual(light["font-size"]?.last.flatMap { Double($0) }, 14)
        XCTAssertEqual(light["background"], ["#fafafa"])
        XCTAssertEqual(light["foreground"], ["#101010"])
        XCTAssertEqual(light["palette"], ["0=#000001", "1=#aa0002"])
        XCTAssertEqual(dark["background"], ["#0b0b0b"])
        XCTAssertEqual(dark["foreground"], ["#efefef"])
        XCTAssertNil(dark["palette"])
    }

    func testOnlyCopyPasteAndSelectAllAreBoundAfterClearingGhosttysOwn() throws {
        let binds = try XCTUnwrap(settings(style.config(dark: true))["keybind"])

        XCTAssertEqual(binds.first, "clear")
        XCTAssertEqual(
            Set(binds.dropFirst()), ["super+c=copy_to_clipboard", "super+v=paste_from_clipboard", "super+a=select_all"])
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
}
