import ComposableArchitecture
import CoreSpotlight
import Foundation
import OSLog

/// A task as the system's search lists it.
package struct SpotlightItem: Hashable, Sendable {
    package var id: UUID
    package var title: String
    /// Its code and Avenue, such as "HEALTH-0004 · Health".
    package var detail: String
    /// Its description, so its words find it too.
    package var details: String

    package init(id: UUID, title: String, detail: String, details: String) {
        self.id = id
        self.title = title
        self.detail = detail
        self.details = details
    }
}

/// Keeps the tasks in the system's search the same as Lifr's.
package struct SpotlightClient: Sendable {
    /// Replaces what the system's search has with `items`; none takes
    /// every task out, as while the lock is on. Failing leaves the last
    /// ones listed, so it reports nothing to the person.
    package var index: @Sendable (_ items: [SpotlightItem]) async -> Void

    package init(index: @escaping @Sendable (_ items: [SpotlightItem]) async -> Void) {
        self.index = index
    }
}

extension SpotlightClient {
    /// The tasks' group in the system's search.
    private static let domain = "tasks"

    /// Lists the tasks in the device's search, skipping a list that's the
    /// same as the last.
    package static func live() -> Self {
        let last = LockIsolated<[SpotlightItem]?>(nil)
        return SpotlightClient { items in
            guard items != last.value else { return }
            let index = CSSearchableIndex.default()
            let searchable = items.map { item in
                let attributes = CSSearchableItemAttributeSet(contentType: .text)
                attributes.title = item.title
                attributes.contentDescription = item.details.isEmpty ? item.detail : "\(item.detail)\n\(item.details)"
                attributes.keywords = [item.detail]
                return CSSearchableItem(
                    uniqueIdentifier: item.id.uuidString, domainIdentifier: domain, attributeSet: attributes)
            }
            do {
                try await index.deleteSearchableItems(withDomainIdentifiers: [domain])
                if !searchable.isEmpty { try await index.indexSearchableItems(searchable) }
                last.setValue(items)
            } catch {
                let message = error.localizedDescription
                Logger(subsystem: "lifr", category: "spotlight").error("\(message, privacy: .public)")
            }
        }
    }

    /// What Lifr is handed when opened from a search result.
    package static let activityType = CSSearchableItemActionType

    /// The task a search result names, when Lifr is opened from one.
    package static func task(in activity: NSUserActivity) -> UUID? {
        guard activity.activityType == CSSearchableItemActionType,
            let id = activity.userInfo?[CSSearchableItemActivityIdentifier] as? String
        else { return nil }
        return UUID(uuidString: id)
    }

    /// Keeps what was last listed, for tests.
    package static func inMemory() -> (client: Self, indexed: LockIsolated<[SpotlightItem]?>) {
        let indexed = LockIsolated<[SpotlightItem]?>(nil)
        return (SpotlightClient { indexed.setValue($0) }, indexed)
    }
}

extension SpotlightClient: TestDependencyKey {
    /// Search isn't part of most tests, so listing does nothing; tests of
    /// what's listed use `inMemory`.
    package static let testValue = SpotlightClient { _ in }
    package static let previewValue = SpotlightClient { _ in }
}

extension DependencyValues {
    package var spotlight: SpotlightClient {
        get { self[SpotlightClient.self] }
        set { self[SpotlightClient.self] = newValue }
    }
}
