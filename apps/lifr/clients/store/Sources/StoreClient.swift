import ComposableArchitecture
import Foundation
import LifrCore

/// Reads and changes Lifr's records. Each change runs Core's rules, so the
/// errors are `LifrError`s whose descriptions can be shown as is.
@DependencyClient
package struct StoreClient: Sendable {
    /// Every record.
    package var load: @Sendable () async throws -> Workspace
    /// `Workspace.setStatus`. Pass `cascade: true` once the person has
    /// confirmed closing open Sub tasks as Won't Do.
    package var setStatus: @Sendable (_ status: TaskStatus, _ of: UUID, _ cascade: Bool) async throws -> Saved
    /// Reverts a change made earlier, such as a parent reopened by its Sub task.
    package var undo: @Sendable (_ change: Change) async throws -> Saved
}

/// The records after a change, and what changed.
package struct Saved: Hashable, Sendable {
    package var workspace: Workspace
    package var change: Change

    package init(workspace: Workspace, change: Change) {
        self.workspace = workspace
        self.change = change
    }
}

extension StoreClient: TestDependencyKey {
    package static let testValue = StoreClient()
    package static let previewValue = StoreClient.inMemory(.sample(now: Date(), timeZone: .current))
}

extension DependencyValues {
    package var store: StoreClient {
        get { self[StoreClient.self] }
        set { self[StoreClient.self] = newValue }
    }
}
