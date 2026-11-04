import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrSessionClient
import LifrSettings
import LifrStoreClient
import XCTest

@MainActor
final class SettingsFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")
    private lazy var claude = ConnectedApp(
        id: "com.anthropic.claude-code", grant: .agentDefault(client: "Claude Code", now: now), lastUsedAt: now)

    private func settings(_ session: InMemorySession) async -> TestStoreOf<SettingsFeature> {
        let store = TestStore(initialState: SettingsFeature.State(showsAgents: true)) {
            SettingsFeature()
        } withDependencies: { [now, records] in
            $0.conversations = .inMemory()
            $0.store = .inMemory(records.workspace)
            $0.session = session.client
            $0.date.now = now
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.appsLoaded)
        await store.receive(\.cliLoaded)
        return store
    }

    func testShowsSettingsAvenuesAndApps() async {
        _ = health
        let store = await settings(InMemorySession(apps: [claude]))

        XCTAssertEqual(store.state.procrastinatedAfterDays, 7)
        XCTAssertEqual(store.state.avenues.map(\.name), ["Health"])
        XCTAssertEqual(store.state.apps.map(\.name), ["Claude Code"])
        XCTAssertEqual(store.state.cli, .notInstalled)
        await store.skipInFlightEffects()
    }

    func testChangesWhenTasksCountAsProcrastinated() async {
        let store = await settings(InMemorySession())

        await store.send(.procrastinatedDaysChanged(14))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.procrastinatedAfterDays, 14)
        let saved = try? await store.dependencies.store.load()
        XCTAssertEqual(saved?.settings.procrastinatedAfterDays, 14)

        await store.send(.procrastinatedDaysChanged(0))
        XCTAssertEqual(store.state.procrastinatedAfterDays, 1)
        await store.skipInFlightEffects()
    }

    func testHidesAnAvenueFromAgents() async {
        _ = health
        let store = await settings(InMemorySession())

        await store.send(.agentAccessPicked(health.id, .hidden))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.avenues.map(\.access), [.hidden])
        await store.skipInFlightEffects()
    }

    func testLetsAnAppMakeChangesAndRenewsIt() async {
        let session = InMemorySession(apps: [claude])
        let store = await settings(session)

        await store.send(.writePicked(claude.id, true))
        await store.receive(\.appsLoaded)
        XCTAssertEqual(store.state.apps.first?.grant.scopes, [.read, .write])

        await store.send(.renewTapped(claude.id))
        await store.receive(\.appsLoaded)
        XCTAssertEqual(session.apps.first?.grant.expiresAt, now + 30 * 24 * 60 * 60)
        await store.skipInFlightEffects()
    }

    func testRevokesAfterConfirming() async {
        let session = InMemorySession(apps: [claude])
        let store = await settings(session)

        await store.send(.revokeTapped(claude.id))
        XCTAssertNotNil(store.state.revoke)
        await store.send(.revoke(.presented(.confirmed(claude.id))))
        await store.receive(\.appsLoaded)

        XCTAssertTrue(store.state.apps.isEmpty)
        XCTAssertTrue(session.apps.isEmpty)
        await store.skipInFlightEffects()
    }

    func testFollowsAppsUsedElsewhere() async {
        let session = InMemorySession(apps: [claude])
        let store = await settings(session)

        session.use(claude.id, at: now + 60, connected: false)
        await store.receive(\.event)

        XCTAssertEqual(store.state.apps.first?.lastUsedAt, now + 60)
        XCTAssertEqual(store.state.apps.first?.isConnected, false)
        await store.skipInFlightEffects()
    }

    func testInstallsTheCLIOrSaysWhyNot() async {
        let store = await settings(InMemorySession())
        await store.send(.installCLITapped)
        await store.receive(\.cliLoaded)
        XCTAssertEqual(store.state.cli, .installed(path: "/usr/local/bin/lifr"))
        await store.skipInFlightEffects()

        let failing = await settings(
            InMemorySession(
                installResult: .failure(.installFailed(path: "/usr/local/bin/lifr", reason: "Permission denied"))))
        await failing.send(.installCLITapped)
        await failing.receive(\.sessionChanged)
        XCTAssertNotNil(failing.state.alert)
        XCTAssertEqual(failing.state.cli, .notInstalled)
        await failing.skipInFlightEffects()
    }

    func testOnIPhoneLeavesAgentsOut() async {
        let store = TestStore(initialState: SettingsFeature.State(showsAgents: false)) {
            SettingsFeature()
        } withDependencies: { [records] in
            $0.conversations = .inMemory()
            $0.store = .inMemory(records.workspace)
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.finish()
    }
}

@MainActor
final class ApprovalFeatureTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")
    private lazy var finance = records.avenue("FINANCE", "Finance")
    private let request = ApprovalRequest(
        id: UUID(0), identity: "com.anthropic.claude-code", client: "Claude Code", method: .searchTasks,
        requestedAt: Date(timeIntervalSince1970: 1_793_700_000))

    private func approval(_ session: InMemorySession) -> TestStoreOf<ApprovalFeature> {
        session.request(request)
        let store = TestStore(initialState: ApprovalFeature.State(request: request, workspace: records.workspace)) {
            ApprovalFeature()
        } withDependencies: { [now] in
            $0.session = session.client
            $0.date.now = now
            $0.dismiss = DismissEffect {}
        }
        store.exhaustivity = .off
        return store
    }

    func testApprovesReadOnlyForThirtyDaysByDefault() async {
        _ = health
        let session = InMemorySession()
        let store = approval(session)

        await store.send(.approveTapped)
        await store.receive(\.answered)

        XCTAssertEqual(session.apps.first?.grant, .agentDefault(client: "Claude Code", now: now))
    }

    func testDoesntOfferHiddenAvenues() throws {
        _ = (health, finance)
        var workspace = records.workspace
        try workspace.setAgentAccess(.hidden, ofAvenue: finance.id)

        let state = ApprovalFeature.State(request: request, workspace: workspace)

        XCTAssertEqual(state.avenues.map(\.name), ["Health"])
    }

    func testApprovesChangesInChosenAvenues() async {
        _ = (health, finance)
        let session = InMemorySession()
        let store = approval(session)

        await store.send(.binding(.set(\.canWrite, true)))
        await store.send(.binding(.set(\.confirmWrites, true)))
        await store.send(.binding(.set(\.allAvenues, false)))
        XCTAssertFalse(store.state.canApprove)
        await store.send(.binding(.set(\.avenueIDs, [health.id])))
        await store.send(.binding(.set(\.expiry, .week)))
        await store.send(.approveTapped)
        await store.receive(\.answered)

        let grant = session.apps.first?.grant
        XCTAssertEqual(grant?.scopes, [.read, .write])
        XCTAssertEqual(grant?.avenueIDs, [health.id])
        XCTAssertEqual(grant?.confirmWrites, true)
        XCTAssertEqual(grant?.expiresAt, now + 7 * 24 * 60 * 60)
    }

    func testDenyingGrantsNothing() async {
        let session = InMemorySession()
        let store = approval(session)

        await store.send(.denyTapped)
        await store.receive(\.answered)

        XCTAssertTrue(session.apps.isEmpty)
    }

    func testSaysWhenTheAppStoppedWaiting() async {
        let session = InMemorySession()
        let store = approval(session)
        session.withdraw(request.id)

        await store.send(.approveTapped)
        await store.receive(\.answered)

        XCTAssertNotNil(store.state.error)
        XCTAssertTrue(session.apps.isEmpty)
    }
}
