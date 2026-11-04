import ComposableArchitecture
import Foundation
import LifrAPI
import LifrSessionClient
import XCTest

final class InMemorySessionTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func request(_ id: UUID = UUID(0)) -> ApprovalRequest {
        ApprovalRequest(
            id: id, identity: "com.anthropic.claude-code", client: "Claude Code", method: .searchTasks,
            requestedAt: now)
    }

    func testApprovingARequestConnectsTheApp() async throws {
        let session = InMemorySession()
        let events = await session.client.events()
        session.request(request())

        try await session.client.approve(UUID(0), .agentDefault(client: "Claude Code", now: now))

        let apps = try await session.client.apps()
        XCTAssertEqual(apps.map(\.name), ["Claude Code"])
        XCTAssertEqual(apps.first?.grant.scopes, [.read])
        var iterator = events.makeAsyncIterator()
        let first = await iterator.next()
        XCTAssertEqual(first, .approvalRequested(request()))
        let second = await iterator.next()
        XCTAssertEqual(second, .appsChanged(apps))
    }

    func testAnsweringAWithdrawnRequestFails() async {
        let session = InMemorySession()
        session.request(request())
        session.withdraw(UUID(0))

        do {
            try await session.client.approve(UUID(0), .agentDefault(client: "Claude Code", now: now))
            XCTFail("Approved a withdrawn request")
        } catch {
            XCTAssertEqual(error as? SessionError, .unknownRequest(UUID(0)))
        }
        XCTAssertTrue(session.apps.isEmpty)
    }

    func testRevokingRemovesTheGrant() async throws {
        let app = ConnectedApp(id: "com.anthropic.claude-code", grant: .agentDefault(client: "Claude Code", now: now))
        let session = InMemorySession(apps: [app])

        try await session.client.revoke(app.id)

        XCTAssertTrue(session.apps.isEmpty)
        do {
            try await session.client.revoke(app.id)
            XCTFail("Revoked twice")
        } catch {
            XCTAssertEqual(error as? SessionError, .unknownApp(app.id))
        }
    }

    func testListsMostRecentlyUsedFirst() async throws {
        let session = InMemorySession(apps: [
            ConnectedApp(id: "a", grant: .agentDefault(client: "Alpha", now: now), lastUsedAt: now - 60),
            ConnectedApp(id: "b", grant: .agentDefault(client: "Beta", now: now), lastUsedAt: now),
            ConnectedApp(id: "c", grant: .agentDefault(client: "Gamma", now: now)),
        ])

        let apps = try await session.client.apps()

        XCTAssertEqual(apps.map(\.name), ["Beta", "Alpha", "Gamma"])
    }

    func testInstallingTheCLIRecordsWhere() async throws {
        let session = InMemorySession()

        let installed = try await session.client.installCLI()

        XCTAssertEqual(installed, .installed(path: "/usr/local/bin/lifr"))
        let current = await session.client.cliInstallation()
        XCTAssertEqual(current, installed)
    }
}
