import HumataDaemonClient
import XCTest

final class InMemoryDaemonTests: XCTestCase {
    private func said(_ events: [SessionEvent]) -> [String] {
        events.compactMap { event in
            switch event.kind {
            case .userMessage(let text): "user: \(text)"
            case .assistantMessage(let text): "agent: \(text)"
            default: nil
            }
        }
    }

    func testAMessageIsAnswered() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "hello", .enqueue)
        let history = try await daemon.client.history(session.id)
        XCTAssertEqual(said(history), ["user: hello", "agent: You said: hello"])
    }

    func testAForkKeepsTheEventsBeforeItsEvent() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "one", .enqueue)
        try await daemon.client.send(session.id, "two", .enqueue)
        let history = try await daemon.client.history(session.id)
        let two = try XCTUnwrap(history.first { $0.kind == .userMessage("two") })

        let fork = try await daemon.client.fork(session.id, two.id, "fork", false)

        XCTAssertEqual(fork.parentID, session.id)
        XCTAssertEqual(fork.forkedBeforeEventID, two.id)
        let forked = try await daemon.client.history(fork.id)
        XCTAssertEqual(said(forked), ["user: one", "agent: You said: one"])
        XCTAssertEqual(said(daemon.history(session.id)).count, 4)
    }

    func testWatchingReplaysTheHistoryThenFollows() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "one", .enqueue)
        var events = await daemon.client.watch(session.id).makeAsyncIterator()
        try await daemon.client.send(session.id, "two", .enqueue)

        var seen: [SessionEvent] = []
        for _ in 0..<10 {
            if let event = try await events.next() { seen.append(event) }
        }
        XCTAssertEqual(said(seen), ["user: one", "agent: You said: one", "user: two", "agent: You said: two"])
    }

    private let edit: @Sendable (String) -> [InMemoryDaemon.Step] = { _ in
        [.think("It needs a file."), .write(path: "/work/a.txt", contents: "a"), .say("Done.")]
    }

    private func waiting(_ events: [SessionEvent]) -> String? {
        var open: [String] = []
        for event in events {
            switch event.kind {
            case .permissionRequested(let request, _, _): open.append(request)
            case .permissionCompleted(let request, _, _): open.removeAll { $0 == request }
            default: break
            }
        }
        return open.last
    }

    func testAnEditWaitsForApprovalThenFinishes() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "write it", .enqueue)
        XCTAssertEqual(daemon.sessions.first?.state, .waitingForApproval)
        XCTAssertEqual(daemon.files(of: session.id), [:])

        let request = try XCTUnwrap(waiting(daemon.history(session.id)))
        try await daemon.client.approve(session.id, request, true)

        XCTAssertEqual(daemon.files(of: session.id), ["/work/a.txt": "a"])
        XCTAssertEqual(daemon.sessions.first?.state, .idle)
        XCTAssertEqual(said(daemon.history(session.id)), ["user: write it", "agent: Done."])
    }

    func testADeniedEditWritesNothing() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "write it", .enqueue)
        let request = try XCTUnwrap(waiting(daemon.history(session.id)))
        try await daemon.client.approve(session.id, request, false)
        XCTAssertEqual(daemon.files(of: session.id), [:])
        XCTAssertEqual(daemon.sessions.first?.state, .idle)
    }

    func testAllowingEditsWritesWithoutAsking() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, workingDirectory: "/work", approvalMode: .allowEdits))
        try await daemon.client.send(session.id, "write it", .enqueue)
        XCTAssertEqual(daemon.files(of: session.id), ["/work/a.txt": "a"])
        XCTAssertNil(waiting(daemon.history(session.id)))
    }

    func testChangingTheModeAnswersTheWaitingRequest() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "write it", .enqueue)
        try await daemon.client.setApprovalMode(session.id, .allowEdits)
        XCTAssertEqual(daemon.files(of: session.id), ["/work/a.txt": "a"])
        XCTAssertEqual(daemon.sessions.first?.approvalMode, .allowEdits)
    }

    func testAQueuedMessageRunsAfterTheWaitingTurnIsStopped() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "one", .enqueue)
        try await daemon.client.send(session.id, "two", .enqueue)
        XCTAssertEqual(said(daemon.history(session.id)), ["user: one"])

        try await daemon.client.abort(session.id)

        let history = daemon.history(session.id)
        XCTAssertTrue(history.contains { $0.kind == .turnEnded(aborted: true) })
        XCTAssertEqual(said(history), ["user: one", "user: two"])
        XCTAssertNotNil(waiting(history))
    }

    func testASessionInAProjectWorksInItsOwnCopy() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("Humata", [Repository(path: "/code/humata")], "")
        XCTAssertEqual(project.repositories, [Repository(name: "humata", path: "/code/humata")])
        let session = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        let other = try await daemon.client.start(StartRequest(backend: .scripted, projectID: project.id))
        XCTAssertNotEqual(session.workingDirectory, "/code/humata")
        XCTAssertNotEqual(session.workingDirectory, other.workingDirectory)
        let listed = try await daemon.client.sessions(project.id)
        XCTAssertEqual(listed.map(\.id), [session.id, other.id])
        let elsewhere = try await daemon.client.sessions("elsewhere")
        XCTAssertEqual(elsewhere, [])
    }

    func testAForkWithCodeHasTheFilesAsTheyWereAndAChatOnlyForkSharesThem() async throws {
        let daemon = InMemoryDaemon { text in [.write(path: "/\(text).md", contents: text), .say("Wrote \(text).")] }
        let project = try await daemon.client.createProject("Humata", [Repository(path: "/code/humata")], "")
        let session = try await daemon.client.start(
            StartRequest(backend: .scripted, approvalMode: .allowEverything, projectID: project.id))
        try await daemon.client.send(session.id, "one", .enqueue)
        try await daemon.client.send(session.id, "two", .enqueue)
        let two = try XCTUnwrap(daemon.history(session.id).first { $0.kind == .userMessage("two") })

        let fork = try await daemon.client.fork(session.id, two.id, nil, false)
        XCTAssertNotEqual(fork.workingDirectory, session.workingDirectory)
        XCTAssertEqual(daemon.files(of: fork.id), ["/one.md": "one"])
        try await daemon.client.send(fork.id, "three", .enqueue)
        XCTAssertEqual(daemon.files(of: fork.id), ["/one.md": "one", "/three.md": "three"])
        XCTAssertEqual(daemon.files(of: session.id), ["/one.md": "one", "/two.md": "two"])

        let chat = try await daemon.client.fork(session.id, two.id, nil, true)
        XCTAssertEqual(chat.workingDirectory, session.workingDirectory)
        XCTAssertEqual(daemon.files(of: chat.id), daemon.files(of: session.id))
        let changes = try await daemon.client.changes(fork.id)
        XCTAssertTrue(changes.contains { $0.patch.contains("+three") })
        XCTAssertFalse(changes.contains { $0.patch.contains("+two") })
    }

    func testAnArchivedSessionTakesNoMessagesUntilUnarchived() async throws {
        let daemon = InMemoryDaemon()
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        let archived = try await daemon.client.update(session.id, SessionUpdate(archived: true))
        XCTAssertTrue(archived.archived)
        do {
            try await daemon.client.send(session.id, "hello", .enqueue)
            XCTFail("Sent to an archived session")
        } catch {
            XCTAssertEqual((error as? DaemonError).flatMap(code), .archived)
        }
        _ = try await daemon.client.update(session.id, SessionUpdate(archived: false))
        try await daemon.client.send(session.id, "hello", .enqueue)
        XCTAssertEqual(said(daemon.history(session.id)), ["user: hello", "agent: You said: hello"])
    }

    func testASessionIsNotArchivedWhileItWaits() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let session = try await daemon.client.start(StartRequest(backend: .scripted, workingDirectory: "/work"))
        try await daemon.client.send(session.id, "write it", .enqueue)
        do {
            _ = try await daemon.client.update(session.id, SessionUpdate(archived: true))
            XCTFail("Archived a waiting session")
        } catch {
            XCTAssertEqual((error as? DaemonError).flatMap(code), .busy)
        }
    }

    func testAnUnknownSessionIsRefused() async {
        let daemon = InMemoryDaemon()
        do {
            _ = try await daemon.client.history("nope")
            XCTFail("Read an unknown session")
        } catch {
            XCTAssertEqual((error as? DaemonError).flatMap(code), .unknownSession)
        }
    }

    private func board(_ daemon: InMemoryDaemon, _ project: String) async throws -> [String] {
        try await daemon.client.items(project).map { "\($0.title): \($0.status)" }
    }

    func testASessionOnASubtaskMovesItAndItsTask() async throws {
        let daemon = InMemoryDaemon(script: edit)
        let project = try await daemon.client.createProject("demo", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let form = try await daemon.client.createItem(project.id, ItemFields(parentID: task.id, title: "form"))
        _ = try await daemon.client.createItem(project.id, ItemFields(parentID: task.id, title: "copy"))

        let session = try await daemon.client.start(StartRequest(backend: .scripted, itemID: form.id))
        XCTAssertEqual(session.projectID, project.id)
        try await daemon.client.send(session.id, "write it", .enqueue)

        let waiting = try await board(daemon, project.id)
        XCTAssertEqual(waiting, ["login: waitingOnYou", "form: waitingOnYou", "copy: todo"])
        let inbox = try await daemon.client.inbox()
        XCTAssertEqual(inbox.map(\.reason), [.approval])
    }

    func testDoneStaysDoneAndLeavesTheInbox() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("demo", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        let session = try await daemon.client.start(StartRequest(backend: .scripted, itemID: task.id))
        try await daemon.client.send(session.id, "hello", .enqueue)
        let reviewed = try await board(daemon, project.id)
        XCTAssertEqual(reviewed, ["login: inReview"])
        let replies = try await daemon.client.inbox()
        XCTAssertEqual(replies.map(\.reason), [.reply])

        _ = try await daemon.client.updateItem(task.id, ItemUpdate(status: .done))

        let done = try await board(daemon, project.id)
        XCTAssertEqual(done, ["login: done"])
        let inbox = try await daemon.client.inbox()
        XCTAssertEqual(inbox, [])
    }

    func testEveryBoardChangeCanBeReverted() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("demo", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        _ = try await daemon.client.updateItem(task.id, ItemUpdate(title: "sign in", priority: .high))
        let log = try await daemon.client.boardLog(project.id)
        let (update, add) = (try XCTUnwrap(log.first), try XCTUnwrap(log.last))

        let reverted = try await daemon.client.revert(update.id)
        XCTAssertEqual(reverted?.title, "login")
        XCTAssertEqual(reverted?.priority, Priority.none)
        let removed = try await daemon.client.revert(add.id)
        XCTAssertNil(removed)
        let empty = try await board(daemon, project.id)
        XCTAssertEqual(empty, [])

        let removals = try await daemon.client.boardLog(project.id)
        let removal = try XCTUnwrap(removals.first)
        let back = try await daemon.client.revert(removal.id)
        XCTAssertEqual(back?.title, "login")
    }

    func testAnItemInUseIsNotRemoved() async throws {
        let daemon = InMemoryDaemon()
        let project = try await daemon.client.createProject("demo", [Repository(path: "/code")], "")
        let task = try await daemon.client.createItem(project.id, ItemFields(title: "login"))
        _ = try await daemon.client.start(StartRequest(backend: .scripted, itemID: task.id))
        let log = try await daemon.client.boardLog(project.id)
        let add = try XCTUnwrap(log.last)
        do {
            _ = try await daemon.client.revert(add.id)
            XCTFail("Removed an item with a session")
        } catch {
            XCTAssertEqual((error as? DaemonError).flatMap(code), .busy)
        }
    }

    private func code(_ error: DaemonError) -> ErrorCode? {
        if case .refused(let code, _) = error { code } else { nil }
    }
}
