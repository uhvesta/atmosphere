import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import SwiftUI
import XCTest

/// Scrolling a chat costs what's on screen, not how long the chat is: a
/// chat five times longer, of every kind of row, grows memory no more as
/// it's read up and down, and memory stops growing once it has been read.
/// How long steps take depends on the machine and what else runs on it,
/// so that is checked only when `HUMATA_PERF` is set
/// (`--test_env=HUMATA_PERF=1`).
@MainActor
final class LongChatScrollTests: XCTestCase {
    func testScrollingCostsWhatShowsNotHowLongTheChatIs() async throws {
        var shortChat = ChatMaker(seed: 7)
        var longChat = ChatMaker(seed: 7)
        let short = try await scrollThrough(shortChat.history(turns: 40))
        let long = try await scrollThrough(longChat.history(turns: 200))
        let report = "short: \(short)\nlong: \(long)"
        print(report)
        XCTContext.runActivity(named: report) { _ in }

        let megabyte: UInt64 = 1_048_576
        XCTAssertLessThan(long.grew, max(short.grew * 2, 150 * megabyte), report)
        XCTAssertLessThan(long.grewAgain, 100 * megabyte, "memory kept growing on a second read\n" + report)

        guard ProcessInfo.processInfo.environment["HUMATA_PERF"] != nil else { return }
        XCTAssertLessThan(long.median, Duration.milliseconds(16), report)
        XCTAssertLessThan(long.worst, Duration.milliseconds(250), report)
        XCTAssertLessThan(long.slow * 100, long.steps * 5, "more than 5% of steps were slow\n" + report)
        XCTAssertLessThan(long.median, short.median * 2 + Duration.milliseconds(4), report)
    }

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
        store.send(.toolToggled(id: "open-me"))
        await settle(window)
        let open = height()
        store.send(.toolToggled(id: "open-me"))
        await settle(window)

        XCTAssertGreaterThan(open, closed + 200, "the chat didn't grow to show the output")
        XCTAssertEqual(height(), closed, accuracy: 2, "the chat didn't give the room back")
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
        let hosting = NSHostingController(rootView: SessionView(store: store))
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

    /// How scrolling a chat went.
    struct Scrolled: CustomStringConvertible {
        var items = 0
        var steps = 0
        var median = Duration.zero
        var worst = Duration.zero
        var slow = 0
        var height: CGFloat = 0
        var opened = Duration.zero
        /// Bytes memory grew by while scrolling.
        var grew: UInt64 = 0
        /// Bytes memory grew by the second time through.
        var grewAgain: UInt64 = 0

        var description: String {
            "\(items) rows, \(Int(height)) pt, opened in \(opened); \(steps) steps, median \(median), worst \(worst), "
                + "\(slow) over 50 ms; memory grew \(grew / 1_048_576) MB, then \(grewAgain / 1_048_576) MB more"
        }
    }

    /// Opens `history` in a session's window, then reads it as a person
    /// does: up from the end a screen at a time, down again, then jumps
    /// about as the scroller is dragged.
    private func scrollThrough(_ history: [SessionEvent]) async throws -> Scrolled {
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
        // Under every other window, so the run doesn't cover the screen.
        window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) + 1)
        let hosting = NSHostingController(rootView: SessionView(store: store))
        // The window keeps its size: the chat takes what it's offered and
        // asks for none, so sized to its content the window keeps only the
        // composer.
        hosting.sizingOptions = []
        window.contentViewController = hosting
        window.setContentSize(CGSize(width: 1180, height: 760))
        window.orderFrontRegardless()
        defer {
            window.close()
            window.contentViewController = nil
        }

        var scrolled = Scrolled()
        let clock = ContinuousClock()
        let start = clock.now
        let task = store.send(.task)
        defer { task.cancel() }
        while !store.historyLoaded || store.items.isEmpty, clock.now - start < .seconds(120) {
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTAssertTrue(store.historyLoaded)
        await settle(window)
        scrolled.opened = clock.now - start
        scrolled.items = store.items.count

        let scroll = try XCTUnwrap(chatScroll(in: window.contentView), "no scroll view holds the chat")
        var steps: [Duration] = []
        func go(to target: (CGFloat) -> CGFloat) async -> Bool {
            let height = scroll.documentView?.frame.height ?? 0
            scrolled.height = max(scrolled.height, height)
            let maxY = max(0, height - scroll.contentView.bounds.height)
            let y = min(max(target(maxY), 0), maxY)
            let stepStart = clock.now
            scroll.contentView.scroll(to: NSPoint(x: 0, y: y))
            scroll.reflectScrolledClipView(scroll.contentView)
            window.contentView?.layoutSubtreeIfNeeded()
            window.displayIfNeeded()
            await Task.yield()
            steps.append(clock.now - stepStart)
            return y <= 0 || y >= maxY
        }

        let before = footprint()
        var position = CGFloat.greatestFiniteMagnitude
        var afterFirst: UInt64 = 0
        for pass in 0..<2 {
            if pass == 1 {
                afterFirst = footprint()
                position = .greatestFiniteMagnitude
            }
            // Up a screen at a time: with no end in sight, stop at 8,000 steps.
            for _ in 0..<8_000 {
                let top = await go { maxY in
                    position = min(position, maxY) - 600
                    return position
                }
                if top, position <= 0 { break }
            }
            position = 0
            for _ in 0..<8_000 {
                let bottom = await go { maxY in
                    position += 600
                    return min(position, maxY)
                }
                if bottom, position > 0 { break }
            }
        }
        scrolled.grewAgain = footprint() &- afterFirst
        if scrolled.grewAgain > 1 << 62 { scrolled.grewAgain = 0 }
        var random = SplitMix(seed: 3)
        for _ in 0..<40 {
            _ = await go { maxY in maxY * CGFloat(random.next() % 1_000) / 1_000 }
        }
        scrolled.grew = footprint() &- before
        if scrolled.grew > 1 << 62 { scrolled.grew = 0 }

        let sorted = steps.sorted()
        scrolled.steps = steps.count
        scrolled.median = sorted[sorted.count / 2]
        scrolled.worst = sorted.last ?? .zero
        scrolled.slow = steps.filter { $0 > .milliseconds(50) }.count
        return scrolled
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

    /// The memory the process uses, as Activity Monitor counts it.
    private func footprint() -> UInt64 {
        var info = task_vm_info_data_t()
        var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<natural_t>.size)
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? info.phys_footprint : 0
    }
}

/// Numbers that are the same on every run.
private struct SplitMix {
    var state: UInt64

    init(seed: UInt64) { state = seed }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var z = state
        z = (z ^ (z >> 30)) &* 0xBF58_476D_1CE4_E5B9
        z = (z ^ (z >> 27)) &* 0x94D0_49BB_1331_11EB
        return z ^ (z >> 31)
    }

    mutating func below(_ bound: Int) -> Int { Int(next() % UInt64(bound)) }

    mutating func chance(_ percent: Int) -> Bool { below(100) < percent }
}

/// A working session's history of every kind: short and pasted messages,
/// thoughts, many tools a turn (commands with small to huge output, reads,
/// searches, edits and whole-file writes, failures, permissions asked and
/// answered), replies from a line to pages of Markdown with code, tables
/// and lists, and the odd notice, failure, compaction and stop.
private struct ChatMaker {
    var random: SplitMix
    private var events: [SessionEvent] = []
    private var calls = 0

    init(seed: UInt64) { random = SplitMix(seed: seed) }

    mutating func history(turns: Int) -> [SessionEvent] {
        for turn in 0..<turns { addTurn(turn) }
        return events
    }

    private mutating func add(_ kind: SessionEvent.Kind) {
        events.append(SessionEvent(id: "e\(events.count)", time: Date(timeIntervalSince1970: 0), kind: kind))
    }

    private mutating func addTurn(_ turn: Int) {
        switch random.below(10) {
        case 0:
            add(
                .userMessage(
                    "Look at this log:\n" + lines("A pasted log line \(turn) that goes on.", 150 + random.below(600))))
        case 1: add(.userMessage("Here's the plan:\n\n" + Self.markdown(turn, sections: 2)))
        default: add(.userMessage("Now fix fixture \(turn), and say why it failed."))
        }
        add(.turnStarted)
        if random.chance(70) {
            add(.reasoning(lines("Check the fixture parses, then the next one.", 3 + random.below(80))))
        }
        let tools = [0, 1, 3, 8, 15, 25][random.below(6)]
        for _ in 0..<tools { addTool() }
        if random.chance(5) { add(.notice("Started over without its earlier conversation.")) }
        if random.chance(4) { add(.compacted) }
        if random.chance(6) {
            add(.failed("The agent stopped: the model's rate limit was reached. Wait a minute and send again."))
            add(.turnEnded(aborted: true))
            return
        }
        switch random.below(4) {
        case 0: add(.assistantMessage("Done: fixture \(turn) parses, **all rows** are fine."))
        case 1: add(.assistantMessage(Self.markdown(turn, sections: 1)))
        case 2: add(.assistantMessage(Self.markdown(turn, sections: 3)))
        default: add(.assistantMessage(Self.markdown(turn, sections: 12)))
        }
        add(.usage(model: "a", inputTokens: 1_000, outputTokens: 100, premiumRequests: 1))
        add(.turnEnded(aborted: random.chance(3)))
    }

    private mutating func addTool() {
        calls += 1
        let id = "c\(calls)"
        let (tool, arguments, output): (String, [String: String], String)
        switch random.below(7) {
        case 0:
            tool = "shell"
            arguments = ["command": "swift test --filter Parser"]
            output = lines(
                "row,ok,2026-11-13,a value long enough to wrap the line", [3, 40, 400, 6_000][random.below(4)])
        case 1:
            tool = "Read"
            arguments = ["file_path": "/work/Sources/Parser\(calls % 30).swift"]
            output = lines("    let field = fields[index].trimmingCharacters(in: .whitespaces)", 20 + random.below(900))
        case 2:
            tool = "Grep"
            arguments = ["pattern": "parse\\(", "path": "/work"]
            output = lines("/work/Sources/Parser.swift:42: func parse(_ line: Substring)", 1 + random.below(120))
        case 3, 4:
            tool = "Edit"
            arguments = [
                "file_path": "/work/Sources/Parser.swift",
                "old_string": lines("    let old = value", 1 + random.below(60)),
                "new_string": lines("    let new = value.trimmed", 1 + random.below(80)),
            ]
            output = "The file was updated."
        case 5:
            tool = "Write"
            arguments = [
                "file_path": "/work/Sources/Row\(calls).swift",
                "content": lines("let row = Row()", 5 + random.below(700)),
            ]
            output = "Wrote the file."
        default:
            tool = "WebFetch"
            arguments = ["url": "https://example.com/docs/\(calls)"]
            output = Self.markdown(calls, sections: 2)
        }
        let json = (try? JSONSerialization.data(withJSONObject: arguments)).flatMap {
            String(data: $0, encoding: .utf8)
        }
        add(.toolStarted(callID: id, tool: tool, argumentsJSON: json ?? "{}"))
        if random.chance(40) {
            let approved = random.chance(90)
            add(
                .permissionRequested(
                    requestID: "p\(id)", kind: tool == "shell" ? .shell : .write, summary: "\(tool) \(id)"))
            add(.permissionCompleted(requestID: "p\(id)", approved: approved, automatic: random.chance(50)))
            if !approved {
                add(.toolCompleted(callID: id, success: false, output: "The person denied this."))
                return
            }
        }
        add(.toolCompleted(callID: id, success: !random.chance(8), output: output))
    }

    private func lines(_ line: String, _ count: Int) -> String {
        (0..<count).map { "\(line) \($0)" }.joined(separator: "\n")
    }

    /// Markdown of `sections` sections, each with a heading, a list, code
    /// and a table.
    private static func markdown(_ n: Int, sections: Int) -> String {
        (0..<sections).map { section in
            """
            ## Step \(section + 1) of fixture \(n)

            The parser now reads each row once, and a malformed row says *which* column failed, with its line. \
            That makes the failures in `Tests.swift` point at the cause instead of the symptom.

            - **Fixtures** load once per run, so the suite is quicker.
            - The `strict` flag is gone; every read is strict.
            - Dates parse as ISO 8601 only.

            ```swift
            func parse(_ line: Substring) throws(ParseError) -> Row {
                let fields = line.split(separator: ",", omittingEmptySubsequences: false)
                guard fields.count == 4 else { throw .columns(found: fields.count) }
                return Row(name: String(fields[0]), ok: fields[1] == "ok", date: String(fields[2]))
            }
            ```

            | File | Lines | Change |
            | --- | --- | --- |
            | Parser.swift | 120 | rewritten |
            | Row.swift | 40 | new field |
            | Tests.swift | 300 | more cases |

            1. Run the tests again.
            2. Look at the failures, which now name the row.
            """
        }.joined(separator: "\n\n")
    }
}
