import ComposableArchitecture
import HumataAppFeature
import HumataDaemonClient
import HumataSession
import SwiftUI
import XCTest

/// What a chat costs as a person uses it: opening it, reading it up and
/// down, reading back while a reply streams, and following a turn of many
/// tool calls. Each prints its measures, one to a line starting `perf:`.
/// What doesn't depend on the machine is always checked: memory, where the
/// chat ends up, and how a cost grows with the chat. Times are checked
/// against budgets only with `HUMATA_PERF` set, and mean most in an
/// optimized build (`FLOWKIT_OPT=1` with `run_mac.sh`).
@MainActor
final class ChatPerfTests: XCTestCase {
    /// Opening a chat keeps the window live: the longest the main thread
    /// goes without drawing or taking a key doesn't grow with the chat.
    func testOpeningAChatKeepsTheWindowLiveHoweverLongItIs() async throws {
        let short = try await open(turns: 200)
        let long = try await open(turns: 1_000)
        Budget.note("chat open, 200 turns: \(short)")
        Budget.note("chat open, 1,000 turns: \(long)")

        // Under 50 ms either is too short to tell from what else the
        // machine was doing.
        XCTAssertLessThanOrEqual(
            long.main.longest, max(short.main.longest * 2, .milliseconds(50)),
            "opening a chat five times longer held the main thread more than twice as long")

        guard Budget.timed else { return }
        XCTAssertLessThan(short.shown, .milliseconds(300), "200 turns took too long to show")
        XCTAssertLessThan(long.shown, .seconds(1), "1,000 turns took too long to show")
        XCTAssertLessThan(short.main.longest, .milliseconds(100), "opening 200 turns stalled the window")
        XCTAssertLessThan(long.main.longest, .milliseconds(100), "opening 1,000 turns stalled the window")
    }

    /// Scrolling a chat costs what's on screen, not how long the chat is: a
    /// chat five times longer, of every kind of row, grows memory no more
    /// as it's read up and down, and memory stops growing once it has been
    /// read.
    func testScrollingCostsWhatShowsNotHowLongTheChatIs() async throws {
        let short = try await scrollThrough(Seeds.chat(turns: 40, seed: 7))
        let long = try await scrollThrough(Seeds.chat(turns: 200, seed: 7))
        Budget.note("chat scroll, 40 turns: \(short)")
        Budget.note("chat scroll, 200 turns: \(long)")
        let report = "short: \(short)\nlong: \(long)"

        let megabyte: UInt64 = 1_048_576
        XCTAssertLessThan(long.grew, max(short.grew * 2, 150 * megabyte), report)
        XCTAssertLessThan(long.grewAgain, 100 * megabyte, "memory kept growing on a second read\n" + report)

        guard Budget.timed else { return }
        XCTAssertLessThan(long.steps.median, Duration.milliseconds(16), report)
        XCTAssertLessThan(long.steps.worst, Duration.milliseconds(250), report)
        XCTAssertLessThan(
            long.steps.over(.milliseconds(50)) * 100, long.steps.count * 5,
            "more than 5% of steps were slow\n" + report)
        XCTAssertLessThan(long.steps.median, short.steps.median * 2 + Duration.milliseconds(4), report)
    }

    /// Reading back up a long chat while a reply streams in: the wheel
    /// moves a few points a frame, and each frame takes what it takes to
    /// draw. The chat stays where the person reads, goes back to its end
    /// when they scroll there, and goes to it when they send.
    func testScrollingWhileAReplyStreams() async throws {
        var history = Seeds.chat(turns: 400, seed: 5)
        history.append(Self.event("asked", .userMessage("Write it all out.")))
        history.append(Self.event("started", .turnStarted))
        let words = 8_000
        let store = Self.store(history) { continuation in
            for word in 0..<words {
                try await Task.sleep(for: .milliseconds(5))
                let text = word % 40 == 39 ? "word\(word)\n\n" : "word\(word) "
                continuation.yield(.event(Self.event("d\(word)", .assistantDelta(text))))
            }
        }
        // In a split view's column, as in the app, with the window views'
        // own size checks on: each column's view asks its content for its
        // smallest size on every change.
        let window = PerfWindow(
            sizedByContent: true,
            AtLeast(width: 820, height: 520) {
                NavigationSplitView {
                    List { Text("Sessions") }
                } detail: {
                    AtLeast(width: 360, height: 300) { SessionView(store: store) }
                }
            })
        let task = store.send(.task)
        defer {
            task.cancel()
            window.close()
        }
        _ = await window.until { store.historyLoaded && !store.items.isEmpty }
        await window.settle()
        let scroll = try XCTUnwrap(window.scroll(), "no scroll view holds the chat")
        // Let the reply get going, then read up and back down.
        try await Task.sleep(for: .seconds(2))
        XCTAssertFalse(store.streaming.isEmpty, "the reply didn't stream")

        let atEnd = await window.soonShowsEnd(of: scroll)
        XCTAssertTrue(atEnd, "the chat wasn't at its end as the reply came")
        let up = await window.wheel(scroll, lines: 2, times: 300)
        let stayed = !window.showsEnd(of: scroll)
        try await Task.sleep(for: .seconds(1))
        XCTAssertTrue(
            stayed && !window.showsEnd(of: scroll), "the chat went back to its end while the person read up")
        let down = await window.wheel(scroll, lines: -4, times: 600) { !window.showsEnd(of: scroll) }
        try await Task.sleep(for: .milliseconds(500))
        let backAtEnd = await window.soonShowsEnd(of: scroll)
        XCTAssertTrue(backAtEnd, "the chat didn't scroll back to its end")

        // Read up again, then send a message: the chat goes to its end to
        // show it, and keeps to it as the reply goes on.
        let upAgain = await window.wheel(scroll, lines: 2, times: 300)
        XCTAssertFalse(window.showsEnd(of: scroll), "the chat didn't scroll up")
        store.send(.binding(.set(\.draft, "And the rest, please.")))
        store.send(.sendTapped(.enqueue))
        let sentAtEnd = await window.soonShowsEnd(of: scroll)
        XCTAssertTrue(sentAtEnd, "sending didn't take the chat to its end")
        try await Task.sleep(for: .seconds(1))
        let keptAtEnd = await window.soonShowsEnd(of: scroll)
        XCTAssertTrue(keptAtEnd, "the chat didn't keep to its end after the message was sent")

        let frames = up + down + upAgain
        Budget.note(
            "scroll while streaming: \(frames), \(frames.over(.milliseconds(33))) over 33 ms; "
                + "reply \(store.streaming.utf8.count) bytes")

        guard Budget.timed else { return }
        XCTAssertLessThan(frames.p95, Duration.milliseconds(33), "frames were slow as the reply streamed")
    }

    /// A turn of many tool calls, as an agent at work makes them: 300 start
    /// and finish 10 ms apart, their output from three lines to 400 KB, and
    /// the chat keeps to its end as their rows come.
    func testAToolHeavyTurnKeepsTheChatAtItsEnd() async throws {
        var history = Seeds.chat(turns: 200, seed: 9)
        history.append(Self.event("asked", .userMessage("Run every fixture.")))
        history.append(Self.event("started", .turnStarted))
        let calls = 300
        let line = "row,ok,2026-11-13,a value long enough to wrap the line and then some"
        let outputs = [3, 40, 400, 6_000].map { count in
            (0..<count).map { "\(line) \($0)" }.joined(separator: "\n")
        }
        let begun = LockIsolated(false)
        let finished = LockIsolated(false)
        let store = Self.store(history) { continuation in
            while !begun.value { try await Task.sleep(for: .milliseconds(10)) }
            for call in 0..<calls {
                continuation.yield(
                    .event(
                        Self.event(
                            "t\(call)", .toolStarted(callID: "live\(call)", tool: "shell", argumentsJSON: "{}"))))
                try await Task.sleep(for: .milliseconds(5))
                continuation.yield(
                    .event(
                        Self.event(
                            "o\(call)",
                            .toolCompleted(
                                callID: "live\(call)", success: true, output: outputs[call % outputs.count]))))
                try await Task.sleep(for: .milliseconds(5))
            }
            finished.setValue(true)
        }
        let window = PerfWindow(SessionView(store: store))
        let task = store.send(.task)
        defer {
            task.cancel()
            window.close()
        }
        _ = await window.until { store.historyLoaded && !store.items.isEmpty }
        await window.settle()
        let scroll = try XCTUnwrap(window.scroll(), "no scroll view holds the chat")
        let atEnd = await window.soonShowsEnd(of: scroll)
        XCTAssertTrue(atEnd, "the chat didn't open at its end")
        let rows = store.items.count

        let watch = MainThread.Watch()
        begun.setValue(true)
        // Looked at four times a second while the calls come.
        var looks = 0
        var away = 0
        while !finished.value, looks < 240 {
            try await Task.sleep(for: .milliseconds(250))
            looks += 1
            if !(await window.soonShowsEnd(of: scroll)) { away += 1 }
        }
        let main = watch.stop()
        await window.settle()
        let endedAtEnd = await window.soonShowsEnd(of: scroll)
        Budget.note("tool-heavy turn, \(calls) calls: \(main); \(store.items.count - rows) rows added")

        XCTAssertTrue(finished.value, "the calls didn't all come within a minute")
        XCTAssertGreaterThan(store.items.count, rows, "the calls' rows didn't reach the chat")
        XCTAssertEqual(away, 0, "the chat left its end \(away) of \(looks) times it was looked at")
        XCTAssertTrue(endedAtEnd, "the chat wasn't at its end once the calls had come")

        guard Budget.timed else { return }
        XCTAssertLessThan(main.longest, .milliseconds(50), "a tool call's row stalled the window")
        XCTAssertLessThan(main.busy, 0.3, "the main thread was busy most of the turn")
    }

    // MARK: - Helpers

    private nonisolated static func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    /// A session's store whose daemon answers a watch with `history`, then
    /// with what `live` sends for as long as the chat is watched.
    private static func store(
        _ history: [SessionEvent],
        live: @escaping @Sendable (AsyncThrowingStream<WatchItem, any Error>.Continuation) async throws -> Void =
            { _ in }
    ) -> StoreOf<SessionFeature> {
        let session = Session(id: "s1", name: "long", backend: .scripted, workingDirectory: "/work")
        return Store(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon = .previewValue
            $0.continuousClock = ContinuousClock()
            $0.daemon.watch = { _, _ in
                AsyncThrowingStream { continuation in
                    for event in history { continuation.yield(.event(event)) }
                    continuation.yield(.historyLoaded)
                    let writer = Task { try await live(continuation) }
                    continuation.onTermination = { _ in writer.cancel() }
                }
            }
            $0.daemon.markSeen = { _, _ in }
        }
    }

    /// How opening a chat went.
    private struct Opened: CustomStringConvertible {
        var events = 0
        var rows = 0
        /// From opening the window to the chat's rows on screen.
        var shown = Duration.zero
        var main = MainThread(longest: .zero, busy: 0)
        /// Bytes memory grew by.
        var grew: UInt64 = 0

        var description: String {
            "\(events) events, \(rows) rows, shown in \(shown.inMilliseconds) ms, \(main), "
                + "memory grew \(grew / 1_048_576) MB"
        }
    }

    /// Opens a chat of `turns` turns in a session's window and waits for
    /// its rows to show.
    private func open(turns: Int) async throws -> Opened {
        let history = Seeds.chat(turns: turns, seed: 21)
        let store = Self.store(history)
        var opened = Opened(events: history.count)
        let before = Memory.footprint()
        let clock = ContinuousClock()
        let start = clock.now
        let watch = MainThread.Watch()
        let window = PerfWindow(SessionView(store: store))
        let task = store.send(.task)
        defer {
            task.cancel()
            window.close()
        }
        // Its rows show once it holds them and its list is taller than the
        // window: a chat this long fills it many times over.
        let shown = await window.until(.seconds(120)) {
            guard store.historyLoaded, !store.items.isEmpty, let scroll = window.scroll() else { return false }
            return (scroll.documentView?.frame.height ?? 0) > scroll.contentView.bounds.height
        }
        await window.settle()
        opened.main = watch.stop()
        XCTAssertNotNil(shown, "a chat of \(turns) turns didn't show within two minutes")
        opened.shown = shown ?? clock.now - start
        opened.rows = store.items.count
        opened.grew = Memory.grown(since: before)
        return opened
    }

    /// How scrolling a chat went.
    private struct Scrolled: CustomStringConvertible {
        var rows = 0
        var height: CGFloat = 0
        var opened = Duration.zero
        var steps = Frames([])
        /// Bytes memory grew by while scrolling.
        var grew: UInt64 = 0
        /// Bytes memory grew by the second time through.
        var grewAgain: UInt64 = 0

        var description: String {
            "\(rows) rows, \(Int(height)) pt, opened in \(opened.inMilliseconds) ms; \(steps), "
                + "\(steps.over(.milliseconds(50))) over 50 ms; memory grew \(grew / 1_048_576) MB, "
                + "then \(grewAgain / 1_048_576) MB more"
        }
    }

    /// Opens `history` in a session's window, then reads it as a person
    /// does: up from the end a screen at a time and down again, twice, then
    /// jumps about as the scroller is dragged.
    private func scrollThrough(_ history: [SessionEvent]) async throws -> Scrolled {
        let store = Self.store(history)
        let window = PerfWindow(SessionView(store: store))
        let task = store.send(.task)
        defer {
            task.cancel()
            window.close()
        }
        var scrolled = Scrolled()
        let loaded = await window.until(.seconds(120)) { store.historyLoaded && !store.items.isEmpty }
        XCTAssertNotNil(loaded, "the chat didn't open")
        await window.settle()
        scrolled.opened = loaded ?? .zero
        scrolled.rows = store.items.count

        let scroll = try XCTUnwrap(window.scroll(), "no scroll view holds the chat")
        let before = Memory.footprint()
        let first = await window.page(scroll, by: 600)
        let afterFirst = Memory.footprint()
        let second = await window.page(scroll, by: 600)
        scrolled.grewAgain = Memory.grown(since: afterFirst)
        var random = SplitMix(seed: 3)
        var jumps: [Duration] = []
        for _ in 0..<40 {
            let place = CGFloat(random.below(1_000)) / 1_000
            let jump = await window.move(scroll) { furthest in furthest * place }
            jumps.append(jump.took)
        }
        scrolled.steps = first + second + Frames(jumps)
        scrolled.grew = Memory.grown(since: before)
        scrolled.height = scroll.documentView?.frame.height ?? 0
        return scrolled
    }
}
