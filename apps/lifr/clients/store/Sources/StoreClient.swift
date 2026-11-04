import ComposableArchitecture
import Foundation
import LifrCore

/// A Core operation on the records. It returns what it changed, or throws
/// a `LifrError` and changes nothing.
package typealias StoreOperation = @Sendable (inout Workspace, Context) throws(LifrError) -> Change

/// Reads and changes Lifr's records. Each change runs Core's rules, so the
/// errors are `LifrError`s whose descriptions can be shown as is.
@DependencyClient
package struct StoreClient: Sendable {
    /// Every record.
    package var load: @Sendable () async throws -> Workspace
    /// Runs `operation` on the records and saves what it changed, all or
    /// nothing. The named operations below are built on this.
    package var save: @Sendable (_ operation: StoreOperation) async throws -> Saved
    /// The records each time something other than this client changed them,
    /// such as a sync import, with what reconciling them fixed. Ends when the
    /// calling task is cancelled.
    package var updates: @Sendable () async -> AsyncStream<Update> = { AsyncStream { $0.finish() } }
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

/// The records after another device or process changed them.
package struct Update: Hashable, Sendable {
    package var workspace: Workspace
    /// What reconciling the imported records fixed, for the banner that
    /// explains it.
    package var fixes: [Fix]

    package init(workspace: Workspace, fixes: [Fix] = []) {
        self.workspace = workspace
        self.fixes = fixes
    }
}

extension StoreClient {
    /// `Workspace.setStatus`. Pass `cascade: true` once the person has
    /// confirmed closing open Sub tasks as Won't Do.
    package func setStatus(_ status: TaskStatus, _ id: UUID, _ cascade: Bool = false) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.setStatus(status, of: id, cascade: cascade, in: context)
        }
    }

    /// Reverts a change made earlier, such as a parent reopened by its Sub task.
    package func undo(_ change: Change) async throws -> Saved {
        try await save { workspace, context in workspace.undo(change, in: context) }
    }

    package func createTask(_ new: Workspace.NewTask) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in try workspace.createTask(new, in: context) }
    }

    package func edit(_ id: UUID, _ edit: Workspace.TaskEdit) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in try workspace.edit(id, edit, in: context) }
    }

    package func addComment(_ body: String, to id: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            _ = try workspace.addComment(body, to: id, in: context)
            return Change()
        }
    }

    package func createAvenue(key: String, name: String) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.createAvenue(key: key, name: name, in: context)
            return Change()
        }
    }

    package func createProject(_ name: String, in avenueID: UUID) async throws -> Saved {
        try await save { (workspace, context) throws(LifrError) in
            try workspace.createProject(name: name, inAvenue: avenueID, in: context)
            return Change()
        }
    }

    package func setSettings(_ settings: Settings) async throws -> Saved {
        try await save { workspace, _ in
            workspace.settings = settings
            return Change()
        }
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
