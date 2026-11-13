import ComposableArchitecture
import HumataDaemonClient
import HumataReview
import XCTest

@MainActor
final class ReviewFeatureTests: XCTestCase {
    private let sum = FileRef(repository: "code", path: "sum.py")

    /// A session whose agent wrote sum.py, and answers a review by renaming
    /// its function.
    private func session() async throws -> (InMemoryDaemon, Session) {
        let daemon = InMemoryDaemon { text in
            if text.contains("[thread 1]") {
                return [
                    .write(path: "sum.py", contents: "def total(a, b):\n    return a + b\n"),
                    .say("[thread 1]\nRenamed it to total."),
                ]
            }
            return [.write(path: "sum.py", contents: "def add(a, b):\n    return a + b\n"), .say("Done.")]
        }
        let project = try await daemon.client.createProject("Humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "Write sum.py", .enqueue)
        return (daemon, session)
    }

    private func store(_ daemon: InMemoryDaemon, _ session: Session) -> TestStoreOf<ReviewFeature> {
        let store = TestStore(initialState: ReviewFeature.State(sessionID: session.id, turns: 1)) {
            ReviewFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    func testACommentSentAsAReviewComesBackAnsweredWithItsFix() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.loaded) {
            $0.selected = self.sum
        }

        await store.send(.lineTapped(1, extending: false)) {
            $0.commenting = ReviewFeature.Commenting(file: self.sum, startLine: 1, endLine: 1)
        }
        await store.send(.binding(.set(\.comment, "Call it total.")))
        await store.send(.commentSaved) {
            $0.commenting = nil
            $0.comment = ""
        }
        await store.receive(\.threadSaved)
        XCTAssertEqual(store.state.pending, 1)

        await store.send(.sendTapped)
        await store.receive(\.sent)
        await store.receive(\.loaded)
        let thread = try XCTUnwrap(store.state.threads.first)
        XCTAssertEqual(thread.comments.map(\.author), [.you, .agent])
        XCTAssertEqual(store.state.pending, 0)

        await store.send(.showFixTapped(thread.id)) {
            $0.range = .turn(2)
        }
        await store.receive(\.loaded)
        let lines = store.state.file?.hunks.flatMap(\.lines) ?? []
        XCTAssertTrue(lines.contains { $0.kind == .added && $0.text == "def total(a, b):" })
    }

    func testShiftClickingPicksTheLinesBetween() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.lineTapped(2, extending: false))
        await store.send(.lineTapped(1, extending: true)) {
            $0.commenting = ReviewFeature.Commenting(file: self.sum, startLine: 1, endLine: 2)
        }
    }

    func testPickedLinesQuotedGoToTheMessageBoxWithTheCommentNotTheReview() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.lineTapped(1, extending: false))
        await store.send(.lineTapped(2, extending: true))
        await store.send(.binding(.set(\.comment, "Why add?")))
        await store.send(.quoteTapped) {
            $0.commenting = nil
            $0.comment = ""
        }
        // The new file's two lines, marked as added, then the comment.
        let code = "+def add(a, b):\n+    return a + b"
        await store.receive(
            \.delegate.quote,
            CodeQuote.text(path: "sum.py", start: 1, end: 2, language: "diff", code: code, comment: "Why add?"))
        XCTAssertEqual(store.state.pending, 0)
    }

    func testLinesCantBeCommentedOnInATurnsChanges() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.rangePicked(.turn)) {
            $0.range = .turn(1)
        }
        await store.receive(\.loaded)
        await store.send(.lineTapped(1, extending: false))
        XCTAssertNil(store.state.commenting)
    }

    func testLeavingATurnForEverythingKeepsTheFileAndLetsLinesBeCommentedOn() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.rangePicked(.turn))
        await store.receive(\.loaded)
        XCTAssertFalse(store.state.canComment)

        await store.send(.rangePicked(.session))
        await store.receive(\.loaded)
        XCTAssertTrue(store.state.canComment)
        XCTAssertEqual(store.state.selected, sum)
        await store.send(.lineTapped(1, extending: false))
        XCTAssertNotNil(store.state.commenting)
    }

    func testATurnsChangesThatComeLateDontReplaceEverythingPickedAfter() async throws {
        let (daemon, session) = try await session()
        let (firstAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (firstAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        let calls = LockIsolated(0)
        var client = daemon.client
        let changes = client.changes
        client.changes = { session, query in
            let call = calls.withValue {
                $0 += 1
                return $0
            }
            if call == 1 {
                asked.yield()
                for await _ in firstAnswers { break }
            }
            return try await changes(session, query)
        }
        let store = TestStore(initialState: ReviewFeature.State(sessionID: session.id, turns: 1)) {
            ReviewFeature()
        } withDependencies: {
            $0.daemon = client
        }
        store.exhaustivity = .off

        await store.send(.rangePicked(.turn))
        for await _ in firstAsked { break }
        await store.send(.rangePicked(.session))
        await store.receive(\.loaded)
        answer.yield()
        // The turn's, which changes nothing.
        await store.receive(\.loaded)

        XCTAssertTrue(store.state.canComment)
        XCTAssertFalse(store.state.loading)
        XCTAssertEqual(store.state.selected, sum)
    }

    func testRevertingAsksFirstThenThrowsTheChangeAway() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.rangePicked(.unstaged)) {
            $0.range = .unstaged
        }
        await store.receive(\.loaded)
        let revert = ChangeEdit(file: sum, action: .revert)
        await store.send(.editTapped(revert))
        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(daemon.files(of: session.id).count, 1, "nothing reverted before the answer")

        await store.send(.alert(.presented(.revert(revert))))
        await store.receive(\.edited)
        await store.receive(\.loaded)
        XCTAssertEqual(daemon.files(of: session.id), [:])
        XCTAssertTrue(store.state.files.isEmpty)
    }

    func testStagingMovesTheFileToStaged() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.rangePicked(.unstaged))
        await store.receive(\.loaded)
        await store.send(.editTapped(ChangeEdit(file: sum, action: .stage)))
        await store.receive(\.edited)
        await store.receive(\.loaded)
        XCTAssertTrue(store.state.files.isEmpty)

        await store.send(.rangePicked(.staged))
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.files, [sum])
    }

    func testAStageWaitsForTheOneBeforeIt() async throws {
        let (daemon, session) = try await session()
        let edits = LockIsolated(0)
        let store = TestStore(initialState: ReviewFeature.State(sessionID: session.id, turns: 1)) {
            ReviewFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            // The first stage is still going when the second tap comes.
            $0.daemon.editChanges = { _, _ in
                edits.withValue { $0 += 1 }
                try await Task.never()
            }
        }
        store.exhaustivity = .off
        await store.send(.rangePicked(.unstaged))
        await store.receive(\.loaded)

        let first = await store.send(.editTapped(ChangeEdit(file: sum, action: .stage)))
        XCTAssertTrue(store.state.editing)
        await store.send(.editTapped(ChangeEdit(file: sum, action: .stage)))
        await Task.megaYield()
        XCTAssertEqual(edits.value, 1)
        await first.cancel()
    }

    func testChangesThatCouldntBeReadSaySoUntilTheyAre() async throws {
        let (daemon, session) = try await session()
        let reachable = LockIsolated(false)
        let store = TestStore(initialState: ReviewFeature.State(sessionID: session.id, turns: 1)) {
            ReviewFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.changes = { session, query in
                guard reachable.value else { throw DaemonError.disconnected }
                return try await daemon.client.changes(session, query)
            }
        }
        store.exhaustivity = .off

        await store.send(.refresh)
        await store.receive(\.loadFailed)
        XCTAssertNotNil(store.state.loadFailure)
        XCTAssertFalse(store.state.loading)

        reachable.setValue(true)
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.loadFailure)
        XCTAssertEqual(store.state.files, [sum])
    }

    func testMarkingViewedKeepsTheMarkWhenReadAgain() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.viewedToggled(sum))
        await store.send(.refresh)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.diff(sum)?.viewed, true)
    }

    func testAnEmptyReviewIsntSent() async throws {
        let (daemon, session) = try await session()
        let store = store(daemon, session)
        await store.send(.refresh)
        await store.receive(\.loaded)
        await store.send(.sendTapped)
        XCTAssertFalse(store.state.sending)
    }

    func testACommentThatCouldntBeSavedGoesBackOnItsLines() async {
        var state = ReviewFeature.State(sessionID: "s1")
        let picked = ReviewFeature.Commenting(file: sum, startLine: 1, endLine: 2)
        state.commenting = picked
        state.comment = "Call it total."
        let store = TestStore(initialState: state) {
            ReviewFeature()
        } withDependencies: {
            $0.daemon.createThread = { _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(.commentSaved)
        await store.receive(\.commentFailed)
        await store.receive(\.failed)
        XCTAssertEqual(store.state.commenting, picked)
        XCTAssertEqual(store.state.comment, "Call it total.")
        XCTAssertNotNil(store.state.alert)
    }

    func testAFailedCommentDoesntReplaceOneStartedSince() async {
        var state = ReviewFeature.State(sessionID: "s1")
        state.selected = sum
        state.commenting = ReviewFeature.Commenting(file: sum, startLine: 1, endLine: 1)
        state.comment = "Call it total."
        let failing = AsyncStream<Void>.makeStream()
        let store = TestStore(initialState: state) {
            ReviewFeature()
        } withDependencies: {
            $0.daemon.createThread = { _, _ in
                for await _ in failing.stream { break }
                throw DaemonError.disconnected
            }
        }
        store.exhaustivity = .off

        await store.send(.commentSaved)
        await store.send(.lineTapped(2, extending: false))
        failing.continuation.yield()
        await store.receive(\.commentFailed)
        await store.receive(\.failed)
        XCTAssertEqual(store.state.commenting, ReviewFeature.Commenting(file: sum, startLine: 2, endLine: 2))
        XCTAssertEqual(store.state.comment, "")
    }

    func testAReplyThatCouldntBeSentGoesBackInItsBox() async {
        let store = TestStore(initialState: ReviewFeature.State(sessionID: "s1")) {
            ReviewFeature()
        } withDependencies: {
            $0.daemon.replyToThread = { _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off

        await store.send(.replyChanged("t1", "Why not sum?"))
        await store.send(.replySent("t1"))
        XCTAssertNil(store.state.replies["t1"])
        await store.receive(\.replyFailed)
        await store.receive(\.failed)
        XCTAssertEqual(store.state.replies["t1"], "Why not sum?")
        XCTAssertNotNil(store.state.alert)
    }

    func testAKeptBranchRangeShowsItsBranchInTheField() {
        var left = ReviewFeature.State(sessionID: "s1")
        left.range = .branch("develop")
        var opened = ReviewFeature.State(sessionID: "s1")
        opened.kept = left.kept
        XCTAssertEqual(opened.range, .branch("develop"))
        XCTAssertEqual(opened.branch, "develop")
    }
}
