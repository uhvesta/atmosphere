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
}
