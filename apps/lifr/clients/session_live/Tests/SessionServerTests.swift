import Foundation
import LifrAPI
import LifrCore
import LifrSessionClient
import XCTest
import os

@testable import LifrSessionLive

final class SessionServerTests: XCTestCase {
    func testAProcessThatFailsTheGateIsRefusedBeforeItCanAsk() async throws {
        let harness = try await Harness()
        harness.peer.withLock { $0 = .failure(.notAuthorized("Not the CLI.")) }
        let client = try harness.connect()
        let refusal = try await background { try client.receive() }
        XCTAssertEqual(refusal?.error?.data.code, .notAuthorized)
        XCTAssertNil(refusal?.id)
        let closed = try await background { try client.receive() }
        XCTAssertNil(closed)
    }

    func testTheCodeSigningCheckRefusesAnythingButTheBundledCLI() async throws {
        // This test process is signed by Apple, not as Lifr's CLI.
        let harness = try await Harness(
            peers: .codeSigning(
                cli: URL(fileURLWithPath: "/Applications/Lifr.app/Contents/Helpers/lifr"), team: LocalSocket.team,
                identifier: LocalSocket.cliIdentifier))
        let client = try harness.connect()
        let refusal = try await background { try client.receive() }
        XCTAssertEqual(refusal?.error?.data.code, .notAuthorized)
    }

    func testAnUnknownAppWaitsForApprovalAndKeepsItsGrant() async throws {
        let harness = try await Harness()
        let events = await harness.server.client.events()
        let client = try harness.connect()
        try await background { try client.hello() }

        async let answer = background { try client.call(.listAvenues) }
        let request = try await next(events) {
            if case .approvalRequested(let request) = $0 { request } else { nil }
        }
        XCTAssertEqual(request.identity, "com.anthropic.claude-code")
        XCTAssertEqual(request.client, "Claude Code")
        XCTAssertEqual(request.method, .listAvenues)
        try await harness.server.client.approve(
            request.id, Grant(client: "Claude Code", scopes: [.read], expiresAt: harness.now + 86_400))
        let response = try await answer
        XCTAssertNil(response.error)
        XCTAssertEqual(harness.touchID.withLock { $0.reasons.count }, 1)

        // A new connection from the same app needs no approval.
        let again = try harness.connect()
        let second = try await background { try again.call(.listAvenues) }
        XCTAssertNil(second.error)
        let apps = try await harness.server.client.apps()
        XCTAssertEqual(apps.map(\.id), ["com.anthropic.claude-code"])
        XCTAssertEqual(apps.first?.isConnected, true)
        XCTAssertEqual(apps.first?.lastUsedAt, harness.now)
        XCTAssertEqual(try harness.grants.load().map(\.identity), ["com.anthropic.claude-code"])
    }

    func testApprovingNeedsTouchID() async throws {
        let harness = try await Harness()
        harness.touchID.withLock { $0.answer = false }
        let events = await harness.server.client.events()
        let client = try harness.connect()
        async let answer = background { try client.call(.listAvenues) }
        let request = try await next(events) {
            if case .approvalRequested(let request) = $0 { request } else { nil }
        }
        do {
            try await harness.server.client.approve(
                request.id, Grant(client: "Claude Code", scopes: [.read], expiresAt: harness.now + 86_400))
            XCTFail("Approved without Touch ID.")
        } catch {
            XCTAssertEqual(error as? SessionError, .notAuthenticated)
        }
        // The request still waits; denying it refuses the CLI.
        try await harness.server.client.deny(request.id)
        let response = try await answer
        XCTAssertEqual(response.error?.data.code, .notAuthorized)
        XCTAssertEqual(try harness.grants.load(), [])
    }

    func testAKeychainThatWouldntOpenAtLaunchKeepsItsGrantsWhenAnAppIsApproved() async throws {
        let kept = StoredGrant(
            identity: "com.example.agent", team: nil,
            grant: Grant(client: "Agent", scopes: [.read], expiresAt: Date(timeIntervalSince1970: 1_900_000_000)))
        let locked = OSAllocatedUnfairLock(initialState: (isLocked: true, grants: [kept]))
        let storage = GrantStorage(
            load: { () throws(SessionError) in
                let (isLocked, grants) = locked.withLock { ($0.isLocked, $0.grants) }
                if isLocked { throw .grantsNotSaved(reason: "locked") }
                return grants
            },
            save: { grants in locked.withLock { $0.grants = grants } })
        let harness = try await Harness(storage: storage)
        let events = await harness.server.client.events()
        let client = try harness.connect()
        async let answer = background { try client.call(.listAvenues) }
        let request = try await next(events) {
            if case .approvalRequested(let request) = $0 { request } else { nil }
        }
        let grant = Grant(client: "Claude Code", scopes: [.read], expiresAt: harness.now + 86_400)

        // Still locked: nothing is saved over the kept grants.
        do {
            try await harness.server.client.approve(request.id, grant)
            XCTFail("Saved over grants it couldn't read.")
        } catch {
            XCTAssertNotNil(error as? SessionError)
        }
        XCTAssertEqual(locked.withLock { $0.grants }, [kept])

        locked.withLock { $0.isLocked = false }
        try await harness.server.client.approve(request.id, grant)
        _ = try await answer
        XCTAssertEqual(
            Set(locked.withLock { $0.grants }.map(\.identity)), ["com.example.agent", "com.anthropic.claude-code"])
    }

    func testACLIThatStopsWaitingWithdrawsItsRequest() async throws {
        let harness = try await Harness()
        let events = await harness.server.client.events()
        let client = try harness.connect()
        try await background { try client.send(.searchTasks) }
        let request = try await next(events) {
            if case .approvalRequested(let request) = $0 { request } else { nil }
        }
        client.close()
        let withdrawn = try await next(events) {
            if case .approvalWithdrawn(let id) = $0 { id } else { nil }
        }
        XCTAssertEqual(withdrawn, request.id)
    }

    func testAnAgentsChangeIsSavedAndShownInTheApp() async throws {
        let harness = try await Harness(grants: [Harness.approved([.read, .write])])
        let events = await harness.server.client.events()
        let client = try harness.connect()
        let response = try await background {
            try client.call(.createTask, ["avenue": "HEALTH", "title": "Book the dentist"])
        }
        XCTAssertNil(response.error)
        let shown = try await next(events) {
            if case .recordsChanged(let workspace, _) = $0 { workspace } else { nil }
        }
        let saved = try await harness.store.load()
        XCTAssertEqual(saved, shown)
        let task = try XCTUnwrap(saved.tasks.values.first)
        XCTAssertEqual(task.title, "Book the dentist")
        XCTAssertEqual(saved.history.first?.actor, .agent("Claude Code", device: "Test Mac", surface: .cli))
        XCTAssertEqual(saved.journal.last?.actor, .agent("Claude Code", device: "Test Mac", surface: .cli))
    }

    func testAChangeOverMCPIsCreditedToMCP() async throws {
        let harness = try await Harness(grants: [Harness.approved([.read, .write])])
        let client = try harness.connect()
        let response = try await background {
            try client.call(.createTask, ["avenue": "HEALTH", "title": "Book the dentist"], surface: .mcp)
        }
        XCTAssertNil(response.error)
        let saved = try await harness.store.load()
        XCTAssertEqual(saved.journal.last?.actor, .agent("Claude Code", device: "Test Mac", surface: .mcp))
    }

    func testAReadOnlyGrantCantChangeAnything() async throws {
        let harness = try await Harness(grants: [Harness.approved([.read])])
        let client = try harness.connect()
        let response = try await background {
            try client.call(.createTask, ["avenue": "HEALTH", "title": "Book the dentist"])
        }
        XCTAssertEqual(response.error?.data.code, .notAuthorized)
        let tasks = try await harness.store.load().tasks
        XCTAssertEqual(tasks, [:])
    }

    func testTrashingAsksForTouchIDAndARefusalChangesNothing() async throws {
        let harness = try await Harness(grants: [Harness.approved([.read, .write])])
        let client = try harness.connect()
        let created = try await background {
            try client.call(.createTask, ["avenue": "HEALTH", "title": "Book the dentist"])
        }
        let code = try XCTUnwrap(created.result?.decode(ChangeResult.self).task.code)

        harness.touchID.withLock { $0.answer = false }
        let refused = try await background { try client.call(.trashTask, ["task": .string(code)]) }
        XCTAssertEqual(refused.error?.data.code, .confirmationDenied)
        var tasks = try await harness.store.load().tasks.values
        XCTAssertNil(tasks.first?.deletedAt)

        harness.touchID.withLock { $0.answer = true }
        let trashed = try await background { try client.call(.trashTask, ["task": .string(code)]) }
        XCTAssertNil(trashed.error)
        tasks = try await harness.store.load().tasks.values
        XCTAssertNotNil(tasks.first?.deletedAt)
        XCTAssertEqual(harness.touchID.withLock { $0.reasons.count }, 2)
    }

    func testAnAppSignedByAnotherTeamIsRefused() async throws {
        let harness = try await Harness(grants: [Harness.approved([.read], team: "SOMEONEELSE")])
        let client = try harness.connect()
        let response = try await background { try client.call(.listAvenues) }
        XCTAssertEqual(response.error?.data.code, .notAuthorized)
    }

    func testATerminalActsAsYouOnlyAfterUnlockAndOnlyInItsOwnTTY() async throws {
        let harness = try await Harness(peer: Harness.terminal)
        let client = try harness.connect()
        let locked = try await background { try client.call(.listAvenues) }
        XCTAssertEqual(locked.error?.data.code, .notAuthorized)

        let unlocked = try await background { try client.call(.unlock) }
        XCTAssertNil(unlocked.error)
        let listed = try await background { try client.call(.listAvenues) }
        XCTAssertNil(listed.error)

        var other = Harness.terminal
        other.tty = 6
        harness.peer.withLock { $0 = .success(other) }
        let otherClient = try harness.connect()
        let refused = try await background { try otherClient.call(.listAvenues) }
        XCTAssertEqual(refused.error?.data.code, .notAuthorized)
    }

    func testOnlyATerminalCanUnlock() async throws {
        let harness = try await Harness()
        let client = try harness.connect()
        let response = try await background { try client.call(.unlock) }
        XCTAssertEqual(response.error?.data.code, .notAuthorized)
        XCTAssertEqual(harness.touchID.withLock { $0.reasons }, [])
    }

    func testTurningTheAPIOffClosesTheSocketAndTellsTheCLI() async throws {
        let harness = try await Harness(grants: [Harness.approved([.read])])
        let client = try harness.connect()
        try await harness.server.client.setAPIEnabled(false)
        let isEnabled = await harness.server.client.isAPIEnabled()
        XCTAssertFalse(isEnabled)
        XCTAssertTrue(
            FileManager.default.fileExists(atPath: harness.folder.appendingPathComponent("api-disabled").path))
        let closed = try await background { try client.receive() }
        XCTAssertNil(closed)
        XCTAssertThrowsError(try harness.connect())

        try await harness.server.client.setAPIEnabled(true)
        let reconnected = try harness.connect()
        let response = try await background { try reconnected.call(.listAvenues) }
        XCTAssertNil(response.error)
    }

    func testEveryRequestIsLogged() async throws {
        let harness = try await Harness(grants: [Harness.approved([.read])])
        let client = try harness.connect()
        _ = try await background { try client.call(.getTask, ["task": "HEALTH-7K2Q"]) }
        let log = AccessLog(file: harness.folder.appendingPathComponent("access.log"))
        XCTAssertEqual(
            log.entries(),
            [
                AccessLog.Entry(
                    at: harness.now, client: "Claude Code", method: "get_task", tasks: ["HEALTH-7K2Q"],
                    result: "notFound")
            ])
    }
}

final class MachineTests: XCTestCase {
    private var folder: URL!

    override func setUp() {
        folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: folder)
    }

    func testTheAccessLogKeeps90Days() {
        let log = AccessLog(file: folder.appendingPathComponent("access.log"))
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let old = AccessLog.Entry(at: now - 91 * 86_400, client: "A", method: "get_task", tasks: [], result: "ok")
        let recent = AccessLog.Entry(at: now - 89 * 86_400, client: "B", method: "get_task", tasks: [], result: "ok")
        log.append(old)
        log.append(recent)
        log.prune(now: now)
        XCTAssertEqual(log.entries(), [recent])
    }

    func testInstallingLinksTheBundledCLIOnce() throws {
        let helper = folder.appendingPathComponent("Lifr.app/Contents/Helpers/lifr")
        try FileManager.default.createDirectory(
            at: helper.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data().write(to: helper)
        let link = CLILink(helper: helper, directories: [folder.appendingPathComponent("bin")])
        XCTAssertEqual(link.installation(), .notInstalled)
        let path = folder.appendingPathComponent("bin/lifr").path
        XCTAssertEqual(try link.install(), .installed(path: path))
        XCTAssertEqual(link.installation(), .installed(path: path))
        XCTAssertEqual(try link.install(), .installed(path: path))
    }

    func testInstallingLeavesAnotherLifrAlone() throws {
        let bin = folder.appendingPathComponent("bin")
        try FileManager.default.createDirectory(at: bin, withIntermediateDirectories: true)
        try Data("other".utf8).write(to: bin.appendingPathComponent("lifr"))
        let link = CLILink(helper: folder.appendingPathComponent("lifr"), directories: [bin])
        XCTAssertThrowsError(try link.install()) { error in
            guard case .installFailed? = error as? SessionError else { return XCTFail("\(error)") }
        }
        XCTAssertEqual(try Data(contentsOf: bin.appendingPathComponent("lifr")), Data("other".utf8))
    }
}
