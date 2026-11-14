import ComposableArchitecture
import HumataAppFeature
import HumataDaemonClient
import HumataDiagram
import HumataSession
import SwiftUI
import XCTest

/// The chat's rows keep to the heights they have: a tool call opened makes
/// room for its output and gives it back, and a diagram drawn after its row
/// was laid out gets the room it needs. What scrolling costs is measured in
/// `PerfTests`.
@MainActor
final class LongChatScrollTests: XCTestCase {
    /// A tool call opened makes room for its output, and closing it gives
    /// the room back: the chat measures its rows once and keeps their
    /// heights, so opening one must count as a change. The chat fits in the
    /// window, so its height is measured, not estimated as a lazy list's is.
    func testOpeningAToolCallMakesRoomForItsOutput() async throws {
        var history: [SessionEvent] = []
        func add(_ kind: SessionEvent.Kind) {
            history.append(SessionEvent(id: "x\(history.count)", time: Date(timeIntervalSince1970: 0), kind: kind))
        }
        add(.userMessage("Run the parser's tests."))
        add(.turnStarted)
        let row = "x\(history.count)"
        add(.toolStarted(callID: "open-me", tool: "shell", argumentsJSON: #"{"command":"swift test"}"#))
        add(
            .toolCompleted(
                callID: "open-me", success: true,
                output: (0..<400).map { "test parser case \($0) passed" }.joined(separator: "\n")))
        add(.assistantMessage("All 400 cases pass."))
        add(.turnEnded(aborted: false))

        let (store, window, task) = try await opened(history)
        defer {
            task.cancel()
            window.close()
            window.contentViewController = nil
        }
        let scroll = try XCTUnwrap(chatScroll(in: window.contentView), "no scroll view holds the chat")
        func height() -> CGFloat { scroll.documentView?.frame.height ?? 0 }

        let closed = height()
        store.send(.toolToggled(id: row))
        await settle(window)
        let open = height()
        store.send(.toolToggled(id: row))
        await settle(window)

        XCTAssertGreaterThan(open, closed + 200, "the chat didn't grow to show the output")
        XCTAssertEqual(height(), closed, accuracy: 2, "the chat didn't give the room back")
    }

    /// A row that changes its own height isn't kept at the height it had:
    /// a diagram drawn after its row was laid out, as a spinner, makes the
    /// chat as tall as it is when it opens with the diagram drawn already.
    func testADiagramDrawnAfterItsRowWasLaidOutMakesRoomForIt() async throws {
        let source =
            "graph TD\n"
            + (0..<9).map { "  N\($0)[Step \($0)] --> N\($0 + 1)[Step \($0 + 1)]" }
            .joined(separator: "\n")
        let history = [
            SessionEvent(id: "x0", time: Date(timeIntervalSince1970: 0), kind: .userMessage("Draw the steps.")),
            SessionEvent(id: "x1", time: Date(timeIntervalSince1970: 0), kind: .turnStarted),
            SessionEvent(
                id: "x2", time: Date(timeIntervalSince1970: 0),
                kind: .assistantMessage("Here they are:\n\n```mermaid\n\(source)\n```\n\nTen steps.")),
            SessionEvent(id: "x3", time: Date(timeIntervalSince1970: 0), kind: .turnEnded(aborted: false)),
        ]
        func chatHeight(_ window: NSWindow) throws -> CGFloat {
            try XCTUnwrap(chatScroll(in: window.contentView), "no scroll view holds the chat").documentView?.frame
                .height ?? 0
        }

        let (_, drawnLater, task) = try await opened(history)
        let clock = ContinuousClock()
        let start = clock.now
        while MermaidRenderer.shared.cached(source, dark: false) == nil
            && MermaidRenderer.shared.cached(source, dark: true) == nil, clock.now - start < .seconds(20)
        {
            try await Task.sleep(for: .milliseconds(50))
        }
        await settle(drawnLater)
        let later = try chatHeight(drawnLater)
        task.cancel()
        drawnLater.close()
        drawnLater.contentViewController = nil

        let (_, drawnFirst, again) = try await opened(history)
        defer {
            again.cancel()
            drawnFirst.close()
            drawnFirst.contentViewController = nil
        }
        let first = try chatHeight(drawnFirst)

        XCTAssertGreaterThan(first, 200, "the diagram wasn't drawn")
        XCTAssertEqual(later, first, accuracy: 2, "the chat kept the spinner's height for the diagram")
    }

    /// The chat of `history` in a window under every other one, opened.
    private func opened(_ history: [SessionEvent]) async throws -> (
        StoreOf<SessionFeature>, NSWindow, StoreTask
    ) {
        let session = Session(id: "s1", name: "long", backend: .scripted, workingDirectory: "/work")
        let store = Store(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = .previewValue
            $0.daemon.watch = { _, _ in
                AsyncThrowingStream { continuation in
                    for event in history { continuation.yield(.event(event)) }
                    continuation.yield(.historyLoaded)
                }
            }
            $0.daemon.markSeen = { _, _ in }
        }
        let window = NSWindow(
            contentRect: CGRect(x: 80, y: 80, width: 1180, height: 760),
            styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) + 1)
        let hosting = NSHostingController(
            rootView: SessionView(store: store).environment(\.codeBlockRenderer, MermaidRenderer.codeBlocks))
        // The window keeps its size: the chat takes what it's offered and
        // asks for none, so sized to its content the window keeps only the
        // composer.
        hosting.sizingOptions = []
        window.contentViewController = hosting
        window.setContentSize(CGSize(width: 1180, height: 760))
        window.orderFrontRegardless()
        let task = store.send(.task)
        let clock = ContinuousClock()
        let start = clock.now
        while !store.historyLoaded || store.items.isEmpty, clock.now - start < .seconds(60) {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(store.historyLoaded)
        await settle(window)
        return (store, window, task)
    }

    /// Lets queued layout and drawing finish.
    private func settle(_ window: NSWindow) async {
        for _ in 0..<20 {
            window.contentView?.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    /// The scroll view with the tallest content: the chat's.
    private func chatScroll(in view: NSView?) -> NSScrollView? {
        guard let view else { return nil }
        var found: [NSScrollView] = []
        func walk(_ view: NSView) {
            if let scroll = view as? NSScrollView { found.append(scroll) }
            for child in view.subviews { walk(child) }
        }
        walk(view)
        return found.max { ($0.documentView?.frame.height ?? 0) < ($1.documentView?.frame.height ?? 0) }
    }
}
