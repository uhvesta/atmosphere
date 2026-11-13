import ComposableArchitecture
import Foundation
import LifrAPI
import LifrAssistantClient
import LifrCore
import LifrLockClient
import LifrRemindersClient
import LifrSessionClient
import LifrSettings
import LifrStoreClient
import LifrUI
import XCTest

@MainActor
final class SettingsFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")
    private lazy var claude = ConnectedApp(
        id: "com.anthropic.claude-code", grant: .agentDefault(client: "Claude Code", now: now), lastUsedAt: now)

    private func settings(_ session: InMemorySession, lock: LockClient = .always()) async
        -> TestStoreOf<SettingsFeature>
    {
        let store = TestStore(initialState: SettingsFeature.State(showsAgents: true)) {
            SettingsFeature()
        } withDependencies: { [now, records] in
            $0.conversations = .inMemory()
            $0.store = .inMemory(records.workspace)
            $0.session = session.client
            $0.lock = lock
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

    func testTurningTheLockOnOrOffAsksFirst() async {
        let store = await settings(InMemorySession(), lock: .always(.touchID))
        XCTAssertEqual(store.state.biometry, .touchID)

        await store.send(.lockToggled(true))
        await store.receive(\.lockConfirmed)
        XCTAssertTrue(store.state.locksApp)

        await store.send(.lockToggled(false))
        await store.receive(\.lockConfirmed)
        XCTAssertFalse(store.state.locksApp)
        await store.skipInFlightEffects()
    }

    func testClosingThePromptLeavesTheLockAsItWas() async {
        let store = await settings(InMemorySession(), lock: .always(failing: .cancelled))

        await store.send(.lockToggled(true))
        await store.receive(\.lockConfirmed)
        XCTAssertFalse(store.state.locksApp)
        XCTAssertNil(store.state.alert)
        await store.skipInFlightEffects()
    }

    func testALockTheDeviceCantBackSaysWhy() async {
        let store = await settings(InMemorySession(), lock: .always(failing: .noPasscode))

        await store.send(.lockToggled(true))
        await store.receive(\.lockConfirmed)
        XCTAssertFalse(store.state.locksApp)
        XCTAssertNotNil(store.state.alert)
        await store.skipInFlightEffects()
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

    func testOpeningAScreenPicksTheTabItIsOn() async {
        let store = await settings(InMemorySession())
        XCTAssertEqual(store.state.page, .general)

        await store.send(.pagePicked(.agents))
        XCTAssertEqual(store.state.page, .agents)

        await store.send(.modelsTapped)
        XCTAssertEqual(store.state.page, .assistant)
        XCTAssertNotNil(store.state.models)

        await store.send(.activityTapped)
        XCTAssertEqual(store.state.page, .general)
        await store.skipInFlightEffects()
    }

    func testDeletingAllDataNeedsTheWordTypedThenLeavesOnlyAnEmptyInbox() async throws {
        _ = health
        let store = await settings(InMemorySession())

        let before = try await store.dependencies.store.load()
        await store.send(.eraseTapped)
        await store.send(\.erase.binding.typed, "Del")
        await store.send(\.erase.eraseTapped)
        let untouched = try await store.dependencies.store.load()
        XCTAssertEqual(untouched, before)

        await store.send(\.erase.binding.typed, " delete ")
        await store.send(\.erase.eraseTapped)
        await store.receive(\.erase.erased)
        await store.receive(\.erase.delegate.erased)
        await store.receive(\.delegate.saved)
        await store.receive(\.erase.dismiss)

        let erased = try await store.dependencies.store.load()
        XCTAssertEqual(erased.avenues.keys.map { $0 }, [Avenue.inboxID])
        XCTAssertTrue(store.state.avenues.allSatisfy { $0.id == Avenue.inboxID })
        XCTAssertNil(store.state.erase)
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

    func testTurningTheDailySummaryOnAsksForNotificationsOnce() async throws {
        let store = await settings(InMemorySession())
        let asked = LockIsolated(0)
        store.dependencies.reminders.requestPermission = {
            asked.withValue { $0 += 1 }
            return true
        }
        let nine = try TimeOfDay("09:00")
        await store.send(.dailySummarySet(DailySummary.morning)).finish()
        await store.send(.dailySummarySet(nine)).finish()
        XCTAssertEqual(store.state.dailySummary.time, nine)
        XCTAssertEqual(asked.value, 1)

        await store.send(.dailySummarySet(nil))
        XCTAssertNil(store.state.dailySummary.time)
        await store.skipInFlightEffects()
    }

    func testPickingADefaultReminderSyncsItAndKeepsTheOtherSettings() async {
        let store = await settings(InMemorySession())
        await store.send(.procrastinatedDaysChanged(14))
        await store.receive(\.delegate.saved)

        await store.send(.defaultReminderPicked(.dueDayMorning))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.defaultReminder, .dueDayMorning)
        let saved = try? await store.dependencies.store.load()
        XCTAssertEqual(saved?.settings, Settings(procrastinatedAfterDays: 14, defaultReminder: .dueDayMorning))

        await store.send(.procrastinatedDaysChanged(10))
        await store.receive(\.delegate.saved)
        let resaved = try? await store.dependencies.store.load()
        XCTAssertEqual(resaved?.settings.defaultReminder, .dueDayMorning)
        await store.skipInFlightEffects()
    }

    func testAProcrastinatedDaysChangeThatIsntSavedGoesBack() async {
        struct Full: Error {}
        let store = await settings(InMemorySession())
        let stored = store.state.procrastinatedAfterDays
        store.dependencies.store.save = { _ in throw Full() }

        await store.send(.procrastinatedDaysChanged(stored + 5))
        await store.receive(\.saved)

        XCTAssertEqual(store.state.procrastinatedAfterDays, stored)
        XCTAssertNotNil(store.state.alert)
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
        await failing.receive(\.cliInstallFailed)
        XCTAssertNotNil(failing.state.alert)
        XCTAssertEqual(failing.state.cli, .notInstalled)
        await failing.skipInFlightEffects()
    }

    func testSettingsThatFailToLoadCanBeLoadedAgain() async {
        struct Unreadable: Error {}
        let failing = LockIsolated(true)
        let stored = StoreClient.inMemory(records.workspace)
        var client = stored
        client.load = {
            if failing.value { throw Unreadable() }
            return try await stored.load()
        }
        let store = TestStore(initialState: SettingsFeature.State(showsAgents: false)) {
            SettingsFeature()
        } withDependencies: { [client] in
            $0.conversations = .inMemory()
            $0.store = client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.loadError)

        failing.setValue(false)
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.loadError)
        await store.finish()
    }

    func testResetTipsShowsEveryTipAgain() async {
        for tip in GestureTip.allCases {
            Shared(wrappedValue: false, .appStorage(tip.storageKey)).withLock { $0 = true }
        }
        let store = TestStore(initialState: SettingsFeature.State(showsAgents: false)) {
            SettingsFeature()
        }
        store.exhaustivity = .off

        await store.send(.resetTipsTapped)

        for tip in GestureTip.allCases {
            XCTAssertFalse(Shared(wrappedValue: true, .appStorage(tip.storageKey)).wrappedValue)
        }
        XCTAssertTrue(store.state.tipsReset)
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

    func testLaunchAtLoginIsReadAndTurnedOn() async {
        let session = InMemorySession()
        let store = await settings(session)
        await store.receive(\.launchAtLoginLoaded)
        XCTAssertFalse(store.state.launchesAtLogin)

        await store.send(.launchAtLoginToggled(true)).finish()

        XCTAssertTrue(store.state.launchesAtLogin)
        let launches = await session.client.launchesAtLogin()
        XCTAssertTrue(launches)
        await store.skipInFlightEffects()
    }

    func testALoginItemMacOSRefusesPutsTheSwitchBackAndSaysWhy() async {
        let store = await settings(InMemorySession())
        await store.receive(\.launchAtLoginLoaded)
        store.dependencies.session.setLaunchesAtLogin = { _ in
            throw SessionError.loginItemFailed(reason: "not allowed")
        }

        await store.send(.launchAtLoginToggled(true))
        await store.receive(\.launchAtLoginLoaded)
        await store.receive(\.loginItemFailed)

        XCTAssertFalse(store.state.launchesAtLogin)
        XCTAssertNotNil(store.state.alert)
        await store.skipInFlightEffects()
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
