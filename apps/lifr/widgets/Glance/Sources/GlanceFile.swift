import Foundation

/// Why the widgets' file couldn't be read or written.
package enum GlanceError: Error, CustomStringConvertible {
    /// The file is there but couldn't be read or decoded.
    case readFailed(URL, any Error)
    /// The file couldn't be written.
    case writeFailed(URL, any Error)

    package var description: String {
        switch self {
        case .readFailed(let url, let error):
            String(
                localized: """
                    The widgets couldn't read Today from \(url.path) (\(error.localizedDescription)). Open Lifr \
                    to write it again.
                    """)
        case .writeFailed(let url, let error):
            String(
                localized: """
                    Lifr couldn't update the widgets at \(url.path) (\(error.localizedDescription)). They show \
                    the last Today they had until the next change saves.
                    """)
        }
    }
}

/// The file the app writes Today to and the widgets read, in the App
/// Group's container that only Lifr's own apps can open.
package struct GlanceFile: Sendable {
    package let url: URL

    package init(url: URL) {
        self.url = url
    }

    /// The file in the App Group's container, or nil when this build has
    /// no App Group, such as an unsigned iPhone build.
    package static var shared: GlanceFile? {
        guard !WidgetGroup.id.isEmpty,
            let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: WidgetGroup.id)
        else { return nil }
        return GlanceFile(url: container.appendingPathComponent("Widgets/Today.json"))
    }

    /// What the app last wrote, or nil when it hasn't yet.
    package func read() throws(GlanceError) -> TodayGlance? {
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        do {
            let decoder = JSONDecoder()
            decoder.dateDecodingStrategy = .iso8601
            return try decoder.decode(TodayGlance.self, from: Data(contentsOf: url))
        } catch {
            throw .readFailed(url, error)
        }
    }

    /// Replaces the file with `glance`, readable by this user only, kept out
    /// of backups and, on iPhone, sealed until the first unlock.
    package func write(_ glance: TodayGlance) throws(GlanceError) {
        do {
            let folder = url.deletingLastPathComponent()
            try FileManager.default.createDirectory(
                at: folder, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
            let encoder = JSONEncoder()
            encoder.dateEncodingStrategy = .iso8601
            encoder.outputFormatting = .sortedKeys
            var options: Data.WritingOptions = .atomic
            #if os(iOS)
                options.insert(.completeFileProtectionUntilFirstUserAuthentication)
            #endif
            try encoder.encode(glance).write(to: url, options: options)
            try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            var excluded = url
            try excluded.setResourceValues(values)
        } catch {
            throw .writeFailed(url, error)
        }
    }
}
