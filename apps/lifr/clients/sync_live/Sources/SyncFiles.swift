import CloudKit
import Foundation
import LifrSyncClient

/// Where sync keeps what it last saw: a folder in Application Support,
/// readable only by Lifr, kept out of backups so a device restored from
/// one starts sync afresh instead of passing for the old device.
struct SyncFiles: Sendable {
    let folder: URL

    /// The app's own folder: its container when it's sandboxed.
    static func onDisk() throws(SyncError) -> Self {
        do {
            let support = try FileManager.default.url(
                for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
            return try Self(folder: support.appending(path: "Sync", directoryHint: .isDirectory))
        } catch let error as SyncError {
            throw error
        } catch {
            throw .ledgerFailed(error.localizedDescription)
        }
    }

    init(folder: URL) throws(SyncError) {
        self.folder = folder
        do {
            try FileManager.default.createDirectory(
                at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            var url = folder
            try url.setResourceValues(values)
        } catch {
            throw .ledgerFailed(error.localizedDescription)
        }
    }

    var ledger: LedgerStorage {
        let url = folder.appending(path: "Ledger.json")
        return LedgerStorage(
            load: { () throws -> SyncLedger in try Self.read(SyncLedger.self, at: url) ?? SyncLedger() },
            save: { ledger in try Self.write(ledger, to: url) })
    }

    var engine: EngineFile.Storage {
        let url = folder.appending(path: "Engine.json")
        return EngineFile.Storage(
            load: { () throws(SyncError) -> EngineFile in try Self.read(EngineFile.self, at: url) ?? EngineFile() },
            save: { file throws(SyncError) in try Self.write(file, to: url) })
    }

    /// Nil when there's no file yet.
    static func read<Value: Decodable>(_ type: Value.Type, at url: URL) throws(SyncError) -> Value? {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch CocoaError.fileReadNoSuchFile {
            return nil
        } catch {
            throw .ledgerFailed(error.localizedDescription)
        }
        do {
            return try JSONDecoder().decode(type, from: data)
        } catch {
            throw .ledgerFailed(error.localizedDescription)
        }
    }

    static func write<Value: Encodable>(_ value: Value, to url: URL) throws(SyncError) {
        do {
            try JSONEncoder().encode(value).write(
                to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        } catch {
            throw .ledgerFailed(error.localizedDescription)
        }
    }
}

/// What the sync engine keeps between launches.
struct EngineFile: Codable, Sendable {
    /// This install, so the records it sent can be told from others'.
    var deviceID = UUID()
    /// CKSyncEngine's own state: what it fetched and what's left to send.
    var state: CKSyncEngine.State.Serialization?
    /// Each record's system fields from the last save or fetch, by name.
    var systemFields: [String: Data] = [:]

    struct Storage: Sendable {
        var load: @Sendable () throws(SyncError) -> EngineFile
        var save: @Sendable (EngineFile) throws(SyncError) -> Void
    }
}
