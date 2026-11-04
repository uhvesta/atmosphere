import ComposableArchitecture
import Foundation
import LifrCore

extension StoreClient {
    /// Keeps the records in memory, starting from `workspace`. Changes use
    /// the `date`, `uuid` and `withRandomNumberGenerator` dependencies, so
    /// tests can fix them.
    package static func inMemory(_ workspace: Workspace, actor: Actor = .user(device: "This device")) -> Self {
        let records = LockIsolated(workspace)

        @Sendable func save(_ operation: (inout Workspace, Context) throws(LifrError) -> Change) throws(LifrError)
            -> Saved
        {
            @Dependency(\.date.now) var now
            @Dependency(\.uuid) var uuid
            @Dependency(\.withRandomNumberGenerator) var random
            let context = Context(
                actor: actor, now: now, newID: { uuid() },
                newCode: { random { TaskCode.random(using: &$0) } })
            var copy = records.value
            let change = try operation(&copy, context)
            records.setValue(copy)
            return Saved(workspace: copy, change: change)
        }

        return StoreClient(
            load: { records.value },
            setStatus: { status, id, cascade in
                try save { (workspace, context) throws(LifrError) in
                    try workspace.setStatus(status, of: id, cascade: cascade, in: context)
                }
            },
            undo: { change in
                try save { workspace, context in workspace.undo(change, in: context) }
            }
        )
    }
}
