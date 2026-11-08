import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import SwiftData

/// Lifr's records in SwiftData. Every change loads the records into a Core
/// `Workspace`, runs Core's operation on it, and writes back only the
/// records that changed, in one save. So the rules live in Core alone.
package actor LiveStore: ModelActor {
    // Written out because `@ModelActor` makes these internal.
    package nonisolated let modelContainer: ModelContainer
    package nonisolated let modelExecutor: any ModelExecutor
    /// Journal entries never change, so each is decoded once.
    private var journal: [UUID: JournalEntry] = [:]
    /// How many saves there have been, to number them.
    private var saves = 0
    /// What each sync import in this process changed, and each change an
    /// announcing client saved, for `updates`.
    nonisolated let imported = Broadcast<Update>()

    package init(modelContainer: ModelContainer) {
        self.modelContainer = modelContainer
        modelExecutor = DefaultSerialModelExecutor(modelContext: ModelContext(modelContainer))
    }

    /// Every record. See Mapping.swift for records that are skipped until
    /// sync brings in what they refer to.
    package func load() throws(LiveStoreError) -> Workspace {
        do {
            return try read(Index(modelContext))
        } catch {
            throw .loadFailed(error)
        }
    }

    /// Runs `operation` on the records and saves what it changed. Refused
    /// operations throw `LifrError` and change nothing.
    package func save(
        in context: @Sendable () -> Context, _ operation: StoreOperation
    ) throws -> Saved {
        let before = try load()
        var after = before
        let change = try after.perform(in: context(), operation)
        try write(from: before, to: after)
        saves += 1
        return Saved(workspace: after, change: change, number: saves)
    }

    /// Fixes rule violations that sync brought in, as Core's `reconcile`
    /// does, and saves the fixes with their history. Call it after each
    /// CloudKit import. Two devices fixing the same records make the same
    /// changes.
    package func reconcile(in context: @Sendable () -> Context) throws(LiveStoreError) -> [Fix] {
        let before = try load()
        var after = before
        var fixes: [Fix] = []
        after.perform(in: context()) { workspace, context in
            fixes = workspace.reconcile(in: context)
            return Change()
        }
        if !fixes.isEmpty { try write(from: before, to: after) }
        return fixes
    }

    /// Writes what sync brought in without a journal entry of its own, then
    /// reconciles, all in one save. Returns what reconciling fixed.
    package func importSynced(
        _ changes: SyncedChanges, in context: @Sendable () -> Context
    ) throws(LiveStoreError) -> [Fix] {
        let before = try load()
        var after = before
        after.importSynced(records: changes.records, journal: changes.journal, history: changes.history)
        var fixes: [Fix] = []
        after.perform(in: context()) { workspace, context in
            fixes = workspace.reconcile(in: context)
            return Change()
        }
        try write(from: before, to: after)
        return fixes
    }

    /// Moves the journal entries made before `cutoff` into each month's
    /// archive, as the fields they changed, in one save. Entries this
    /// version can't read stay as they are, and so do archives it can't
    /// read: a new archive for that month is added beside one.
    package func archiveJournal(before cutoff: Date) throws(LiveStoreError) {
        do {
            let old = try modelContext.fetch(
                FetchDescriptor<StoredJournalEntry>(predicate: #Predicate { $0.at < cutoff }))
            let readable = old.compactMap { model in (journal[model.id] ?? model.record).map { (model, $0) } }
            guard !readable.isEmpty else { return }
            let archives = try modelContext.fetch(FetchDescriptor<StoredJournalArchive>())
            let months = Dictionary(grouping: readable.map { $0.1.archived }) { JournalMonth(containing: $0.at) }
            for (month, entries) in months {
                let stored = archives.filter { $0.month == month.description }
                    .compactMap { model in model.archived.map { (model, $0) } }
                let merged = stored.flatMap(\.1).merging(entries)
                let archive = stored.first?.0 ?? StoredJournalArchive(id: UUID())
                if stored.isEmpty {
                    archive.month = month.description
                    modelContext.insert(archive)
                }
                try archive.write(merged)
                for (model, _) in stored.dropFirst() { modelContext.delete(model) }
            }
            for (model, entry) in readable {
                modelContext.delete(model)
                journal[entry.id] = nil
            }
            try modelContext.save()
        } catch {
            modelContext.rollback()
            throw .archiveFailed(error)
        }
    }

    /// Every archived journal entry this version can read, oldest first.
    package func archivedJournal() throws(LiveStoreError) -> [ArchivedEntry] {
        do {
            return try modelContext.fetch(FetchDescriptor<StoredJournalArchive>())
                .compactMap(\.archived).reduce([]) { $0.merging($1) }
        } catch {
            throw .loadFailed(error)
        }
    }

    /// Adds every record in `workspace` that isn't stored yet, and changes
    /// none that are, such as fixtures in tests.
    package func insert(_ workspace: Workspace) throws(LiveStoreError) {
        let before = try load()
        let merged = Workspace(
            avenues: Array(before.avenues.values) + workspace.avenues.values.filter { before.avenues[$0.id] == nil },
            projects: Array(before.projects.values)
                + workspace.projects.values.filter { before.projects[$0.id] == nil },
            tasks: Array(before.tasks.values) + workspace.tasks.values.filter { before.tasks[$0.id] == nil },
            labels: Array(before.labels.values) + workspace.labels.values.filter { before.labels[$0.id] == nil },
            comments: Array(before.comments.values)
                + workspace.comments.values.filter { before.comments[$0.id] == nil },
            reminders: Array(before.reminders.values)
                + workspace.reminders.values.filter { before.reminders[$0.id] == nil },
            history: before.history
                + workspace.history.filter { event in
                    !before.history.contains { $0.id == event.id }
                },
            journal: before.journal, settings: before.settings)
        try write(from: before, to: merged)
    }

    // MARK: - Reading

    private func read(_ index: Index) throws -> Workspace {
        let settings = try modelContext.fetch(
            FetchDescriptor<StoredSettings>(sortBy: [SortDescriptor(\.createdAt), SortDescriptor(\.id)]))
        return Workspace(
            avenues: index.avenues.values.compactMap(\.record),
            projects: index.projects.values.compactMap(\.record),
            tasks: index.tasks.values.compactMap(\.record),
            labels: index.labels.values.map(\.record),
            comments: index.comments.values.compactMap(\.record),
            reminders: index.reminders.values.compactMap(\.record),
            // Each device appends in its own order; sync interleaves them by
            // time.
            history: index.history.values.compactMap(\.record)
                .sorted { ($0.createdAt, $0.id.uuidString) < ($1.createdAt, $1.id.uuidString) },
            journal: journal(index),
            settings: settings.first.map { Settings(procrastinatedAfterDays: $0.procrastinatedAfterDays) }
                ?? Settings())
    }

    /// The journal, oldest first. Each device appends in its own order;
    /// sync interleaves them by time.
    private func journal(_ index: Index) -> [JournalEntry] {
        for (id, model) in index.journal where journal[id] == nil {
            journal[id] = model.record
        }
        return index.journal.keys.compactMap { journal[$0] }
            .sorted { ($0.at, $0.id.uuidString) < ($1.at, $1.id.uuidString) }
    }

    // MARK: - Writing

    private func write(from before: Workspace, to after: Workspace) throws(LiveStoreError) {
        do {
            let index = try Index(modelContext)
            sync(before.avenues, after.avenues, &index.avenues) { $0.write($1) }
            sync(before.labels, after.labels, &index.labels) { $0.write($1) }
            sync(before.projects, after.projects, &index.projects) { $0.write($1, in: index) }
            sync(before.tasks, after.tasks, &index.tasks) { $0.write($1, in: index) }
            // Parents last: a new Sub task's parent may be new too.
            for (id, task) in after.tasks where before.tasks[id] != task {
                index.tasks[id]?.writeParent(task, in: index)
            }
            sync(before.comments, after.comments, &index.comments) { $0.write($1, in: index) }
            sync(before.reminders, after.reminders, &index.reminders) { $0.write($1, in: index) }
            let previous = Dictionary(before.history.map { ($0.id, $0) }) { first, _ in first }
            let next = Dictionary(after.history.map { ($0.id, $0) }) { first, _ in first }
            sync(previous, next, &index.history) { $0.write($1, in: index) }
            // Append-only: entries are only ever added.
            for entry in after.journal where index.journal[entry.id] == nil {
                let model = StoredJournalEntry(id: entry.id)
                modelContext.insert(model)
                try model.write(entry)
                index.journal[entry.id] = model
            }
            if before.settings != after.settings {
                try storedSettings().procrastinatedAfterDays = after.settings.procrastinatedAfterDays
            }
            try modelContext.save()
        } catch {
            modelContext.rollback()
            throw .saveFailed(error)
        }
    }

    /// Inserts, updates and deletes models so they match `after`.
    private func sync<Record: Equatable, Model: PersistentModel>(
        _ before: [UUID: Record], _ after: [UUID: Record], _ models: inout [UUID: Model],
        _ write: (Model, Record) -> Void
    ) where Model: Identified {
        for (id, record) in after where before[id] != record {
            let model = models[id] ?? Model(id: id)
            if models[id] == nil {
                modelContext.insert(model)
                models[id] = model
            }
            write(model, record)
        }
        for id in before.keys where after[id] == nil {
            if let model = models.removeValue(forKey: id) { modelContext.delete(model) }
        }
    }

    private func storedSettings() throws -> StoredSettings {
        let stored = try modelContext.fetch(
            FetchDescriptor<StoredSettings>(sortBy: [SortDescriptor(\.createdAt), SortDescriptor(\.id)]))
        if let first = stored.first { return first }
        let settings = StoredSettings(id: UUID())
        settings.createdAt = Date()
        modelContext.insert(settings)
        return settings
    }
}

/// Every stored model by ID.
final class Index {
    var avenues: [UUID: StoredAvenue]
    var projects: [UUID: StoredProject]
    var tasks: [UUID: StoredTask]
    var labels: [UUID: StoredLabel]
    var comments: [UUID: StoredComment]
    var reminders: [UUID: StoredReminder]
    var history: [UUID: StoredHistoryEvent]
    var journal: [UUID: StoredJournalEntry]

    init(_ context: ModelContext) throws {
        avenues = try Self.all(context)
        projects = try Self.all(context)
        tasks = try Self.all(context)
        labels = try Self.all(context)
        comments = try Self.all(context)
        reminders = try Self.all(context)
        history = try Self.all(context)
        journal = try Self.all(context)
    }

    /// Two devices can create the same record only by syncing it, so
    /// duplicate IDs don't happen; if they do, the first one fetched wins.
    private static func all<Model: PersistentModel & Identified>(_ context: ModelContext) throws -> [UUID: Model] {
        Dictionary(try context.fetch(FetchDescriptor<Model>()).map { ($0.id, $0) }) { first, _ in first }
    }
}

/// Sends each value to every stream made so far.
final class Broadcast<Value: Sendable>: Sendable {
    private let continuations = LockIsolated<[UUID: AsyncStream<Value>.Continuation]>([:])

    func stream() -> AsyncStream<Value> {
        let (stream, continuation) = AsyncStream<Value>.makeStream()
        let id = UUID()
        continuations.withValue { $0[id] = continuation }
        continuation.onTermination = { [continuations] _ in continuations.withValue { $0[id] = nil } }
        return stream
    }

    func yield(_ value: Value) {
        for continuation in continuations.value.values { continuation.yield(value) }
    }
}

/// A model with Lifr's own ID, and a way to create one.
protocol Identified {
    var id: UUID { get }
    init(id: UUID)
}

extension StoredAvenue: Identified {}
extension StoredProject: Identified {}
extension StoredTask: Identified {}
extension StoredLabel: Identified {}
extension StoredComment: Identified {}
extension StoredReminder: Identified {}
extension StoredHistoryEvent: Identified {}
extension StoredJournalEntry: Identified {}

/// Reading or writing the store failed. Core's rules never throw these; they
/// throw `LifrError`.
package enum LiveStoreError: Error, LocalizedError, CustomStringConvertible {
    case openFailed(any Error)
    case loadFailed(any Error)
    case saveFailed(any Error)
    case archiveFailed(any Error)

    package var description: String {
        switch self {
        case .openFailed(let error):
            "Lifr couldn't open its data on this device: \(error). Check that this device has free storage, "
                + "then reopen Lifr. Don't delete the app to fix this: that also deletes changes that haven't "
                + "synced to iCloud yet."
        case .loadFailed(let error):
            "Lifr couldn't read its data on this device: \(error). Quit and reopen Lifr. Don't delete the app "
                + "to fix this: that also deletes changes that haven't synced to iCloud yet."
        case .saveFailed(let error):
            "Lifr couldn't save the change, so nothing changed: \(error). Check that this device has free "
                + "storage, then try again."
        case .archiveFailed(let error):
            "Lifr couldn't pack changes older than 90 days to save room: \(error). Nothing was removed, and "
                + "they can still be undone; Lifr tries again the next time it opens. If this keeps happening, "
                + "check that this device has free storage."
        }
    }

    package var errorDescription: String? { description }
}
