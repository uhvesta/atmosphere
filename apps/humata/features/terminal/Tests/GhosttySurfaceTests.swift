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
        var exited: Int32?? = nil
        view.exited = { exited = .some($0) }

        spin(for: 5) { text(view).contains("hello from ghostty") || exited != nil }
        spin(for: 0.5) { exited != nil }

        XCTAssertTrue(text(view).contains("hello from ghostty"), "screen: \(text(view))")
        XCTAssertNil(exited, "exited with \(String(describing: exited))")
        view.close()
        window.close()
    }

    func testACommandThatEndsIsToldWithItsCode() throws {
        let (view, window) = try surface(running: "exit 3")
        var exited: Int32?? = nil
        view.exited = { exited = .some($0) }

        spin(for: 5) { exited != nil }

        XCTAssertEqual(exited, .some(3))
        view.close()
        window.close()
    }
}
