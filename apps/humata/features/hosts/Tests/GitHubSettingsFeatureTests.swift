import ComposableArchitecture
import HumataDaemonClient
import HumataHosts
import XCTest

@MainActor
final class GitHubSettingsFeatureTests: XCTestCase {
    private func store(_ daemon: InMemoryDaemon) -> TestStoreOf<GitHubSettingsFeature> {
        let store = TestStore(initialState: GitHubSettingsFeature.State()) {
            GitHubSettingsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off
        return store
    }

    func testPullRequestsAreReadAutomaticallyUntilSetToOnlyWhenAsked() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon)

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.refresh, PullRefresh(automatic: true, everyMinutes: 2))
        XCTAssertEqual(store.state.every, .minutes(2))

        await store.send(.automaticPicked(false))
        XCTAssertTrue(store.state.saving)
        await store.receive(\.saved)
        XCTAssertFalse(store.state.saving)
        let saved = try await daemon.client.pullRefresh()
        XCTAssertFalse(saved.automatic)
        XCTAssertEqual(saved.everyMinutes, 2)
    }

    func testAnOfferedIntervalIsSavedAtOnce() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.everyPicked(.minutes(30)))
        await store.receive(\.saved)
        XCTAssertEqual(store.state.every, .minutes(30))
        let saved = try await daemon.client.pullRefresh()
        XCTAssertEqual(saved, PullRefresh(automatic: true, everyMinutes: 30))
    }

    func testCustomMinutesAreSavedOnlyOnceTypedAndInRange() async throws {
        let daemon = InMemoryDaemon()
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.everyPicked(.custom))
        XCTAssertEqual(store.state.every, .custom)
        XCTAssertFalse(store.state.saving)

        await store.send(\.binding.customMinutes, "2000")
        await store.send(.customMinutesSubmitted)
        XCTAssertNotNil(store.state.failure)
        XCTAssertFalse(store.state.saving)
        let unchanged = try await daemon.client.pullRefresh()
        XCTAssertEqual(unchanged.everyMinutes, 2)

        await store.send(\.binding.customMinutes, "90")
        await store.send(.customMinutesSubmitted)
        await store.receive(\.saved)
        XCTAssertNil(store.state.failure)
        XCTAssertEqual(store.state.every, .custom)
        let saved = try await daemon.client.pullRefresh()
        XCTAssertEqual(saved.everyMinutes, 90)
    }

    func testMinutesSavedBeforeThatArentOfferedShowAsCustom() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.setPullRefresh(PullRefresh(automatic: true, everyMinutes: 45))
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.every, .custom)
        XCTAssertEqual(store.state.customMinutes, "45")
    }

    func testAChangeThatWasntSavedGoesBackAndSaysWhy() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: GitHubSettingsFeature.State()) {
            GitHubSettingsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.setPullRefresh = { _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.automaticPicked(false))
        XCTAssertEqual(store.state.refresh?.automatic, false)
        await store.receive(\.saveFailed)
        XCTAssertEqual(store.state.refresh?.automatic, true)
        XCTAssertFalse(store.state.saving)
        XCTAssertNotNil(store.state.failure)
    }

    func testSettingsThatCantBeReadSayWhyAndCanBeTriedAgain() async throws {
        let daemon = InMemoryDaemon()
        let failing = LockIsolated(true)
        let store = TestStore(initialState: GitHubSettingsFeature.State()) {
            GitHubSettingsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.pullRefresh = {
                if failing.value { throw DaemonError.disconnected }
                return PullRefresh.standard
            }
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loadFailed)
        XCTAssertNil(store.state.refresh)
        XCTAssertNotNil(store.state.failure)

        failing.setValue(false)
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNil(store.state.failure)
        XCTAssertEqual(store.state.refresh, PullRefresh.standard)
    }
}
