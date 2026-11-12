import ComposableArchitecture
import HumataDaemonClient
import HumataSearch
import XCTest

@MainActor
final class SearchFeatureTests: XCTestCase {
    private static let calendar: Calendar = {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(secondsFromGMT: 0) ?? .gmt
        return calendar
    }()

    /// A daemon whose agent runs the tests when asked to, and otherwise
    /// says what it was told.
    private func daemon() -> InMemoryDaemon {
        InMemoryDaemon { text in
            text.contains("test")
                ? [.run(command: "cargo test --workspace", output: "test result: ok. 12 passed"), .say("All pass.")]
                : [.say("You said: \(text)")]
        }
    }

    func testTypingSearchesOnceItPausesAndOpensTheHitsSessionAtItsEvent() async throws {
        let daemon = daemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "hello", .enqueue)
        try await daemon.client.send(session.id, "run the tests", .enqueue)
        let clock = TestClock()
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.calendar = Self.calendar
            $0.continuousClock = clock
            $0.date = .constant(daemon.start)
        }
        store.exhaustivity = .off

        await store.send(\.binding.text, "cargo te")
        await store.send(\.binding.text, "cargo tes")
        await clock.advance(by: .milliseconds(300))
        await store.receive(\.found)
        await store.receive(\.finished)
        let hit = try XCTUnwrap(store.state.hits.first)
        XCTAssertEqual(store.state.hits.count, 1)
        XCTAssertEqual(hit.kind, .toolCall)
        XCTAssertEqual(hit.turn, 2)
        XCTAssertEqual(hit.sessionID, session.id)
        XCTAssertEqual(hit.snippet.filter(\.found).map(\.text), ["cargo", "tes"])
        let event = try XCTUnwrap(daemon.history(session.id).first { $0.id == hit.eventID })
        guard case .toolStarted = event.kind else { return XCTFail("the hit isn't the command: \(event)") }

        await store.send(.hitTapped(hit))
        await store.receive(\.delegate.openSession)
    }

    func testRelatedWordsFindWhatWasSaidInOtherWordsAndAreMarked() async throws {
        let daemon = daemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "run the tests", .enqueue)
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.calendar = Self.calendar
            $0.continuousClock = TestClock()
            $0.relatedWords = RelatedWordsClient { _ in ["cargo"] }
            $0.date = .constant(daemon.start)
        }
        store.exhaustivity = .off

        await store.send(\.binding.text, "rust")
        await store.send(.submitted)
        await store.receive(\.found)
        XCTAssertEqual(store.state.hits, [])
        await store.receive(\.found)
        await store.receive(\.finished)
        XCTAssertEqual(store.state.related, ["cargo"])
        XCTAssertEqual(store.state.hits.map(\.kind), [.toolCall])
        XCTAssertEqual(store.state.hits.map(\.related), [true])
    }

    func testATimeScopeLeavesOutWhatHappenedBefore() async throws {
        let daemon = daemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "run the tests", .enqueue)
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.calendar = Self.calendar
            $0.continuousClock = TestClock()
            // A month on, so the session ran before the past week.
            $0.date = .constant(daemon.start.addingTimeInterval(30 * 24 * 3600))
        }
        store.exhaustivity = .off

        await store.send(\.binding.text, "cargo")
        await store.send(.submitted)
        await store.receive(\.found)
        XCTAssertEqual(store.state.hits.count, 1)
        await store.send(\.binding.period, .pastWeek)
        await store.receive(\.found)
        XCTAssertEqual(store.state.hits, [])
    }

    func testClearingTheTextClearsTheHits() async throws {
        let daemon = daemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "run the tests", .enqueue)
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.calendar = Self.calendar
            $0.continuousClock = TestClock()
            $0.date = .constant(daemon.start)
        }
        store.exhaustivity = .off

        await store.send(\.binding.text, "cargo")
        await store.send(.submitted)
        await store.receive(\.found)
        await store.send(\.binding.text, "  ")
        XCTAssertEqual(store.state.hits, [])
        XCTAssertFalse(store.state.isSearching)
    }
}
