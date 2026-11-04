import Foundation
import SwiftData

// SwiftData models for Lifr's records, synced through CloudKit's private
// database. CloudKit shapes them:
//
// - Every property has a default or is optional, and every relationship is
//   optional with an inverse.
// - Nothing is `.unique`: Core enforces unique codes and keys, and
//   reconciliation fixes duplicates that sync brings in.
// - Once the schema is in CloudKit production, changes are additive only.
//   Never rename or remove a model or property, and never change a
//   property's encryption.
//
// User content (names, titles, details, comments, history values, labels)
// is encrypted end to end with `.allowsCloudEncryption`. IDs, timestamps,
// enums and relationships stay plain.
//
// Enums, calendar dates (YYYY-MM-DD) and times of day (HH:MM) are stored as
// their raw strings, so an older app version can still load what a newer one
// wrote.

/// The models in the order they're registered.
let models: [any PersistentModel.Type] = [
    StoredAvenue.self, StoredProject.self, StoredTask.self, StoredLabel.self, StoredComment.self,
    StoredReminder.self, StoredHistoryEvent.self, StoredSettings.self, StoredJournalEntry.self,
]

@Model
final class StoredAvenue {
    var id = UUID()
    var key = ""
    @Attribute(.allowsCloudEncryption) var name = ""
    var color = ""
    var symbol = ""
    var sortOrder = ""
    var agentAccess = ""
    var createdAt = Date.distantPast
    var archivedAt: Date?

    @Relationship(deleteRule: .nullify, inverse: \StoredProject.avenue) var projects: [StoredProject]?
    @Relationship(deleteRule: .nullify, inverse: \StoredTask.avenue) var tasks: [StoredTask]?

    init(id: UUID) { self.id = id }
}

@Model
final class StoredProject {
    var id = UUID()
    var avenue: StoredAvenue?
    @Attribute(.allowsCloudEncryption) var name = ""
    @Attribute(.allowsCloudEncryption) var details = ""
    var status = ""
    var dueDate: String?
    var sortOrder = ""
    var createdAt = Date.distantPast
    var completedAt: Date?

    @Relationship(deleteRule: .nullify, inverse: \StoredTask.project) var tasks: [StoredTask]?

    init(id: UUID) { self.id = id }
}

@Model
final class StoredTask {
    var id = UUID()
    var code = ""
    @Attribute(.allowsCloudEncryption) var title = ""
    @Attribute(.allowsCloudEncryption) var details = ""
    var status = ""
    var priority = ""
    var startDate: String?
    var dueDate: String?
    var dueTime: String?
    var avenue: StoredAvenue?
    var project: StoredProject?
    var parent: StoredTask?
    var sortOrder = ""
    var createdAt = Date.distantPast
    var updatedAt = Date.distantPast
    var lastActivityAt = Date.distantPast
    var closedAt: Date?
    var deletedAt: Date?

    @Relationship(deleteRule: .nullify, inverse: \StoredTask.parent) var subtasks: [StoredTask]?
    @Relationship(deleteRule: .nullify, inverse: \StoredLabel.tasks) var labels: [StoredLabel]?
    @Relationship(deleteRule: .cascade, inverse: \StoredComment.task) var comments: [StoredComment]?
    @Relationship(deleteRule: .cascade, inverse: \StoredReminder.task) var reminders: [StoredReminder]?
    @Relationship(deleteRule: .cascade, inverse: \StoredHistoryEvent.task) var history: [StoredHistoryEvent]?

    init(id: UUID) { self.id = id }
}

@Model
final class StoredLabel {
    var id = UUID()
    @Attribute(.allowsCloudEncryption) var name = ""
    var color = ""
    var createdAt = Date.distantPast
    var tasks: [StoredTask]?

    init(id: UUID) { self.id = id }
}

@Model
final class StoredComment {
    var id = UUID()
    var task: StoredTask?
    @Attribute(.allowsCloudEncryption) var body = ""
    var authorKind = ""
    @Attribute(.allowsCloudEncryption) var authorDevice = ""
    @Attribute(.allowsCloudEncryption) var authorClient: String?
    /// `Actor.Surface`; nil for comments written before it was recorded.
    var authorSurface: String?
    var createdAt = Date.distantPast
    var editedAt: Date?
    var deletedAt: Date?

    init(id: UUID) { self.id = id }
}

/// Either `fireAt` or `secondsBeforeDue` is set.
@Model
final class StoredReminder {
    var id = UUID()
    var task: StoredTask?
    var fireAt: Date?
    var secondsBeforeDue: Int?

    init(id: UUID) { self.id = id }
}

/// Written once, never changed.
@Model
final class StoredHistoryEvent {
    var id = UUID()
    var task: StoredTask?
    var actorKind = ""
    @Attribute(.allowsCloudEncryption) var actorDevice = ""
    @Attribute(.allowsCloudEncryption) var actorClient: String?
    /// `Actor.Surface`; nil for events written before it was recorded.
    var actorSurface: String?
    var kind = ""
    var field: String?
    @Attribute(.allowsCloudEncryption) var oldValue: String?
    @Attribute(.allowsCloudEncryption) var newValue: String?
    var reason: String?
    var createdAt = Date.distantPast

    init(id: UUID) { self.id = id }
}

/// One per person. Two devices that start offline each create one; the
/// oldest wins.
@Model
final class StoredSettings {
    var id = UUID()
    var procrastinatedAfterDays = 7
    var createdAt = Date.distantPast

    init(id: UUID) { self.id = id }
}

/// One journal entry. Written once, never changed or deleted. The steps
/// hold whole records, so they're encrypted like the records are.
@Model
final class StoredJournalEntry {
    var id = UUID()
    var at = Date.distantPast
    var actorKind = ""
    @Attribute(.allowsCloudEncryption) var actorDevice = ""
    @Attribute(.allowsCloudEncryption) var actorClient: String?
    var actorSurface: String?
    var conversation: UUID?
    /// `[UUID]` as JSON.
    var reverts: Data?
    /// `[JournalStep]` as JSON.
    @Attribute(.allowsCloudEncryption) var steps: Data?

    init(id: UUID) { self.id = id }
}
