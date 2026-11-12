import ComposableArchitecture
import CryptoKit
import Foundation
import Security
import os

/// Why bug reports couldn't be kept or read.
package enum BugReportError: Error, Hashable, CustomStringConvertible {
    /// The Keychain wouldn't give the key that encrypts reports.
    case keyUnavailable(status: Int32, reason: String)
    /// A file or folder couldn't be read, written or removed.
    case fileSystem(operation: Operation, path: String, reason: String)
    /// A saved report couldn't be decrypted or decoded.
    case unreadable(file: String, reason: String)

    package enum Operation: Hashable, Sendable {
        case list, read, write, delete
    }

    package var description: String {
        switch self {
        case .keyUnavailable(let status, let reason):
            String(
                localized: """
                    Lifr couldn't get the key that protects bug reports from the Keychain (\(reason), \
                    status \(status)). Unlock this device and try again. Don't remove Lifr's Keychain \
                    items to fix this: reports already saved can't be read without that key.
                    """)
        case .fileSystem(.list, let path, let reason), .fileSystem(.read, let path, let reason):
            String(
                localized: """
                    Lifr couldn't read \(path): \(reason). Unlock this device and try again.
                    """)
        case .fileSystem(.write, let path, let reason):
            String(
                localized: """
                    Lifr couldn't save the report to \(path): \(reason). Free some space on this device, \
                    then save again; until then, Copy or Share keeps it.
                    """)
        case .fileSystem(.delete, let path, let reason):
            String(localized: "Lifr couldn't delete \(path): \(reason). Try again after restarting Lifr.")
        case .unreadable(let file, let reason):
            String(
                localized: """
                    The saved report \(file) can't be read (\(reason)). It may be damaged or from a \
                    newer Lifr. Update Lifr, or delete the report if it still can't be opened.
                    """)
        }
    }
}

/// The reports saved on this device.
package struct SavedReports: Equatable, Sendable {
    /// Newest first.
    package var reports: [BugReport]
    /// Reports that are there but can't be read, so they can be deleted.
    package var unreadable: [Unreadable]

    package init(reports: [BugReport] = [], unreadable: [Unreadable] = []) {
        self.reports = reports
        self.unreadable = unreadable
    }

    package struct Unreadable: Identifiable, Hashable, Sendable {
        package var id: UUID
        package var error: BugReportError

        package init(id: UUID, error: BugReportError) {
            self.id = id
            self.error = error
        }
    }
}

/// Keeps bug reports on this device. Every call throws `BugReportError`.
@DependencyClient
package struct BugReportsClient: Sendable {
    package var load: @Sendable () async throws -> SavedReports
    package var save: @Sendable (BugReport) async throws -> Void
    package var delete: @Sendable (UUID) async throws -> Void
}

extension BugReportsClient {
    /// Kept in memory, for tests and previews.
    package static func inMemory(_ reports: [BugReport] = []) -> Self {
        let stored = OSAllocatedUnfairLock(initialState: reports)
        return BugReportsClient(
            load: { SavedReports(reports: stored.withLock { $0 }.sorted { $0.createdAt > $1.createdAt }) },
            save: { report in
                stored.withLock { reports in
                    reports.removeAll { $0.id == report.id }
                    reports.append(report)
                }
            },
            delete: { id in stored.withLock { $0.removeAll { $0.id == id } } })
    }

    /// One file per report in `folder`, encrypted with `key`. The folder
    /// and files are readable only by Lifr, kept out of backups and, on the
    /// iPhone, unreadable while it's locked.
    package static func onDisk(
        folder: URL, key: @escaping @Sendable () throws(BugReportError) -> SymmetricKey
    ) -> Self {
        let files = ReportFiles(folder: folder, key: key)
        return BugReportsClient(
            load: { try files.load() },
            save: { try files.save($0) },
            delete: { try files.delete($0) })
    }
}

extension BugReportsClient: DependencyKey {
    package static var liveValue: BugReportsClient {
        let support = URL.applicationSupportDirectory
        return .onDisk(folder: support.appending(path: "BugReports", directoryHint: .isDirectory)) {
            () throws(BugReportError) in try ReportKey.load()
        }
    }
    package static let testValue = BugReportsClient()
    package static let previewValue = BugReportsClient.inMemory()
}

extension DependencyValues {
    package var bugReports: BugReportsClient {
        get { self[BugReportsClient.self] }
        set { self[BugReportsClient.self] = newValue }
    }
}

/// The reports folder.
struct ReportFiles: Sendable {
    let folder: URL
    let key: @Sendable () throws(BugReportError) -> SymmetricKey

    private static let suffix = "lifrbug"

    func load() throws(BugReportError) -> SavedReports {
        let names: [String]
        do {
            names = try FileManager.default.contentsOfDirectory(atPath: folder.path(percentEncoded: false))
        } catch CocoaError.fileReadNoSuchFile {
            return SavedReports()
        } catch {
            throw .fileSystem(operation: .list, path: folder.lastPathComponent, reason: error.localizedDescription)
        }
        let key = try key()
        var saved = SavedReports()
        for name in names where name.hasSuffix(".\(Self.suffix)") {
            guard let id = UUID(uuidString: String(name.dropLast(Self.suffix.count + 1))) else { continue }
            do {
                saved.reports.append(try read(url(for: id), key: key))
            } catch .unreadable(let file, let reason) {
                saved.unreadable.append(.init(id: id, error: .unreadable(file: file, reason: reason)))
            }
        }
        saved.reports.sort { $0.createdAt > $1.createdAt }
        return saved
    }

    func save(_ report: BugReport) throws(BugReportError) {
        let url = url(for: report.id)
        let sealed: Data
        do {
            let data = try JSONEncoder().encode(report)
            guard let combined = try AES.GCM.seal(data, using: try key()).combined else {
                throw BugReportError.fileSystem(
                    operation: .write, path: url.lastPathComponent, reason: "it couldn't be encrypted")
            }
            sealed = combined
        } catch let error as BugReportError {
            throw error
        } catch {
            throw .fileSystem(operation: .write, path: url.lastPathComponent, reason: error.localizedDescription)
        }
        do {
            try FileManager.default.createDirectory(
                at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            var excluded = folder
            try excluded.setResourceValues(values)
            #if os(iOS)
                // Unreadable while the iPhone is locked. The Mac has no
                // such class, and writing with it fails there.
                try sealed.write(to: url, options: [.atomic, .completeFileProtection])
            #else
                try sealed.write(to: url, options: .atomic)
            #endif
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o600], ofItemAtPath: url.path(percentEncoded: false))
        } catch {
            throw .fileSystem(operation: .write, path: url.lastPathComponent, reason: error.localizedDescription)
        }
    }

    func delete(_ id: UUID) throws(BugReportError) {
        let url = url(for: id)
        do {
            try FileManager.default.removeItem(at: url)
        } catch CocoaError.fileNoSuchFile {
            return
        } catch {
            throw .fileSystem(operation: .delete, path: url.lastPathComponent, reason: error.localizedDescription)
        }
    }

    private func url(for id: UUID) -> URL {
        folder.appending(path: "\(id.uuidString).\(Self.suffix)", directoryHint: .notDirectory)
    }

    private func read(_ url: URL, key: SymmetricKey) throws(BugReportError) -> BugReport {
        let data: Data
        do {
            data = try Data(contentsOf: url)
        } catch {
            throw .unreadable(file: url.lastPathComponent, reason: error.localizedDescription)
        }
        do {
            let opened = try AES.GCM.open(AES.GCM.SealedBox(combined: data), using: key)
            return try JSONDecoder().decode(BugReport.self, from: opened)
        } catch {
            throw .unreadable(file: url.lastPathComponent, reason: String(describing: error))
        }
    }
}

/// The key that encrypts bug reports: made on first use and kept in this
/// device's Keychain only, never synced or restored to another device.
enum ReportKey {
    private static let query: [CFString: Any] = [
        kSecClass: kSecClassGenericPassword,
        kSecAttrService: "com.avestabarzegar.lifr.bug-reports",
        kSecAttrAccount: "key",
    ]

    static func load() throws(BugReportError) -> SymmetricKey {
        if let existing = try find() { return existing }
        let key = SymmetricKey(size: .bits256)
        var add = query
        add[kSecValueData] = key.withUnsafeBytes { Data($0) }
        #if os(iOS)
            add[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        #endif
        let status = SecItemAdd(add as CFDictionary, nil)
        switch status {
        case errSecSuccess: return key
        case errSecDuplicateItem:
            // Made at the same time by another save: use that one.
            guard let existing = try find() else { throw failure(status) }
            return existing
        default: throw failure(status)
        }
    }

    private static func find() throws(BugReportError) -> SymmetricKey? {
        var search = query
        search[kSecReturnData] = true
        search[kSecMatchLimit] = kSecMatchLimitOne
        var item: CFTypeRef?
        let status = SecItemCopyMatching(search as CFDictionary, &item)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess, let data = item as? Data else { throw failure(status) }
        return SymmetricKey(data: data)
    }

    private static func failure(_ status: OSStatus) -> BugReportError {
        .keyUnavailable(
            status: status, reason: (SecCopyErrorMessageString(status, nil) as String?) ?? "unknown error")
    }
}

/// Its description is written for the person, so alerts show it.
extension BugReportError: LocalizedError {
    package var errorDescription: String? { description }
}
