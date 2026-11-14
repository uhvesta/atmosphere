import ComposableArchitecture
import HumataDaemonClient
import HumataHosts
import XCTest

@MainActor
final class StorageFeatureTests: XCTestCase {
    func testAutoDeleteIsOffUntilTurnedOnAndKeepsTheDaysPicked() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.storage?.autoDelete, false)
        XCTAssertEqual(store.state.storage?.days, 45)

        await store.send(.autoDeleteToggled(true))
        await store.receive(\.loaded)
        await store.send(.daysPicked(90))
        await store.receive(\.loaded)
        let saved = try await daemon.client.storage()
        XCTAssertEqual(saved.autoDelete, true)
        XCTAssertEqual(saved.days, 90)
    }

    func testAnAutoDeleteThatWasntSavedTurnsBackOff() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.setStorageSettings = { _, _ in throw DaemonError.disconnected }
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.autoDeleteToggled(true))
        XCTAssertEqual(store.state.storage?.autoDelete, true)
        await store.receive(\.settingsFailed)
        XCTAssertEqual(store.state.storage?.autoDelete, false)
        XCTAssertNotNil(store.state.actionFailure)
    }

    func testAFailedSettingDuringACleanupStaysShownThroughProgressReads() async throws {
        let daemon = InMemoryDaemon()
        daemon.startCleanup(Storage.Job(step: "Packing repositories", done: 2, total: 5))
        let clock = TestClock()
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.daemon.setStorageSettings = { _, _ in throw DaemonError.disconnected }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.storage?.job)

        await store.send(.autoDeleteToggled(true))
        await store.receive(\.settingsFailed)
        XCTAssertNotNil(store.state.actionFailure)

        // The cleanup's progress is read again, and again.
        await clock.advance(by: .milliseconds(500))
        await store.receive(\.loaded)
        await clock.advance(by: .milliseconds(500))
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.storage?.job)
        XCTAssertNotNil(store.state.actionFailure, "what wasn't saved is still said while the cleanup runs")
        XCTAssertNil(store.state.failure)

        // The next thing the person changes takes it away.
        await store.send(.daysPicked(90))
        XCTAssertNil(store.state.actionFailure)
        await store.skipInFlightEffects()
    }

    func testACleanupInProgressIsReadAgainUntilItsDone() async throws {
        let daemon = InMemoryDaemon()
        daemon.startCleanup(Storage.Job(step: "Packing repositories", done: 2, total: 5))
        let clock = TestClock()
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.storage?.job?.done, 2)

        _ = try await daemon.client.cleanUpStorage(Storage.Cleanup(compact: true))
        await clock.advance(by: .milliseconds(500))
        await store.receive(\.loaded)
        XCTAssertNil(store.state.storage?.job)
        XCTAssertNotNil(store.state.storage?.cleaned)
        // Done, so nothing is read again.
        await clock.advance(by: .seconds(5))
        await store.finish()
    }

    func testACleanupWhoseProgressCouldntBeReadOnceIsReadAgainUntilItsDone() async throws {
        let daemon = InMemoryDaemon()
        daemon.startCleanup(Storage.Job(step: "Packing repositories", done: 2, total: 5))
        let clock = TestClock()
        let fails = LockIsolated(false)
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            let read = $0.daemon.storage
            $0.daemon.storage = {
                if fails.value { throw DaemonError.disconnected }
                return try await read()
            }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.storage?.job?.done, 2)

        fails.setValue(true)
        await clock.advance(by: .milliseconds(500))
        await store.receive(\.failed)
        XCTAssertNotNil(store.state.failure)
        XCTAssertNotNil(store.state.storage?.job, "the cleanup is still shown while its progress is read again")

        // The daemon answers again, and the cleanup has ended meanwhile.
        fails.setValue(false)
        _ = try await daemon.client.cleanUpStorage(Storage.Cleanup(compact: true))
        await clock.advance(by: .seconds(2))
        await store.receive(\.loaded)
        XCTAssertNil(store.state.storage?.job)
        XCTAssertNotNil(store.state.storage?.cleaned)
        XCTAssertNil(store.state.failure)
        await clock.advance(by: .seconds(5))
        await store.finish()
    }

    func testACleanupWhoseProgressCantBeReadIsGivenUpAndSaysWhy() async throws {
        let daemon = InMemoryDaemon()
        daemon.startCleanup(Storage.Job(step: "Packing repositories", done: 2, total: 5))
        let clock = TestClock()
        let fails = LockIsolated(false)
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            let read = $0.daemon.storage
            $0.daemon.storage = {
                if fails.value { throw DaemonError.disconnected }
                return try await read()
            }
            $0.continuousClock = clock
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertNotNil(store.state.storage?.job)

        fails.setValue(true)
        await clock.advance(by: .seconds(600))
        await store.skipReceivedActions()
        XCTAssertNil(store.state.storage?.job, "the cleanup's buttons come back")
        XCTAssertNotNil(store.state.failure)
        // Given up, so nothing is read again.
        await clock.advance(by: .seconds(600))
        await store.finish()
    }

    func testDeletingArchivedSessionsAsksFirstAndKeepsKeepers() async throws {
        let daemon = InMemoryDaemon()
        let old = daemon.start.addingTimeInterval(-100 * 86_400)
        let gone = try await daemon.client.start(StartRequest(backend: .scripted, name: "Old spike"))
        let kept = try await daemon.client.start(StartRequest(backend: .scripted, name: "The keeper"))
        _ = try await daemon.client.update(gone.id, SessionUpdate(archived: true))
        _ = try await daemon.client.update(kept.id, SessionUpdate(archived: true, keeper: true))
        daemon.setLastActive(old, of: [gone.id, kept.id])
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.deleteArchivedTapped)
        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(daemon.sessions.count, 2)

        await store.send(.alert(.presented(.deleteArchived(days: 45))))
        await store.receive(\.loaded)
        XCTAssertEqual(daemon.sessions.map(\.id), [kept.id])
        XCTAssertEqual(store.state.storage?.deletedSessions, 1)
        XCTAssertEqual(store.state.storage?.skipped.map(\.sessionID), [kept.id])
    }

    func testRemovingUnusedClonesAsksFirstAndFreesTheirSpace() async throws {
        let daemon = InMemoryDaemon()
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = daemon.client
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.loaded)
        let before = try XCTUnwrap(store.state.storage)
        await store.send(.removeUnusedClonesTapped)
        XCTAssertNotNil(store.state.alert)

        await store.send(.alert(.presented(.removeUnusedClones)))
        await store.receive(\.loaded)
        let after = try XCTUnwrap(store.state.storage)
        XCTAssertEqual(after.unusedClonesBytes, 0)
        XCTAssertEqual(after.total, before.total - before.unusedClonesBytes)
    }

    func testDeletingAModelAsksFirstAndListsItNoMore() async throws {
        let models = LockIsolated([
            ModelFiles.Model(id: "qwen", name: "Qwen", bytes: 4_000_000_000, path: "/Models/qwen"),
            ModelFiles.Model(id: "gemma", name: "Gemma", bytes: 2_000_000_000, path: "/Models/gemma"),
        ])
        let store = TestStore(initialState: StorageFeature.State()) {
            StorageFeature()
        } withDependencies: {
            $0.daemon = InMemoryDaemon().client
            $0.modelFiles = ModelFiles(
                folder: "/Models",
                installed: { models.value },
                delete: { id in models.withValue { $0.removeAll { $0.id == id } } })
        }
        store.exhaustivity = .off

        await store.send(.task)
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.total, (store.state.storage?.total ?? 0) + 6_000_000_000)
        await store.send(.deleteModelTapped("qwen"))
        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(models.value.count, 2)

        await store.send(.alert(.presented(.deleteModel(id: "qwen"))))
        await store.receive(\.modelsLoaded)
        XCTAssertEqual(store.state.models.map(\.id), ["gemma"])
    }
}
