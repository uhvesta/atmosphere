import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrSyncClient
import XCTest

/// Two devices that share an in-memory cloud.
final class SyncerTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private struct Device {
        let store: StoreClient
        let syncer: Syncer
        let storage: LedgerStorage

        func task(_ id: UUID) async throws -> LifrTask? { try await store.load().tasks[id] }
    }

    private func device(_ name: String, _ workspace: Workspace, cloud: InMemoryCloud, storage: LedgerStorage? = nil)
        -> Device
    {
        let store = StoreClient.inMemory(workspace, actor: .user(device: name, surface: .iPhone))
        let storage = storage ?? .inMemory()
        let syncer = Syncer(cloud: cloud.connection(device: name), store: store, storage: storage) { [now] in now }
        return Device(store: store, syncer: syncer, storage: storage)
    }

    private func run(_ body: () async throws -> Void) async throws {
        try await withDependencies { [now] in
            $0.date.now = now
            $0.uuid = UUIDGenerator { UUID() }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        } operation: {
            try await body()
        }
    }

    /// Both devices with a HEALTH Avenue and a "Buy socks" task, synced.
    private func pair() async throws -> (phone: Device, mac: Device, cloud: InMemoryCloud, socks: UUID) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let socks = records.task("Buy socks", in: health)
        let cloud = InMemoryCloud()
        let phone = device("iPhone", records.workspace, cloud: cloud)
        let mac = device("Mac", records.workspace, cloud: cloud)
        await phone.syncer.sync()
        await mac.syncer.sync()
        return (phone, mac, cloud, socks.id)
    }

    func testATaskAddedOnOneDeviceArrivesOnTheOtherWithItsJournalEntry() async throws {
        try await run {
            let (phone, mac, _, _) = try await pair()
            let health = try await unwrap({ try await phone.store.load().avenues.values.first })
            let saved = try await phone.store.createTask(Workspace.NewTask(title: "Stretch", avenueID: health.id))
            let stretch = try XCTUnwrap(saved.workspace.tasks.values.first { $0.title == "Stretch" })

            await phone.syncer.sync()
            let state = await mac.syncer.sync()

            XCTAssertEqual(state.status, .idle)
            try await assertEqual({ try await mac.task(stretch.id) }, stretch)
            let journal = try await mac.store.load().journal.map(\.id)
            XCTAssertEqual(journal, saved.workspace.journal.map(\.id))
        }
    }

    func testDifferentFieldsChangedOnEachDeviceBothStand() async throws {
        try await run {
            let (phone, mac, _, socks) = try await pair()
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy wool socks"))
            _ = try await mac.store.edit(socks, Workspace.TaskEdit(priority: .high))

            await phone.syncer.sync()
            let state = await mac.syncer.sync()
            await phone.syncer.sync()

            XCTAssertEqual(state.review, [])
            for device in [phone, mac] {
                let task = try await unwrap({ try await device.task(socks) })
                XCTAssertEqual(task.title, "Buy wool socks")
                XCTAssertEqual(task.priority, .high)
            }
        }
    }

    func testAFieldBothChangedIsHeldForReviewAndResolvedAsTheyChose() async throws {
        try await run {
            let (phone, mac, _, socks) = try await pair()
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy wool socks", priority: .high))
            _ = try await mac.store.edit(socks, Workspace.TaskEdit(title: "Buy cotton socks"))

            await phone.syncer.sync()
            let state = await mac.syncer.sync()

            let item = try XCTUnwrap(state.review.first)
            XCTAssertEqual(state.review.count, 1)
            XCTAssertEqual(item.device, "iPhone")
            XCTAssertEqual(item.conflict.kind, .fields([.title]))
            // The Mac keeps its title, takes the priority, and sends nothing.
            try await assertEqual({ try await mac.task(socks)?.title }, "Buy cotton socks")
            try await assertEqual({ try await mac.task(socks)?.priority }, .high)
            await phone.syncer.sync()
            try await assertEqual({ try await phone.task(socks)?.title }, "Buy wool socks")

            let entries = try await mac.store.load().journal.count
            let resolved = await mac.syncer.resolve([item.id: [.title: .theirs]])
            await phone.syncer.sync()

            XCTAssertEqual(resolved.review, [])
            try await assertEqual({ try await mac.store.load().journal.count }, entries + 1)
            for device in [phone, mac] {
                try await assertEqual({ try await device.task(socks)?.title }, "Buy wool socks")
            }
        }
    }

    func testChoosingThisDeviceSendsItsValueToTheOther() async throws {
        try await run {
            let (phone, mac, _, socks) = try await pair()
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy wool socks"))
            _ = try await mac.store.edit(socks, Workspace.TaskEdit(title: "Buy cotton socks"))
            await phone.syncer.sync()
            let state = await mac.syncer.sync()
            let item = try XCTUnwrap(state.review.first)

            await mac.syncer.resolve([item.id: [.title: .mine]])
            await phone.syncer.sync()

            try await assertEqual({ try await phone.task(socks)?.title }, "Buy cotton socks")
        }
    }

    func testAReviewPutOffShowsTheOtherDevicesLatestChangeNextTime() async throws {
        try await run {
            let (phone, mac, _, socks) = try await pair()
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy wool socks"))
            _ = try await mac.store.edit(socks, Workspace.TaskEdit(title: "Buy cotton socks"))
            await phone.syncer.sync()
            await mac.syncer.sync()

            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy silk socks"))
            await phone.syncer.sync()
            let state = await mac.syncer.sync()

            let item = try XCTUnwrap(state.review.first)
            XCTAssertEqual(state.review.count, 1)
            guard case .task(let theirs)? = item.conflict.theirs else { return XCTFail("expected a task") }
            XCTAssertEqual(theirs.title, "Buy silk socks")
        }
    }

    func testAHeldRecordTheOtherDeviceChangesBackMergesWithoutReview() async throws {
        try await run {
            let (phone, mac, _, socks) = try await pair()
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy wool socks"))
            _ = try await mac.store.edit(socks, Workspace.TaskEdit(title: "Buy cotton socks"))
            await phone.syncer.sync()
            await mac.syncer.sync()

            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy socks"))
            await phone.syncer.sync()
            let state = await mac.syncer.sync()
            await phone.syncer.sync()

            XCTAssertEqual(state.review, [])
            try await assertEqual({ try await phone.task(socks)?.title }, "Buy cotton socks")
        }
    }

    func testAnEditAgainstADeleteIsReviewedAndKeepingItRestoresIt() async throws {
        try await run {
            let (phone, mac, _, socks) = try await pair()
            _ = try await phone.store.save { (workspace, context) throws(LifrError) in
                try workspace.moveToTrash(socks, in: context)
            }
            _ = try await mac.store.edit(socks, Workspace.TaskEdit(priority: .high))
            await phone.syncer.sync()
            let state = await mac.syncer.sync()

            let item = try XCTUnwrap(state.review.first)
            XCTAssertEqual(item.conflict.kind, .removed(by: .theirs))
            try await assertNil({ try await mac.task(socks)?.deletedAt })

            await mac.syncer.resolve([item.id: [.deletedAt: .mine]])
            await phone.syncer.sync()

            try await assertNil({ try await phone.task(socks)?.deletedAt })
            try await assertEqual({ try await phone.task(socks)?.priority }, .high)
        }
    }

    func testAnEditAgainstADeleteCanBeDeleted() async throws {
        try await run {
            let (phone, mac, _, socks) = try await pair()
            _ = try await phone.store.save { (workspace, context) throws(LifrError) in
                try workspace.moveToTrash(socks, in: context)
            }
            _ = try await mac.store.edit(socks, Workspace.TaskEdit(priority: .high))
            await phone.syncer.sync()
            let state = await mac.syncer.sync()
            let item = try XCTUnwrap(state.review.first)

            await mac.syncer.resolve([item.id: [.deletedAt: .theirs]])

            try await assertNotNil({ try await mac.task(socks)?.deletedAt })
        }
    }

    func testWhenTheCloudCantBeReachedTheChangesWaitForTheNextSync() async throws {
        try await run {
            let (phone, mac, cloud, socks) = try await pair()
            cloud.setOnline(false, device: "iPhone")
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy wool socks"))

            let failed = await phone.syncer.sync()
            guard case .failed(.unreachable) = failed.status else { return XCTFail("got \(failed.status)") }
            cloud.setOnline(true, device: "iPhone")
            let state = await phone.syncer.sync()
            await mac.syncer.sync()

            XCTAssertEqual(state.status, .idle)
            try await assertEqual({ try await mac.task(socks)?.title }, "Buy wool socks")
        }
    }

    func testChangesThatCantBeSavedHereArriveOnTheNextSync() async throws {
        try await run {
            let (phone, mac, cloud, socks) = try await pair()
            struct DiskFull: Error {}
            let full = LockIsolated(true)
            var store = mac.store
            let importSynced = store.importSynced
            store.importSynced = { changes in
                if full.value { throw DiskFull() }
                return try await importSynced(changes)
            }
            let clock = { [now] in now }
            let failing = Syncer(
                cloud: cloud.connection(device: "Mac"), store: store, storage: mac.storage, now: clock)
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy wool socks"))
            await phone.syncer.sync()

            let failed = await failing.sync()
            guard case .failed = failed.status else { return XCTFail("got \(failed.status)") }
            try await assertEqual({ try await mac.task(socks)?.title }, "Buy socks")

            full.setValue(false)
            let state = await failing.sync()
            XCTAssertEqual(state.status, .idle)
            try await assertEqual({ try await mac.task(socks)?.title }, "Buy wool socks")
        }
    }

    func testAnEditSavedWhileSyncMergesIsKeptAlongsideTheOtherDevicesChange() async throws {
        try await run {
            let (phone, mac, cloud, socks) = try await pair()
            var store = mac.store
            let importSynced = store.importSynced
            let edits = mac.store
            store.importSynced = { changes in
                // The person renames the task just before the import lands.
                _ = try await edits.edit(socks, Workspace.TaskEdit(title: "Buy wool socks"))
                return try await importSynced(changes)
            }
            let editing = Syncer(cloud: cloud.connection(device: "Mac"), store: store, storage: mac.storage)
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(priority: .high))
            await phone.syncer.sync()

            let state = await editing.sync()

            XCTAssertEqual(state.status, .idle)
            try await assertEqual({ try await mac.task(socks)?.title }, "Buy wool socks")
            try await assertEqual({ try await mac.task(socks)?.priority }, .high)
            await phone.syncer.sync()
            try await assertEqual({ try await phone.task(socks)?.title }, "Buy wool socks")
        }
    }

    func testTheLedgerCarriesHeldRecordsAcrossLaunches() async throws {
        try await run {
            let (phone, mac, cloud, socks) = try await pair()
            _ = try await phone.store.edit(socks, Workspace.TaskEdit(title: "Buy wool socks"))
            _ = try await mac.store.edit(socks, Workspace.TaskEdit(title: "Buy cotton socks"))
            await phone.syncer.sync()
            await mac.syncer.sync()

            let relaunched = Syncer(cloud: cloud.connection(device: "Mac"), store: mac.store, storage: mac.storage)
            let state = await relaunched.sync()

            XCTAssertEqual(state.review.map(\.id), [RecordID(.task, socks)])
        }
    }

    func testReconcilingAfterAMergeIsSentBack() async throws {
        try await run {
            let (phone, mac, _, socks) = try await pair()
            let parent = try await phone.store.createTask(
                Workspace.NewTask(
                    title: "Errands", avenueID: try await unwrap({ try await phone.task(socks) }).avenueID))
            let errands = try XCTUnwrap(parent.workspace.tasks.values.first { $0.title == "Errands" })
            await phone.syncer.sync()
            await mac.syncer.sync()
            _ = try await phone.store.setStatus(.done, errands.id)
            _ = try await mac.store.save { (workspace, context) throws(LifrError) in
                try workspace.createTask(
                    Workspace.NewTask(title: "Socks too", avenueID: errands.avenueID, parentID: errands.id), in: context
                )
            }

            await phone.syncer.sync()
            await mac.syncer.sync()
            await phone.syncer.sync()

            try await assertEqual({ try await phone.task(errands.id)?.status }, .inProgress)
            try await assertEqual({ try await mac.task(errands.id)?.status }, .inProgress)
        }
    }

    func testALedgerRoundTripsThroughJSON() async throws {
        try await run {
            let (_, mac, _, _) = try await pair()
            let ledger = try mac.storage.load()

            let decoded = try JSONDecoder().decode(SyncLedger.self, from: JSONEncoder().encode(ledger))

            XCTAssertEqual(decoded, ledger)
        }
    }
}

private func assertEqual<T: Equatable>(
    _ value: () async throws -> T, _ expected: T, file: StaticString = #filePath, line: UInt = #line
) async throws {
    let actual = try await value()
    XCTAssertEqual(actual, expected, file: file, line: line)
}

private func assertNil<T>(_ value: () async throws -> T?, file: StaticString = #filePath, line: UInt = #line)
    async throws
{
    let actual = try await value()
    XCTAssertNil(actual, file: file, line: line)
}

private func assertNotNil<T>(_ value: () async throws -> T?, file: StaticString = #filePath, line: UInt = #line)
    async throws
{
    let actual = try await value()
    XCTAssertNotNil(actual, file: file, line: line)
}

private func unwrap<T>(_ value: () async throws -> T?, file: StaticString = #filePath, line: UInt = #line)
    async throws -> T
{
    let actual = try await value()
    return try XCTUnwrap(actual, file: file, line: line)
}
