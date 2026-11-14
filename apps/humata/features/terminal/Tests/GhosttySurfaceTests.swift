import AppKit
import XCTest

@testable import HumataTerminal

/// A real Ghostty surface in a window kept off screen.
@MainActor
final class GhosttySurfaceTests: XCTestCase {
    private func surface(running line: String) throws -> (GhosttySurfaceView, NSWindow) {
        let runtime = try GhosttyRuntime.shared.get()
        let view = GhosttySurfaceView(runtime: runtime, command: TerminalCommand(line: line))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 400), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        return (view, window)
    }

    /// Runs the main run loop until `done` or `seconds` pass.
    private func spin(for seconds: TimeInterval, until done: () -> Bool) {
        let end = Date().addingTimeInterval(seconds)
        while !done(), Date() < end {
            RunLoop.main.run(until: Date().addingTimeInterval(0.05))
        }
    }

    private func text(_ view: GhosttySurfaceView) -> String {
        view.accessibilityValue() as? String ?? ""
    }

    func testARunningCommandShowsWhatItPrintsAndKeepsRunning() throws {
        let (view, window) = try surface(running: "printf 'hello from ghostty'; exec cat")
        var exited: Int32??
        view.exited = { exited = .some($0) }

        spin(for: 5) { text(view).contains("hello from ghostty") || exited != nil }
        spin(for: 0.5) { exited != nil }

        XCTAssertTrue(text(view).contains("hello from ghostty"), "screen: \(text(view))")
        XCTAssertNil(exited, "exited with \(String(describing: exited))")
        view.close()
        window.close()
    }

    private func type(_ characters: String, keyCode: UInt16, into view: GhosttySurfaceView) {
        for type in [NSEvent.EventType.keyDown, .keyUp] {
            guard
                let event = NSEvent.keyEvent(
                    with: type, location: .zero, modifierFlags: [], timestamp: 0,
                    windowNumber: view.window?.windowNumber ?? 0, context: nil, characters: characters,
                    charactersIgnoringModifiers: characters, isARepeat: false, keyCode: keyCode)
            else { return XCTFail("no key event for \(characters)") }
            if type == .keyDown { view.keyDown(with: event) } else { view.keyUp(with: event) }
        }
    }

    func testTypedKeysReachTheCommand() throws {
        let (view, window) = try surface(running: "exec cat")
        window.makeFirstResponder(view)
        spin(for: 0.5) { false }

        type("h", keyCode: 4, into: view)
        type("i", keyCode: 34, into: view)
        type("\r", keyCode: 36, into: view)
        // The pty echoes the line, then cat prints it back.
        spin(for: 5) { text(view).components(separatedBy: "hi").count > 2 }

        XCTAssertEqual(text(view).components(separatedBy: "hi").count, 3, "screen: \(text(view))")
        view.close()
        window.close()
    }

    func testFindCountsTheMatchesOnScreen() throws {
        let (view, window) = try surface(running: "printf 'one needle, two needle'; exec cat")
        let search = TerminalSearch()
        view.search = search
        search.view = view
        spin(for: 5) { text(view).contains("two needle") }

        search.needle = "needle"
        search.needleChanged()
        spin(for: 5) { search.total == 2 }

        XCTAssertEqual(search.total, 2, "screen: \(text(view))")
        view.close()
        window.close()
    }

    func testACommandThatEndsIsToldWithItsCode() throws {
        let (view, window) = try surface(running: "exit 3")
        var exited: Int32??
        view.exited = { exited = .some($0) }

        spin(for: 5) { exited != nil }

        XCTAssertEqual(exited, .some(3), "screen: \(text(view))")
        view.close()
        window.close()
    }

    func testAProgramReadingTheClipboardIsAskedAboutFirst() throws {
        let pasteboard = NSPasteboard.general
        let kept = pasteboard.string(forType: .string)
        defer {
            pasteboard.clearContents()
            if let kept { pasteboard.setString(kept, forType: .string) }
        }
        pasteboard.clearContents()
        pasteboard.setString("copied", forType: .string)
        let runtime = try GhosttyRuntime.shared.get()
        // OSC 52 asks for the clipboard.
        let line = "printf '\\033]52;c;?\\007'; exec cat"
        let view = GhosttySurfaceView(runtime: runtime, command: TerminalCommand(line: line))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 400), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view

        spin(for: 5) { window.attachedSheet != nil }

        XCTAssertNotNil(window.attachedSheet, "screen: \(text(view))")
        if let sheet = window.attachedSheet { window.endSheet(sheet, returnCode: .alertFirstButtonReturn) }
        spin(for: 0.5) { false }
        XCTAssertNil(window.attachedSheet)
        view.close()
        window.close()
    }

    /// A terminal's host, as its tab shows it, put in a window.
    private func shownTab(_ search: TerminalSearch, findBarUp: Bool) -> (TerminalSurfaceHost, NSWindow) {
        let host = TerminalSurfaceHost(frame: NSRect(x: 0, y: 0, width: 640, height: 400))
        host.show(TerminalCommand(line: "exec cat"), generation: 0, search: search) { _ in }
        if findBarUp {
            // As ⌘F does: Ghostty starts the search and tells the find bar.
            _ = host.surface?.perform("start_search")
            spin(for: 2) { search.shown }
        }
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 400), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        return (host, window)
    }

    func testAShownTerminalTakesTheKeys() throws {
        _ = try GhosttyRuntime.shared.get()
        let (host, window) = shownTab(TerminalSearch(), findBarUp: false)
        spin(for: 2) { host.surface.map { window.firstResponder === $0 } ?? false }

        XCTAssertNotNil(host.surface)
        XCTAssertTrue(host.surface.map { window.firstResponder === $0 } ?? false)
        host.close()
        window.close()
    }

    func testAShownTerminalLeavesTheKeysWithItsFindBar() throws {
        _ = try GhosttyRuntime.shared.get()
        let search = TerminalSearch()
        let (host, window) = shownTab(search, findBarUp: true)
        spin(for: 0.5) { false }

        XCTAssertNotNil(host.surface)
        XCTAssertTrue(search.shown)
        XCTAssertFalse(host.surface.map { window.firstResponder === $0 } ?? true)
        host.close()
        window.close()
    }
}
