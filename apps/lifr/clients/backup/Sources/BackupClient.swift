import ComposableArchitecture
import Foundation
import LifrCore

/// One day's automatic backup.
package struct Backup: Identifiable, Hashable, Sendable {
    /// The day it's of.
    package var day: CalendarDate
    package var size: Int

    package var id: CalendarDate { day }

    package init(day: CalendarDate, size: Int) {
        self.day = day
        self.size = size
    }
}

/// Why a backup couldn't be written or read.
package enum BackupError: Error, Hashable, Sendable, CustomStringConvertible {
    case unwritable(String)
    case unreadable(day: CalendarDate, String)

    package var description: String {
        switch self {
        case .unwritable(let reason):
            String(
                localized: """
                    Today's backup couldn't be written (\(reason)). Free some space on this device; \
                    Lifr tries again when it next opens.
                    """)
        case .unreadable(let day, let reason):
            String(
                localized: """
                    The backup of \(day.description) can't be read (\(reason)). Pick another day's backup, \
                    or import a file you exported.
                    """)
        }
    }
}

/// The automatic local backups: one a day, the last `kept` days.
package struct BackupClient: Sendable {
    /// Writes today's backup, replacing one written earlier today, and
    /// deletes those older than the last `kept` days.
    package var save: @Sendable (_ export: LifrExport, _ day: CalendarDate) async throws(BackupError) -> Void
    /// The backups kept, newest first.
    package var list: @Sendable () async -> [Backup]
    package var read: @Sendable (_ day: CalendarDate) async throws(BackupError) -> Data

    package init(
        save: @escaping @Sendable (_ export: LifrExport, _ day: CalendarDate) async throws(BackupError) -> Void,
        list: @escaping @Sendable () async -> [Backup],
        read: @escaping @Sendable (_ day: CalendarDate) async throws(BackupError) -> Data
    ) {
        self.save = save
        self.list = list
        self.read = read
    }

    /// How many days are kept.
    package static let kept = 7
}

extension BackupClient: DependencyKey {
    /// In `Backups` in Application Support, readable only by Lifr,
    /// encrypted while the device is locked after a restart, and left out
    /// of the device's own backups: the records are in iCloud already.
    package static var liveValue: BackupClient {
        let folder: @Sendable () -> URL? = {
            try? FileManager.default.url(
                for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
            ).appending(path: "Backups", directoryHint: .isDirectory)
        }
        return .inFolder(folder)
    }

    /// Backups in the folder `folder` gives.
    package static func inFolder(_ folder: @escaping @Sendable () -> URL?) -> BackupClient {
        BackupClient(
            save: { export, day throws(BackupError) in
                guard let folder = folder() else { throw .unwritable("no Application Support folder") }
                let data: Data
                do throws(ImportError) {
                    data = try export.encoded()
                } catch {
                    throw .unwritable(error.description)
                }
                do {
                    try FileManager.default.createDirectory(
                        at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
                    var values = URLResourceValues()
                    values.isExcludedFromBackup = true
                    var excluded = folder
                    try excluded.setResourceValues(values)
                    let url = folder.appending(path: fileName(of: day))
                    try data.write(to: url, options: [.atomic, .completeFileProtectionUntilFirstUserAuthentication])
                    try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
                    for old in backups(in: folder).dropFirst(kept) {
                        try FileManager.default.removeItem(at: folder.appending(path: fileName(of: old.day)))
                    }
                } catch {
                    throw .unwritable(error.localizedDescription)
                }
            },
            list: { folder().map(backups(in:)) ?? [] },
            read: { day throws(BackupError) in
                guard let folder = folder() else { throw .unreadable(day: day, "no Application Support folder") }
                do {
                    return try Data(contentsOf: folder.appending(path: fileName(of: day)))
                } catch {
                    throw .unreadable(day: day, error.localizedDescription)
                }
            })
    }

    private static func fileName(of day: CalendarDate) -> String { "Lifr-\(day.description).json" }

    /// The backups in `folder`, newest first.
    private static func backups(in folder: URL) -> [Backup] {
        let manager = FileManager.default
        let urls = (try? manager.contentsOfDirectory(at: folder, includingPropertiesForKeys: [.fileSizeKey])) ?? []
        return urls.compactMap { url -> Backup? in
            let name = url.deletingPathExtension().lastPathComponent
            guard name.hasPrefix("Lifr-"), url.pathExtension == "json",
                let day = try? CalendarDate(String(name.dropFirst(5)))
            else { return nil }
            let size = (try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0
            return Backup(day: day, size: size)
        }
        .sorted { $0.day > $1.day }
    }

    /// No backups, for tests and previews; tests of them use `inMemory`.
    package static let testValue = BackupClient(
        save: { _, _ in },
        list: { [] },
        read: { day throws(BackupError) in throw .unreadable(day: day, "no backups in tests") })
    package static let previewValue = testValue

    /// Backups kept in memory, for tests.
    package static func inMemory() -> (client: BackupClient, saved: LockIsolated<[CalendarDate: Data]>) {
        let saved = LockIsolated<[CalendarDate: Data]>([:])
        let client = BackupClient(
            save: { export, day throws(BackupError) in
                let data: Data
                do throws(ImportError) {
                    data = try export.encoded()
                } catch {
                    throw .unwritable(error.description)
                }
                saved.withValue { backups in
                    backups[day] = data
                    for old in backups.keys.sorted(by: >).dropFirst(kept) { backups[old] = nil }
                }
            },
            list: { saved.value.map { Backup(day: $0.key, size: $0.value.count) }.sorted { $0.day > $1.day } },
            read: { day throws(BackupError) in
                guard let data = saved.value[day] else { throw .unreadable(day: day, "not kept") }
                return data
            })
        return (client, saved)
    }
}

extension DependencyValues {
    package var backups: BackupClient {
        get { self[BackupClient.self] }
        set { self[BackupClient.self] = newValue }
    }
}
