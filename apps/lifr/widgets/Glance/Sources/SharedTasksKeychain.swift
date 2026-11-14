import Foundation
import Security

/// The tasks added from the share sheet, kept in a keychain group the share
/// extension and Lifr both hold until Lifr saves them, for iPhone builds
/// with no App Group. Each task is one item, readable after the first
/// unlock and never synced or backed up off the device.
package struct SharedTasksKeychain: Sendable {
    /// The team-prefixed keychain access group.
    package let group: String

    private static let service = "lifr.shared-tasks"

    package init(group: String) {
        self.group = group
    }

    /// Leaves `task` for Lifr to save.
    package func append(_ task: SharedTask) throws(SharedTasksError) {
        let data: Data
        do {
            data = try JSONEncoder().encode(task)
        } catch {
            throw .unencodable(error)
        }
        let add: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: Self.service,
            kSecAttrAccessGroup: group,
            kSecAttrAccount: task.id.uuidString,
            kSecAttrAccessible: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
            kSecUseDataProtectionKeychain: true,
            kSecValueData: data,
        ]
        let status = SecItemAdd(add as CFDictionary, nil)
        guard status == errSecSuccess else { throw .keychainFailed(status) }
    }

    /// The tasks waiting, oldest first, leaving none. Only the items read
    /// are removed, so a task shared meanwhile waits for the next time; one
    /// that can't be decoded is removed rather than kept forever.
    package func take() throws(SharedTasksError) -> [SharedTask] {
        let search: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: Self.service,
            kSecAttrAccessGroup: group,
            kSecUseDataProtectionKeychain: true,
            kSecMatchLimit: kSecMatchLimitAll,
            kSecReturnAttributes: true,
            kSecReturnData: true,
        ]
        var result: CFTypeRef?
        let status = SecItemCopyMatching(search as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        guard status == errSecSuccess else { throw .keychainFailed(status) }
        let items = (result as? [[String: Any]]) ?? []
        for account in items.compactMap({ $0[kSecAttrAccount as String] as? String }) {
            let remove: [CFString: Any] = [
                kSecClass: kSecClassGenericPassword,
                kSecAttrService: Self.service,
                kSecAttrAccessGroup: group,
                kSecAttrAccount: account,
                kSecUseDataProtectionKeychain: true,
            ]
            let removed = SecItemDelete(remove as CFDictionary)
            guard removed == errSecSuccess || removed == errSecItemNotFound else { throw .keychainFailed(removed) }
        }
        return Self.tasks(from: items.compactMap { $0[kSecValueData as String] as? Data })
    }

    /// The tasks in `items`, oldest first, skipping any that don't decode.
    package static func tasks(from items: [Data]) -> [SharedTask] {
        let decoder = JSONDecoder()
        return items.compactMap { try? decoder.decode(SharedTask.self, from: $0) }
            .sorted { ($0.sharedAt, $0.id.uuidString) < ($1.sharedAt, $1.id.uuidString) }
    }
}

/// Where the tasks from the share sheet wait for Lifr: the App Group's
/// file when the build has one, otherwise the shared keychain group.
package enum SharedTasks: Sendable {
    case file(SharedTasksFile)
    case keychain(SharedTasksKeychain)

    /// This build's queue, or nil when it has neither an App Group nor a
    /// shared keychain group.
    package static var shared: SharedTasks? {
        if let file = SharedTasksFile.shared { return .file(file) }
        guard !WidgetGroup.sharedKeychain.isEmpty else { return nil }
        return .keychain(SharedTasksKeychain(group: WidgetGroup.sharedKeychain))
    }

    /// Leaves `task` for Lifr to save.
    package func append(_ task: SharedTask) throws(SharedTasksError) {
        switch self {
        case .file(let file): try file.append(task)
        case .keychain(let keychain): try keychain.append(task)
        }
    }

    /// The tasks waiting, oldest first, leaving none.
    package func take() throws(SharedTasksError) -> [SharedTask] {
        switch self {
        case .file(let file): try file.take()
        case .keychain(let keychain): try keychain.take()
        }
    }
}
