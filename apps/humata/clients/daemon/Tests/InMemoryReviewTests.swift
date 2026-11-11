import HumataDaemonClient
import XCTest

final class InMemoryReviewTests: XCTestCase {
    /// A daemon whose agent writes sum.py, then answers a review by renaming
    /// its function.
    private func reviewed() async throws -> (InMemoryDaemon, Session) {
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

    private let sum = FileRef(repository: "code", path: "sum.py")

    func testAReplyAndFixComeBackOnTheThread() async throws {
        let (daemon, session) = try await reviewed()
        let thread = try await daemon.client.createThread(
            session.id, NewThread(file: sum, startLine: 1, endLine: 1, text: "Call it total."))
        XCTAssertEqual(thread.code, ["def add(a, b):"])
        XCTAssertEqual(thread.pending, 1)

        let sent = try await daemon.client.sendReview(session.id)
        XCTAssertEqual(sent, 1)

        let threads = try await daemon.client.threads(session.id)
        let answered = try XCTUnwrap(threads.first)
        XCTAssertEqual(answered.comments.map(\.author), [.you, .agent])
        XCTAssertEqual(answered.comments.last?.text, "Renamed it to total.")
        XCTAssertTrue(answered.outdated, "the line it was on changed")
        let turn = try XCTUnwrap(answered.comments.last?.turn)
        let fix = try await daemon.client.changes(session.id, ChangesQuery(range: .turn(turn)))
        let lines = fix.flatMap(\.files).flatMap(\.hunks).flatMap(\.lines)
        XCTAssertTrue(lines.contains(DiffLine(kind: .removed, old: 1, new: nil, text: "def add(a, b):")))
        XCTAssertTrue(lines.contains(DiffLine(kind: .added, old: nil, new: 1, text: "def total(a, b):")))
    }

    func testNothingToSendIsRefused() async throws {
        let (daemon, session) = try await reviewed()
        do {
            _ = try await daemon.client.sendReview(session.id)
            XCTFail("Sent an empty review")
        } catch DaemonError.refused(let code, _) {
            XCTAssertEqual(code, .invalidRequest)
        }
    }

    func testAThreadOutsideTheFileIsRefused() async throws {
        let (daemon, session) = try await reviewed()
        do {
            _ = try await daemon.client.createThread(
                session.id, NewThread(file: sum, startLine: 3, endLine: 9, text: "Here?"))
            XCTFail("Commented past the end")
        } catch DaemonError.refused(let code, _) {
            XCTAssertEqual(code, .invalidRequest)
        }
    }

    func testAThreadFollowsItsLinesWhenOthersAreAddedAbove() async throws {
        let daemon = InMemoryDaemon { text in
            text == "more"
                ? [.write(path: "a.txt", contents: "new\none\ntwo\n")]
                : [.write(path: "a.txt", contents: "one\ntwo\n")]
        }
        let project = try await daemon.client.createProject("Humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "write", .enqueue)
        let file = FileRef(repository: "code", path: "a.txt")
        _ = try await daemon.client.createThread(
            session.id, NewThread(file: file, startLine: 2, endLine: 2, text: "Why two?"))
        try await daemon.client.send(session.id, "more", .enqueue)

        let threads = try await daemon.client.threads(session.id)
        let thread = try XCTUnwrap(threads.first)
        XCTAssertEqual(thread.startLine, 3)
        XCTAssertFalse(thread.outdated)
    }

    func testStagingMovesAFileFromUnstagedToStagedAndRevertPutsItBack() async throws {
        let (daemon, session) = try await reviewed()
        func paths(_ range: ChangeRange) async throws -> [String] {
            try await daemon.client.changes(session.id, ChangesQuery(range: range)).flatMap(\.files).map(\.path)
        }
        try await daemon.client.editChanges(session.id, ChangeEdit(file: sum, action: .stage))
        let unstaged = try await paths(.unstaged)
        let staged = try await paths(.staged)
        XCTAssertEqual(unstaged, [])
        XCTAssertEqual(staged, ["sum.py"])

        try await daemon.client.editChanges(session.id, ChangeEdit(file: sum, action: .unstage))
        try await daemon.client.editChanges(session.id, ChangeEdit(file: sum, action: .revert))
        XCTAssertEqual(daemon.files(of: session.id), [:])
        do {
            try await daemon.client.editChanges(session.id, ChangeEdit(file: sum, action: .revert))
            XCTFail("Reverted a change that's gone")
        } catch DaemonError.refused(let code, _) {
            XCTAssertEqual(code, .changed)
        }
    }

    func testAViewedFileStaysViewedUntilItChanges() async throws {
        let (daemon, session) = try await reviewed()
        let changes = try await daemon.client.changes(session.id, ChangesQuery())
        let file = try XCTUnwrap(changes.first?.files.first)
        try await daemon.client.markViewed(session.id, sum, file.blob, true)
        let viewed = try await daemon.client.changes(session.id, ChangesQuery()).first?.files.first?.viewed
        XCTAssertEqual(viewed, true)

        _ = try await daemon.client.createThread(
            session.id, NewThread(file: sum, startLine: 1, endLine: 1, text: "Call it total."))
        _ = try await daemon.client.sendReview(session.id)
        let after = try await daemon.client.changes(session.id, ChangesQuery()).first?.files.first?.viewed
        XCTAssertEqual(after, false)
    }

    func testAContextOfOneShowsOneLineAroundAChange() async throws {
        let old = (1...10).map { "line \($0)\n" }.joined()
        let new = old.replacingOccurrences(of: "line 5\n", with: "five\n")
        let daemon = InMemoryDaemon { text in
            [.write(path: "f.txt", contents: text == "first" ? old : new)]
        }
        let project = try await daemon.client.createProject("Humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "first", .enqueue)
        try await daemon.client.send(session.id, "second", .enqueue)
        let changes = try await daemon.client.changes(session.id, ChangesQuery(range: .turn(2), context: 1))
        let hunks = changes.flatMap(\.files).flatMap(\.hunks)
        XCTAssertEqual(hunks.map(\.lines.count), [4])
        XCTAssertEqual(hunks.first.map { [$0.oldStart, $0.oldCount, $0.newStart, $0.newCount] }, [4, 3, 4, 3])
    }
}
