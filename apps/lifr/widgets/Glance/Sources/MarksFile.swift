import Foundation

/// Why the tasks marked from a widget couldn't be read or kept.
package enum MarksError: Error, CustomStringConvertible {
    /// The file is there but couldn't be read or decoded.
    case readFailed(URL, any Error)
    /// The file couldn't be written or removed.
    case writeFailed(URL, any Error)
    /// This build has no App Group to keep the mark in, such as an unsigned
    /// iPhone build.
    case noAppGroup

    package var description: String {
        switch self {
        case .noAppGroup:
            String(
                localized: """
                    This copy of Lifr has no App Group, so a task can't be marked Done from outside it. \
                    Open Lifr and mark it there.
                    """)
        case .readFailed(let url, let error):
            String(
                localized: """
                    Lifr couldn't read the tasks marked Done from a widget at \(url.path) \
                    (\(error.localizedDescription)). Check them in Lifr and mark them there.
                    """)
        case .writeFailed(let url, let error):
            String(
                localized: """
                    Lifr couldn't keep a task marked Done from a widget at \(url.path) \
                    (\(error.localizedDescription)). Open Lifr and mark it there.
                    """)
        }
    }
}

/// The tasks marked Done from a widget, kept in the App Group until Lifr
/// next opens and saves them, since a widget never opens the records.
package struct MarksFile: Sendable {
    package let url: URL

    package init(url: URL) {
        self.url = url
    }

    /// The file in the App Group's container, or nil when this build has
    /// no App Group.
    package static var shared: MarksFile? {
        guard !WidgetGroup.id.isEmpty,
            let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: WidgetGroup.id)
        else { return nil }
        return MarksFile(url: container.appendingPathComponent("Widgets/Marked.json"))
    }

    /// The tasks marked and not yet saved.
    package func read() throws(MarksError) -> Set<UUID> {
        guard FileManager.default.fileExists(atPath: url.path) else { return [] }
        do {
            return try JSONDecoder().decode(Set<UUID>.self, from: Data(contentsOf: url))
        } catch {
            throw .readFailed(url, error)
        }
    }

    /// Marks the task, or unmarks it when it already was; true when it's
    /// marked now.
    @discardableResult
    package func toggle(_ id: UUID) throws(MarksError) -> Bool {
        var marked = try read()
        let isMarked = marked.insert(id).inserted
        if !isMarked { marked.remove(id) }
        try write(marked)
        return isMarked
    }

    /// Marks the task, leaving it marked when it already was.
    package func mark(_ id: UUID) throws(MarksError) {
        var marked = try read()
        guard marked.insert(id).inserted else { return }
        try write(marked)
    }

    /// The marked tasks, leaving none marked.
    package func take() throws(MarksError) -> Set<UUID> {
        let marked = try read()
        guard !marked.isEmpty else { return [] }
        try write([])
        return marked
    }

    /// Replaces the file with `marked`, readable by this user only and, on
    /// iPhone, sealed until the first unlock; removes it when none are.
    private func write(_ marked: Set<UUID>) throws(MarksError) {
        do {
            guard !marked.isEmpty else {
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
            try JSONEncoder().encode(marked).write(to: url, options: options)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        } catch {
            throw .writeFailed(url, error)
        }
    }
}

extension TodayGlance {
    /// This day and the ones after it as they read with `marked` done:
    /// the rows stay, so a mis-tap shows, but they're counted no more.
    package func marking(_ marked: Set<UUID>) -> TodayGlance {
        guard !marked.isEmpty else { return self }
        var glance = self
        let done = rows.filter { marked.contains($0.id) }
        glance.count = max(0, count - done.count)
        glance.overdue = max(0, overdue - done.filter(\.isOverdue).count)
        glance.later = later.map { $0.marking(marked) }
        return glance
    }
}
