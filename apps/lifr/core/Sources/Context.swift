import Foundation

/// Who is making a change, when, and where new IDs and codes come from. Tests
/// pass fixed ones so results are repeatable.
package struct Context {
    package var actor: Actor
    package var now: Date
    /// Where "today" is, for the next occurrence of a repeating task.
    package var timeZone: TimeZone
    var newID: () -> UUID
    var newCode: () -> TaskCode

    package init(
        actor: Actor,
        now: Date,
        timeZone: TimeZone = .current,
        newID: @escaping () -> UUID = UUID.init,
        newCode: @escaping () -> TaskCode = {
            var generator = SystemRandomNumberGenerator()
            return TaskCode.random(using: &generator)
        }
    ) {
        self.actor = actor
        self.now = now
        self.timeZone = timeZone
        self.newID = newID
        self.newCode = newCode
    }

    var system: Actor { .system(device: actor.device, surface: actor.surface) }
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
    /// Records the operation added besides tasks' own, in the order it added
    /// them, so undo can remove them too.
    package internal(set) var added: [Added] = []

    package enum Added: Hashable, Sendable {
        case avenue(UUID)
        case project(UUID)
        case label(UUID)
        case task(UUID)
        case comment(UUID)
        case reminder(UUID)
    }

    /// Who the journal credits with the change, when not the context's
    /// actor: a session sets the approved client here.
    package var actor: Actor?
    /// The assistant conversation the change was made in.
    package var conversation: UUID?
    /// The journal entries this change reverts.
    package internal(set) var reverts: [UUID] = []
    /// What reverting left as it was, and why.
    package internal(set) var skipped: [SkippedRevert] = []
    /// The journal entry `perform` wrote for the change, if it changed
    /// anything.
    package internal(set) var entry: UUID?

    package init() {}

    /// Adds what `other` changed to this change. A task keeps the state it
    /// had before the first of them.
    package mutating func merge(_ other: Change) {
        for (id, task) in other.previous where previous[id] == nil {
            previous[id] = task
        }
        events += other.events
        reopenedParent = reopenedParent ?? other.reopenedParent
        added += other.added
        actor = actor ?? other.actor
        conversation = conversation ?? other.conversation
        reverts += other.reverts
        skipped += other.skipped
    }

    package var isEmpty: Bool { previous.isEmpty && events.isEmpty }
}
