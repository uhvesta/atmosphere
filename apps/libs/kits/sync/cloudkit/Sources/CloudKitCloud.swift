import CloudKit
import Foundation
import OSLog
import SyncKit

/// The cloud through a CKSyncEngine in the private database. Syncing runs
/// when the Syncer asks, not on the engine's own schedule, so every change
/// that arrives goes through the merge.
actor CloudKitCloud<Snapshot: SyncSnapshot>: CKSyncEngineDelegate {
    private let container: CKContainer
    private let device: String
    private let storage: EngineFile.Storage
    private let records: CloudRecord<Snapshot>
    private let logger: Logger
    private var file: EngineFile?
    private var syncEngine: CKSyncEngine?
    private var isZoneReady = false

    /// What the current fetch received, by name, latest last.
    private var fetched: [String: CloudChange<Snapshot>] = [:]
    private var fetchOrder: [String] = []
    /// What the current send is saving, by name.
    private var sending: [String: CloudItem<Snapshot>] = [:]
    /// What the current send had refused, with the server's version.
    private var refused: [CloudChange<Snapshot>] = []
    /// Records to send again, such as one the server no longer has.
    private var retry: Set<String> = []
    private var sendFailure: SyncError?

    /// Syncs through `zone` in the private database of `container`.
    /// `subsystem` names the app in the log.
    init(container: CKContainer, zone: String, device: String, storage: EngineFile.Storage, subsystem: String) {
        self.container = container
        self.device = device
        self.storage = storage
        records = CloudRecord(zone: zone)
        logger = Logger(subsystem: subsystem, category: "sync")
    }

    nonisolated var cloud: Cloud<Snapshot> {
        Cloud(
            fetch: { try await self.fetch() },
            send: { items in try await self.send(items) })
    }

    // MARK: Cloud

    private func fetch() async throws(SyncError) -> [CloudChange<Snapshot>] {
        let engine = try await ready()
        fetched = [:]
        fetchOrder = []
        do {
            try await engine.fetchChanges()
        } catch {
            throw SyncError(error)
        }
        let changes = fetchOrder.compactMap { fetched[$0] }
        fetched = [:]
        fetchOrder = []
        return changes
    }

    private func send(_ items: [CloudItem<Snapshot>]) async throws(SyncError) -> [CloudChange<Snapshot>] {
        guard !items.isEmpty else { return [] }
        let engine = try await ready()
        sending = Dictionary(items.map { ($0.name, $0) }, uniquingKeysWith: { $1 })
        refused = []
        sendFailure = nil
        defer {
            sending = [:]
            refused = []
            retry = []
        }
        var names = Set(sending.keys)
        // A second round for records the server lost, or a zone it lost.
        for _ in 0..<2 where !names.isEmpty {
            retry = []
            engine.state.add(pendingRecordZoneChanges: names.map { .saveRecord(records.id($0)) })
            do {
                try await engine.sendChanges()
            } catch {
                throw SyncError(error)
            }
            if let sendFailure { throw sendFailure }
            names = retry
            if !names.isEmpty { try await ensureZone() }
        }
        guard names.isEmpty else { throw .refused(String(localized: "iCloud lost the records twice")) }
        return refused
    }

    // MARK: CKSyncEngineDelegate

    func handleEvent(_ event: CKSyncEngine.Event, syncEngine: CKSyncEngine) async {
        switch event {
        case .stateUpdate(let stateUpdate):
            update(\.state, to: stateUpdate.stateSerialization)

        case .accountChange(let change):
            switch change.changeType {
            case .signIn: break
            case .signOut, .switchAccounts:
                // Another account's records have other change tags.
                update(\.systemFields, to: [:])
                isZoneReady = false
            @unknown default: break
            }

        case .fetchedRecordZoneChanges(let changes):
            for modification in changes.modifications {
                let record = modification.record
                let name = record.recordID.recordName
                updateSystemFields(of: record)
                do {
                    let (change, deviceID) = try records.change(record)
                    guard deviceID != file?.deviceID else { continue }
                    if fetched[name] == nil { fetchOrder.append(name) }
                    fetched[name] = change
                } catch {
                    logger.error("\(error.description, privacy: .public)")
                }
            }
            for deletion in changes.deletions {
                // Sync never deletes a record, only sends it as removed.
                update(\.systemFields[deletion.recordID.recordName], to: nil)
            }

        case .fetchedDatabaseChanges(let changes):
            if changes.deletions.contains(where: { $0.zoneID == records.zone }) {
                // The person deleted the app's iCloud data in Settings.
                // What's on this device is sent again on the next sync.
                logger.notice("The app's iCloud zone was deleted")
                update(\.systemFields, to: [:])
                isZoneReady = false
            }

        case .sentRecordZoneChanges(let sent):
            for record in sent.savedRecords { updateSystemFields(of: record) }
            for failure in sent.failedRecordSaves {
                let name = failure.record.recordID.recordName
                switch failure.error.code {
                case .serverRecordChanged:
                    guard let server = failure.error.serverRecord else {
                        sendFailure = SyncError(failure.error)
                        continue
                    }
                    updateSystemFields(of: server)
                    do {
                        refused.append(try records.change(server).change)
                    } catch {
                        logger.error("\(error.description, privacy: .public)")
                        sendFailure = .refused(error.description)
                    }
                case .unknownItem:
                    update(\.systemFields[name], to: nil)
                    retry.insert(name)
                case .zoneNotFound, .userDeletedZone:
                    update(\.systemFields, to: [:])
                    isZoneReady = false
                    retry.insert(name)
                default:
                    sendFailure = SyncError(failure.error)
                }
            }

        case .sentDatabaseChanges, .willFetchChanges, .willFetchRecordZoneChanges, .didFetchRecordZoneChanges,
            .didFetchChanges, .willSendChanges, .didSendChanges:
            break

        @unknown default:
            break
        }
    }

    func nextRecordZoneChangeBatch(
        _ context: CKSyncEngine.SendChangesContext, syncEngine: CKSyncEngine
    ) async -> CKSyncEngine.RecordZoneChangeBatch? {
        let changes = syncEngine.state.pendingRecordZoneChanges.filter { context.options.scope.contains($0) }
        return await CKSyncEngine.RecordZoneChangeBatch(pendingChanges: changes) { id in
            await self.record(named: id.recordName)
        }
    }

    /// The record to save for `name`, or nil to drop it: something left
    /// pending from an earlier launch that this send doesn't have.
    private func record(named name: String) -> CKRecord? {
        guard let item = sending[name], let file else { return nil }
        do {
            return try records.record(
                item, device: device, deviceID: file.deviceID, systemFields: file.systemFields[name])
        } catch {
            logger.error("\(error.description, privacy: .public)")
            sendFailure = .refused(error.description)
            return nil
        }
    }

    // MARK: Setup

    /// The engine, once the account and zone are there.
    private func ready() async throws(SyncError) -> CKSyncEngine {
        let status: CKAccountStatus
        do {
            status = try await container.accountStatus()
        } catch {
            throw SyncError(error)
        }
        switch status {
        case .available: break
        case .noAccount, .restricted: throw .noAccount
        case .couldNotDetermine, .temporarilyUnavailable:
            throw .unreachable(String(localized: "iCloud isn't available right now"))
        @unknown default: throw .noAccount
        }
        let engine = try engine()
        if !isZoneReady { try await ensureZone() }
        return engine
    }

    private func engine() throws(SyncError) -> CKSyncEngine {
        if let syncEngine { return syncEngine }
        let file = try storage.load()
        self.file = file
        var configuration = CKSyncEngine.Configuration(
            database: container.privateCloudDatabase, stateSerialization: file.state, delegate: self)
        configuration.automaticallySync = false
        let engine = CKSyncEngine(configuration)
        syncEngine = engine
        return engine
    }

    private func ensureZone() async throws(SyncError) {
        do {
            _ = try await container.privateCloudDatabase.modifyRecordZones(
                saving: [CKRecordZone(zoneID: records.zone)], deleting: [])
            isZoneReady = true
        } catch {
            throw SyncError(error)
        }
        await subscribe()
    }

    /// Asks iCloud for a silent push when another device changes the
    /// database, so an app with the push entitlement can sync then. Saving
    /// it again only replaces it. Without the entitlement, or offline, the
    /// app syncs on its own schedule instead, so a failure is only logged.
    private func subscribe() async {
        let subscription = CKDatabaseSubscription(subscriptionID: records.zone.zoneName + ".changes")
        let notification = CKSubscription.NotificationInfo()
        notification.shouldSendContentAvailable = true
        subscription.notificationInfo = notification
        do {
            _ = try await container.privateCloudDatabase.modifySubscriptions(saving: [subscription], deleting: [])
        } catch {
            logger.notice("Couldn't ask iCloud for pushes: \(error.localizedDescription, privacy: .public)")
        }
    }

    private func updateSystemFields(of record: CKRecord) {
        update(\.systemFields[record.recordID.recordName], to: CloudRecord<Snapshot>.systemFields(of: record))
    }

    private func update<Value>(_ path: WritableKeyPath<EngineFile, Value>, to value: Value) {
        guard var file else { return }
        file[keyPath: path] = value
        self.file = file
        do {
            try storage.save(file)
        } catch {
            logger.error("Saving sync's state failed: \(error.description, privacy: .public)")
        }
    }
}
