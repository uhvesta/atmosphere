import Foundation

/// Who is making a change, when, and where new IDs and codes come from. Tests
/// pass fixed ones so results are repeatable.
package struct Context {
    package var actor: Actor
    package var now: Date
    var newID: () -> UUID
    var newCode: () -> TaskCode

    package init(
        actor: Actor,
        now: Date,
        newID: @escaping () -> UUID = UUID.init,
        newCode: @escaping () -> TaskCode = {
            var generator = SystemRandomNumberGenerator()
            return TaskCode.random(using: &generator)
        }
    ) {
        self.actor = actor
        self.now = now
        self.newID = newID
        self.newCode = newCode
    }

    var system: Actor { .system(device: actor.device) }
}

/// What one operation changed, so a client can show it and undo it.
package struct Change: Hashable, Sendable {
    /// Every task the operation touched, as it was before. `nil` for tasks it
    /// created.
    package internal(set) var previous: [UUID: LifrTask?] = [:]
    package internal(set) var events: [HistoryEvent] = []
    /// A closed parent the operation moved back to In Progress because one of
    /// its Sub tasks is open again. The UI offers to undo the whole change.
    package internal(set) var reopenedParent: UUID?

    package var isEmpty: Bool { previous.isEmpty && events.isEmpty }
}
