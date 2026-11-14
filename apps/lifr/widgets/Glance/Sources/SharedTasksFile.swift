import Foundation
import Security

/// A task added from the share sheet, waiting for Lifr to save it. The
/// share extension can't open the records, so it leaves these in the App
/// Group and Lifr saves them when it next opens or comes to the front.
package struct SharedTask: Codable, Hashable, Sendable, Identifiable {
    package var id: UUID
    package var title: String
    /// The link or text that was shared, for the task's description.
    package var details: String
    /// The Avenue picked, or nil for the Inbox.
    package var avenueID: UUID?
    /// The day it's due, as YYYY-MM-DD, or nil for none.
    package var due: String?
    package var sharedAt: Date

    package init(id: UUID, title: String, details: String, avenueID: UUID?, due: String?, sharedAt: Date) {
        self.id = id
        self.title = title
        self.details = details
        self.avenueID = avenueID
        self.due = due
        self.sharedAt = sharedAt
    }
}

/// Why the tasks from the share sheet couldn't be read or kept.
package enum SharedTasksError: Error, CustomStringConvertible {
    /// This build has no App Group, so there's nowhere to leave the task.
    case noAppGroup
    /// The file is there but couldn't be read or decoded.
    case readFailed(URL, any Error)
    /// The file couldn't be written or removed.
    case writeFailed(URL, any Error)
    /// The task couldn't be made into data to keep.
    case unencodable(any Error)
    /// The keychain refused to keep, read or remove a task.
    case keychainFailed(OSStatus)

    package var description: String {
        switch self {
        case .noAppGroup:
            String(
                localized: """
                    This build of Lifr can't take tasks from the share sheet. Copy the text and paste it into \
                    Quick Add instead.
                    """)
        case .readFailed(let url, let error):
            String(
                localized: """
                    Lifr couldn't read the tasks added from the share sheet at \(url.path) \
                    (\(error.localizedDescription)). Share them again to add them.
                    """)
        case .writeFailed(let url, let error):
            String(
                localized: """
                    Lifr couldn't keep the task to add at \(url.path) (\(error.localizedDescription)). \
                    Open Lifr and add it with Quick Add.
                    """)
        case .unencodable(let error):
            String(
                localized: """
                    Lifr couldn't keep the task to add (\(error.localizedDescription)). Open Lifr and add it \
                    with Quick Add.
                    """)
        case .keychainFailed(let status):
            String(
                localized: """
                    Lifr couldn't reach the tasks added from the share sheet in the keychain \
                    (\(Self.message(status))). Unlock the device and try again; if it keeps happening, add the \
                    task with Quick Add.
                    """)
        }
    }
}

extension SharedTasksError {
    /// What the keychain says `status` means, or its number.
    fileprivate static func message(_ status: OSStatus) -> String {
        (SecCopyErrorMessageString(status, nil) as String?) ?? "\(status)"
    }
}

/// The tasks added from the share sheet, kept in the App Group until Lifr
/// saves them.
package struct SharedTasksFile: Sendable {
    package let url: URL

    package init(url: URL) {
        self.url = url
    }

    /// The file in the App Group's container, or nil when this build has
    /// no App Group.
    package static var shared: SharedTasksFile? {
        guard !WidgetGroup.id.isEmpty,
            let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: WidgetGroup.id)
        else { return nil }
        return SharedTasksFile(url: container.appendingPathComponent("Share/Tasks.json"))
    }

    /// The tasks waiting, oldest first.
    package func read() throws(SharedTasksError) -> [SharedTask] {
        guard FileManager.default.fileExists(atPath: url.path) else { return [] }
        do {
            return try JSONDecoder().decode([SharedTask].self, from: Data(contentsOf: url))
        } catch {
            throw .readFailed(url, error)
        }
    }

    /// Leaves `task` for Lifr to save.
    package func append(_ task: SharedTask) throws(SharedTasksError) {
        try write(try read() + [task])
    }

    /// The tasks waiting, leaving none.
    package func take() throws(SharedTasksError) -> [SharedTask] {
        let tasks = try read()
        guard !tasks.isEmpty else { return [] }
        try write([])
        return tasks
    }

    /// Replaces the file with `tasks`, readable by this user only and, on
    /// iPhone, sealed until the first unlock; removes it when there are none.
    private func write(_ tasks: [SharedTask]) throws(SharedTasksError) {
        do {
            guard !tasks.isEmpty else {
                if FileManager.default.fileExists(atPath: url.path) { try FileManager.default.removeItem(at: url) }
                return
            }
            try FileManager.default.createDirectory(
                at: url.deletingLastPathComponent(), withIntermediateDirectories: true,
                attributes: [.posixPermissions: 0o700])
            var options: Data.WritingOptions = .atomic
            #if os(iOS)
                options.insert(.completeFileProtectionUntilFirstUserAuthentication)
            #endif
            try JSONEncoder().encode(tasks).write(to: url, options: options)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        } catch {
            throw .writeFailed(url, error)
        }
    }
}
