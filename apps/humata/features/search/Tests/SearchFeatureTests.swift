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

    func testRelatedWordsHitsGoBelowThePlainOnesWithoutRepeatingThem() async throws {
        let daemon = InMemoryDaemon { _ in [.run(command: "cargo build", output: "built"), .say("Compiled it.")] }
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "compile it", .enqueue)
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.calendar = Self.calendar
            $0.continuousClock = TestClock()
            $0.relatedWords = RelatedWordsClient { _ in ["cargo", "compiled"] }
            $0.date = .constant(daemon.start)
        }
        store.exhaustivity = .off

        await store.send(\.binding.text, "compile")
        await store.send(.submitted)
        await store.receive(\.found)
        let plain = store.state.hits
        XCTAssertFalse(plain.isEmpty)
        await store.receive(\.found)
        await store.receive(\.finished)
        XCTAssertEqual(Array(store.state.hits.prefix(plain.count)), plain)
        XCTAssertGreaterThan(store.state.hits.count, plain.count)
        XCTAssertEqual(Set(store.state.hits.map(\.id)).count, store.state.hits.count)
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

    func testASearchMadeWhileTheIndexFillsIsMadeAgainUntilItHasFilled() async throws {
        let daemon = daemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "hello", .enqueue)
        let calls = LockIsolated(0)
        var client = daemon.client
        let search = client.search
        // The index has nothing the first time, and has caught up the third.
        client.search = { query in
            let call = calls.withValue {
                $0 += 1
                return $0
            }
            var results = try await search(query)
            if call == 1 { results.hits = [] }
            results.catchingUp = call < 3
            return results
        }
        let clock = TestClock()
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: {
            $0.daemon = client
            $0.relatedWords.related = { _ in [] }
            $0.calendar = Self.calendar
            $0.continuousClock = clock
            $0.date = .constant(daemon.start)
        }
        store.exhaustivity = .off

        await store.send(\.binding.text, "hello")
        await store.send(.submitted)
        await store.receive(\.found)
        XCTAssertEqual(store.state.hits, [])
        XCTAssertTrue(store.state.indexing)

        await clock.advance(by: .seconds(2))
        await store.receive(\.foundMore)
        XCTAssertFalse(store.state.hits.isEmpty)
        XCTAssertTrue(store.state.indexing)

        await clock.advance(by: .seconds(2))
        await store.receive(\.foundMore)
        await store.receive(\.finished)
        XCTAssertFalse(store.state.indexing)
        XCTAssertFalse(store.state.isSearching)
        XCTAssertEqual(calls.value, 3)
    }

    func testASearchTheIndexNeverCaughtUpWithSaysSoUntilSearchedAgain() async throws {
        let daemon = daemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "hello", .enqueue)
        // The index is still filling however often it's asked, until told.
        let filled = LockIsolated(false)
        var client = daemon.client
        let search = client.search
        client.search = { query in
            var results = try await search(query)
            results.catchingUp = !filled.value
            if !filled.value { results.hits = [] }
            return results
        }
        let clock = TestClock()
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: {
            $0.daemon = client
            $0.relatedWords.related = { _ in [] }
            $0.calendar = Self.calendar
            $0.continuousClock = clock
            $0.date = .constant(daemon.start)
        }
        store.exhaustivity = .off

        await store.send(\.binding.text, "hello")
        await store.send(.submitted)
        await store.receive(\.found)
        XCTAssertFalse(store.state.incomplete, "it's still searching again by itself")

        await clock.advance(by: .seconds(60))
        await store.receive(\.finished)
        XCTAssertFalse(store.state.isSearching)
        XCTAssertTrue(store.state.incomplete)
        XCTAssertEqual(store.state.hits, [])

        filled.setValue(true)
        await store.send(.submitted)
        XCTAssertFalse(store.state.incomplete)
        await store.receive(\.found)
        await store.receive(\.finished)
        XCTAssertFalse(store.state.incomplete)
        XCTAssertFalse(store.state.hits.isEmpty)
    }

    func testASlowEarlierSearchDoesNotReplaceTheResultsOfALaterOne() async throws {
        let daemon = daemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, approvalMode: .allowEverything))
        try await daemon.client.send(session.id, "hello", .enqueue)
        let (firstAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (firstAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        let calls = LockIsolated(0)
        var client = daemon.client
        let search = client.search
        client.search = { query in
            let call = calls.withValue {
                $0 += 1
                return $0
            }
            if call == 1 {
                asked.yield()
                for await _ in firstAnswers { break }
            }
            return try await search(query)
        }
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: {
            $0.daemon = client
            $0.calendar = Self.calendar
            $0.continuousClock = TestClock()
            $0.date = .constant(daemon.start)
        }
        store.exhaustivity = .off

        await store.send(\.binding.text, "cargo")
        await store.send(.submitted)
        for await _ in firstAsked { break }
        await store.send(\.binding.text, "hello")
        await store.send(.submitted)
        await store.receive(\.finished)
        // The slow one was stopped, so its answer changes nothing.
        answer.yield()

        XCTAssertEqual(store.state.words, "hello")
        XCTAssertFalse(store.state.isSearching)
    }
}
