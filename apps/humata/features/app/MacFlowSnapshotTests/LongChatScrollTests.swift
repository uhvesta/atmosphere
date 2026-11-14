import ComposableArchitecture
import HumataAppFeature
import HumataDaemonClient
import HumataDiagram
import HumataSession
import SwiftUI
import XCTest

/// The chat's rows keep to the heights they have: a tool call opened makes
/// room for its output and gives it back, and a diagram drawn after its row
/// was laid out gets the room it needs. A long chat goes to a row far up it
/// and stays there, and earlier rows asked for come above what's in view.
/// What scrolling costs is measured in `PerfTests`.
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

    /// A search hit far up a chat longer than the rows it opens with is
    /// what the chat shows, and goes on showing: the rows above its last
    /// come as it's gone to, and aren't taken for the chat growing at its
    /// end.
    func testASearchHitFarUpALongChatIsShownNotTheChatsEnd() async throws {
        // The message that starts the 50th of 1,000 turns: about row 100.
        let (_, window, task) = try await opened(Self.long(), highlighted: "x200")
        defer {
            task.cancel()
            window.close()
            window.contentViewController = nil
        }
        let scroll = try XCTUnwrap(chatScroll(in: window.contentView), "no scroll view holds the chat")

        await settle(window)
        XCTAssertGreaterThan(belowView(scroll), 200, "the chat is at its end, not at what the search found")
        await settle(window)
        XCTAssertGreaterThan(belowView(scroll), 200, "the chat went back to its end")
    }

    /// Showing a waiting proposal far up a long chat goes to it from the
    /// chat's end, and stays.
    func testShowingAProposalFarUpGoesToItFromTheChatsEnd() async throws {
        let (store, window, task) = try await opened(Self.long(proposingInTurn: 50))
        defer {
            task.cancel()
            window.close()
            window.contentViewController = nil
        }
        let scroll = try XCTUnwrap(chatScroll(in: window.contentView), "no scroll view holds the chat")
        XCTAssertLessThan(belowView(scroll), 60, "the chat didn't open at its end")

        store.send(.proposals(.listed([Proposal(id: "proposed", title: "Login form")], changes: [:])))
        await settle(window)
        store.send(.showProposalsTapped)
        // Twice: the chat is taken there with an animation.
        await settle(window)
        await settle(window)
        XCTAssertGreaterThan(belowView(scroll), 200, "the chat is at its end, not at the proposal")
        await settle(window)
        XCTAssertGreaterThan(belowView(scroll), 200, "the chat went back to its end")
    }

    /// Show Earlier Messages brings rows above those the chat opened with,
    /// and what was in view stays in view.
    func testShowingEarlierMessagesKeepsWhatWasInView() async throws {
        let (_, window, task) = try await opened(Self.long())
        defer {
            task.cancel()
            window.close()
            window.contentViewController = nil
        }
        let scroll = try XCTUnwrap(chatScroll(in: window.contentView), "no scroll view holds the chat")
        func height() -> CGFloat { scroll.documentView?.frame.height ?? 0 }
        // To the first of the rows it opened with, where the button is.
        // Twice: the rows passed on the way are measured, which moves them.
        for _ in 0..<2 {
            scrollToTop(scroll)
            await settle(window)
        }
        let button = try XCTUnwrap(
            element(identified: "chat.earlier", under: window.contentView),
            "a chat of 2,000 rows doesn't offer its earlier ones")
        let before = (height: height(), aboveEnd: aboveEnd(scroll))

        XCTAssertTrue(button.accessibilityPerformPress(), "Show Earlier Messages couldn't be pressed")
        await settle(window)
        await settle(window)

        XCTAssertGreaterThan(height(), before.height + 500, "no earlier rows came")
        // What's in view is as far from the chat's end as it was, within a
        // row: the rows came above it.
        XCTAssertEqual(aboveEnd(scroll), before.aboveEnd, accuracy: 60, "what was in view moved")
    }

    /// A proposed spec edit shows what it takes out of the spec and puts
    /// in, inside its card: the chat makes room for it, where a spinner
    /// left in its place would take a line.
    func testAProposedSpecEditShowsItsChangeAndNoSpinner() async throws {
        let current = (1...8).map { "The form checks field \($0) as it is typed in." }.joined(separator: "\n")
        let spec = (1...8).map { "Field \($0) is checked once the form is sent." }.joined(separator: "\n")
        var history: [SessionEvent] = []
        func add(_ kind: SessionEvent.Kind) {
            history.append(SessionEvent(id: "x\(history.count)", time: Date(timeIntervalSince1970: 0), kind: kind))
        }
        add(.userMessage("Tighten the login form's spec."))
        add(.turnStarted)
        add(.toolStarted(callID: "proposed", tool: "propose", argumentsJSON: "{}"))
        add(.toolCompleted(callID: "proposed", success: true, output: "Proposed."))
        add(.assistantMessage("Here's a tighter spec."))
        add(.turnEnded(aborted: false))
        let edit = Proposal(
            id: "proposed", kind: .specEdit, title: "Login form", spec: spec, itemID: "i1", currentSpec: current)
        let task = Proposal(id: "proposed", title: "Login form", spec: spec)

        /// The chat's height with `proposal` listed, and how many spinners
        /// it shows.
        func shown(_ proposal: Proposal) async throws -> (height: CGFloat, spinners: Int) {
            let (store, window, watching) = try await opened(history, proposals: [proposal])
            defer {
                watching.cancel()
                window.close()
                window.contentViewController = nil
            }
            store.send(.proposals(.refresh))
            let clock = ContinuousClock()
            let start = clock.now
            while store.proposals.proposals.isEmpty
                || proposal.currentSpec != nil && store.proposals.changes.isEmpty, clock.now - start < .seconds(20)
            {
                try await Task.sleep(for: .milliseconds(20))
            }
            XCTAssertEqual(store.proposals.proposals.map(\.id), [proposal.id])
            await settle(window)
            await settle(window)
            let scroll = try XCTUnwrap(chatScroll(in: window.contentView), "no scroll view holds the chat")
            return (scroll.documentView?.frame.height ?? 0, spinners(in: scroll))
        }

        let asEdit = try await shown(edit)
        let asTask = try await shown(task)

        XCTAssertEqual(asEdit.spinners, 0, "the spec edit is still on its spinner")
        // Sixteen lines taken out and put in, against the task's eight.
        XCTAssertGreaterThan(asEdit.height, asTask.height + 40, "the chat made no room for what the edit changes")
    }

    /// A chat of 1,000 short turns, each a message and a reply: 2,000 rows,
    /// far more than a chat opens with. Each turn is four events, so turn
    /// `n`'s message is the event `x(4n)`. The turn `proposingInTurn` also
    /// makes the tool call `proposed`.
    private static func long(proposingInTurn proposing: Int? = nil) -> [SessionEvent] {
        var history: [SessionEvent] = []
        func add(_ kind: SessionEvent.Kind) {
            history.append(SessionEvent(id: "x\(history.count)", time: Date(timeIntervalSince1970: 0), kind: kind))
        }
        for turn in 0..<1_000 {
            add(.userMessage("Check fixture \(turn)."))
            add(.turnStarted)
            if turn == proposing {
                add(.toolStarted(callID: "proposed", tool: "propose", argumentsJSON: "{}"))
                add(.toolCompleted(callID: "proposed", success: true, output: "Proposed."))
            }
            add(.assistantMessage("Fixture \(turn) parses."))
            add(.turnEnded(aborted: false))
        }
        return history
    }

    /// The chat of `history` in a window under every other one, opened:
    /// with the event `highlighted` as a search's hit, when one is given,
    /// and `proposals` as what the daemon lists for it.
    private func opened(
        _ history: [SessionEvent], highlighted: String? = nil, proposals: [Proposal] = []
    ) async throws -> (
        StoreOf<SessionFeature>, NSWindow, StoreTask
    ) {
        let session = Session(id: "s1", name: "long", backend: .scripted, workingDirectory: "/work")
        let store = Store(initialState: SessionFeature.State(session: session, highlighted: highlighted)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = .previewValue
            $0.daemon.proposals = { _ in proposals }
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

    /// How much of the chat is below what's in view: nothing at its end.
    private func belowView(_ scroll: NSScrollView) -> CGFloat {
        guard let document = scroll.documentView else { return 0 }
        let visible = scroll.documentVisibleRect
        return document.isFlipped ? document.frame.height - visible.maxY : visible.minY
    }

    /// How far above the chat's end the top of what's in view is.
    private func aboveEnd(_ scroll: NSScrollView) -> CGFloat {
        guard let document = scroll.documentView else { return 0 }
        let visible = scroll.documentVisibleRect
        return document.isFlipped ? document.frame.height - visible.minY : visible.maxY
    }

    /// Scrolls the chat to the first of the rows its list holds.
    private func scrollToTop(_ scroll: NSScrollView) {
        guard let document = scroll.documentView else { return }
        let top = document.isFlipped ? 0 : max(0, document.frame.height - scroll.contentView.bounds.height)
        scroll.contentView.scroll(to: NSPoint(x: 0, y: top))
        scroll.reflectScrolledClipView(scroll.contentView)
    }

    /// The control named `identifier` under `root`, as assistive
    /// technology finds it.
    private func element(identified identifier: String, under root: Any?, depth: Int = 0) -> NSAccessibilityProtocol? {
        guard depth < 60, let node = root as? NSAccessibilityProtocol else { return nil }
        if node.accessibilityIdentifier() == identifier { return node }
        for child in node.accessibilityChildren() ?? [] {
            if let found = element(identified: identifier, under: child, depth: depth + 1) { return found }
        }
        return nil
    }

    /// How many spinners or progress bars are drawn in the chat.
    private func spinners(in scroll: NSScrollView) -> Int {
        var count = 0
        func walk(_ view: NSView) {
            if view is NSProgressIndicator, !view.isHiddenOrHasHiddenAncestor { count += 1 }
            for child in view.subviews { walk(child) }
        }
        if let document = scroll.documentView { walk(document) }
        return count
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
