import ComposableArchitecture
import Foundation

/// How the person wants the assistant to behave, in their own words: the
/// opening of its instructions. Lifr adds the rest itself (what the
/// records are, the Avenues, the tools, today's date and the answer's
/// shape), so no edit here can stop drafts or tools from working.
///
/// Every version the person saved is kept, newest first, so any can be
/// used again, as can Lifr's default. It stays on this device.
package struct Guidance: Hashable, Codable, Sendable {
    package struct Version: Hashable, Codable, Sendable, Identifiable {
        package var id: UUID
        package var text: String
        package var savedAt: Date

        package init(id: UUID, text: String, savedAt: Date) {
            self.id = id
            self.text = text
            self.savedAt = savedAt
        }
    }

    /// The version in use, or nil for Lifr's default.
    package var current: UUID?
    /// Every version saved, newest first.
    package var history: [Version] = []

    package init(current: UUID? = nil, history: [Version] = []) {
        self.current = current
        self.history = history
    }

    /// Lifr's own opening.
    package static let defaultText = """
        You help the person plan what to add to Lifr, their task manager. Ask short questions when you need \
        to, then propose a plan. Keep "message" to one to three sentences; the plan shows the details.
        """

    /// The most characters, so the instructions still fit the on-device
    /// model's window with the records and tools.
    package static let limit = 600
    /// The most versions kept; the oldest go first.
    package static let historyLimit = 30

    /// What the assistant is told now.
    package var text: String {
        current.flatMap { id in history.first { $0.id == id }?.text } ?? Self.defaultText
    }

    package var isDefault: Bool { current == nil }

    /// Saves `text` as a new version and uses it. Text that is Lifr's
    /// default goes back to the default instead.
    package mutating func save(_ text: String, id: UUID, at date: Date) throws(GuidanceError) {
        let text = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw .empty }
        guard text.count <= Self.limit else { throw .tooLong(text.count) }
        guard text != Self.defaultText else {
            current = nil
            return
        }
        guard text != self.text else { return }
        history.insert(Version(id: id, text: text, savedAt: date), at: 0)
        current = id
        history = Array(history.prefix(Self.historyLimit))
    }

    /// Uses a saved version again, or Lifr's default for nil.
    package mutating func use(_ id: UUID?) {
        guard let id else {
            current = nil
            return
        }
        guard history.contains(where: { $0.id == id }) else { return }
        current = id
    }
}

/// The person's instructions couldn't be saved as written.
package enum GuidanceError: Error, Hashable, CustomStringConvertible {
    case empty
    case tooLong(Int)
    /// Reading or writing them on this device failed.
    case storageFailed(String)

    package var description: String {
        switch self {
        case .empty:
            String(localized: "Write how the assistant should behave, or use Lifr's default.")
        case .tooLong(let count):
            String(
                localized: """
                    The instructions are \(count) characters; the most is \(Guidance.limit), so the \
                    assistant still has room for your records. Shorten them, then save.
                    """)
        case .storageFailed(let reason):
            String(
                localized: """
                    Couldn't keep the instructions on this device (\(reason)). The assistant uses the ones \
                    from before. Check that this device has free storage, then try again.
                    """)
        }
    }
}

/// Keeps the person's instructions and their history on this device.
@DependencyClient
package struct GuidanceClient: Sendable {
    /// Throws `GuidanceError.storageFailed`.
    package var load: @Sendable () async throws -> Guidance
    /// Throws `GuidanceError.storageFailed`.
    package var save: @Sendable (Guidance) async throws -> Void
}

extension GuidanceClient {
    /// Kept in memory, for tests and previews.
    package static func inMemory(_ guidance: Guidance = Guidance()) -> Self {
        let stored = LockIsolated(guidance)
        return GuidanceClient(load: { stored.value }, save: { stored.setValue($0) })
    }
}

extension GuidanceClient: TestDependencyKey {
    package static let testValue = GuidanceClient()
    package static let previewValue = GuidanceClient.inMemory()
}

extension DependencyValues {
    package var guidance: GuidanceClient {
        get { self[GuidanceClient.self] }
        set { self[GuidanceClient.self] = newValue }
    }
}
