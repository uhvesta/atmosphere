import BlobKit
import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import SwiftData
import XCTest

@testable import LifrStoreLive

final class LiveStoreTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func context(device: String = "Test", after seconds: TimeInterval = 0) -> @Sendable () -> Context {
        let now = now.addingTimeInterval(seconds)
        return {
            Context(actor: .user(device: device, surface: .mac), now: now)
        }
    }

    private func task(_ title: String, in workspace: Workspace) throws -> LifrTask {
        try XCTUnwrap(workspace.tasks.values.first { $0.title == title })
    }

    /// The records, with history in the order the store keeps it.
    private func sorted(_ workspace: Workspace) -> Workspace {
        Workspace(
            avenues: Array(workspace.avenues.values), projects: Array(workspace.projects.values),
            tasks: Array(workspace.tasks.values), labels: Array(workspace.labels.values),
            comments: Array(workspace.comments.values), reminders: Array(workspace.reminders.values),
            history: workspace.history.sorted { ($0.createdAt, $0.id.uuidString) < ($1.createdAt, $1.id.uuidString) },
            journal: workspace.journal, settings: workspace.settings)
    }

    func testEveryRecordReadsBackAsWritten() async throws {
        var workspace = Workspace.sample(now: now, timeZone: .gmt)
        let context = context()()
        let label = workspace.createLabel(name: "errand", in: context)
        let plants = try task("Water the plants", in: workspace)
        _ = try workspace.edit(plants.id, Workspace.TaskEdit(labelIDs: [label.id]), in: context)
        _ = try workspace.addComment("Use the **rain water**.", to: plants.id, in: context)
        let store = LiveStore(modelContainer: try LiveStore.inMemory())

        try await store.insert(workspace)

        let loaded = try await store.load()
        XCTAssertEqual(loaded, sorted(workspace))
    }

    func testAChangeIsStillThereAfterReopening() async throws {
        let container = try LiveStore.inMemory()
        try await LiveStore(modelContainer: container).insert(.sample(now: now, timeZone: .gmt))
        let client = withDependencies {
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            StoreClient.live(LiveStore(modelContainer: container), actor: .user(device: "Test", surface: .mac))
        }
        let plants = try task("Water the plants", in: try await client.load())

        let saved = try await withDependencies {
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            try await client.setStatus(.done, plants.id, false)
        }

        let reopened = try await LiveStore(modelContainer: container).load()
        XCTAssertEqual(reopened.tasks[plants.id]?.status, .done)
        XCTAssertEqual(reopened, sorted(saved.workspace))
    }

    func testAnAnnouncingClientsChangesReachTheOtherClientsUpdates() async throws {
        let store = LiveStore(modelContainer: try LiveStore.inMemory())
        try await store.insert(.sample(now: now, timeZone: .gmt))
        let app = StoreClient.live(store, actor: .user(device: "Test", surface: .mac))
        let shortcuts = StoreClient.live(store, actor: .user(device: "Test", surface: .shortcuts), announces: true)
        let plants = try task("Water the plants", in: try await app.load())
        let updates = await app.updates()

        _ = try await withDependencies {
            $0.date.now = now
            $0.uuid = .incrementing
        } operation: {
            try await shortcuts.setStatus(.done, plants.id, false)
        }

        var iterator = updates.makeAsyncIterator()
        let update = await iterator.next()
        XCTAssertEqual(update?.workspace.tasks[plants.id]?.status, .done)
        XCTAssertEqual(update?.fixes, [])
    }

    func testTheJournalIsKeptAndCanBeRevertedAfterReopening() async throws {
        let container = try LiveStore.inMemory()
        try await LiveStore(modelContainer: container).insert(.sample(now: now, timeZone: .gmt))
        let store = LiveStore(modelContainer: container)
        let plants = try task("Water the plants", in: try await store.load())
        let mac = Actor.user(device: "Test", surface: .mac)
        let now = now
        let saved = try await store.save(
            in: { Context(actor: mac, now: now) },
            { (workspace, context) throws(LifrError) in
                try workspace.edit(plants.id, .init(title: "Water the ferns", priority: .high), in: context)
            })

        let reopened = try await LiveStore(modelContainer: container).load()
        XCTAssertEqual(reopened.journal, saved.workspace.journal)
        XCTAssertEqual(reopened.journal.map(\.actor), [mac])

        let entry = try XCTUnwrap(saved.change.entry)
        let revert: StoreOperation = { (workspace, context) throws(LifrError) in
            try workspace.revert([entry], in: context)
        }
        let reverted = try await LiveStore(modelContainer: container).save(in: context(after: 60), revert)
        XCTAssertEqual(reverted.workspace.tasks[plants.id]?.title, "Water the plants")
        XCTAssertEqual(reverted.workspace.tasks[plants.id]?.priority, plants.priority)
        let journal = try await store.load().journal
        XCTAssertEqual(journal.last?.reverts, [entry])
    }

    func testAJournalWrittenAsPlainJSONStillLoads() async throws {
        let container = try LiveStore.inMemory()
        try await LiveStore(modelContainer: container).insert(.sample(now: now, timeZone: .gmt))
        let store = LiveStore(modelContainer: container)
        let plants = try task("Water the plants", in: try await store.load())
        let mac = Actor.user(device: "Test", surface: .mac)
        let now = now
        let saved = try await store.save(
            in: { Context(actor: mac, now: now) },
            { (workspace, context) throws(LifrError) in
                try workspace.edit(plants.id, .init(title: "Water the ferns"), in: context)
            })

        let context = ModelContext(container)
        for record in try context.fetch(FetchDescriptor<StoredJournalEntry>()) {
            let steps = try XCTUnwrap(record.steps)
            XCTAssertNotEqual(try Blob.format(of: steps), .legacy)
            record.steps = try JSONEncoder().encode(try Blob.decode([JournalStep].self, from: steps))
        }
        try context.save()

        let reopened = try await LiveStore(modelContainer: container).load()
        XCTAssertEqual(reopened.journal, saved.workspace.journal)
    }

    func testARefusedChangeSavesNothing() async throws {
        let store = LiveStore(modelContainer: try LiveStore.inMemory())
        try await store.insert(.sample(now: now, timeZone: .gmt))
        let before = try await store.load()
        let review = try task("Review Q4 roadmap", in: before)

        do {
            _ = try await store.save(in: context()) { (workspace, context) throws(LifrError) in
                try workspace.setStatus(.done, of: review.id, in: context)
            }
            XCTFail("Done with open Sub tasks should be refused")
        } catch let error as LifrError {
            guard case .openSubtasks = error else { return XCTFail("Unexpected error: \(error)") }
        }

        let after = try await store.load()
        XCTAssertEqual(after, before)
    }

    func testReconcilingReopensAParentThatSyncLeftClosedOverAnOpenSubTask() async throws {
        let container = try LiveStore.inMemory()
        let store = LiveStore(modelContainer: container)
        try await store.insert(.sample(now: now, timeZone: .gmt))
        let review = try task("Review Q4 roadmap", in: try await store.load())
        _ = try await store.save(in: context()) { (workspace, context) throws(LifrError) in
            try workspace.setStatus(.wontDo, of: review.id, cascade: true, in: context)
        }
        // Another device reopened a Sub task while offline; sync writes it
        // as it is.
        let imported = ModelContext(container)
        let subtask = try XCTUnwrap(
            try imported.fetch(FetchDescriptor<StoredTask>()).first { $0.title == "Send summary to the team" })
        subtask.status = TaskStatus.todo.rawValue
        try imported.save()

        let fixes = try await store.reconcile(in: context(device: "Mac", after: 60))

        XCTAssertEqual(fixes, [.reopenedParent(task: review.id)])
        let after = try await LiveStore(modelContainer: container).load()
        XCTAssertEqual(after.tasks[review.id]?.status, .inProgress)
        let fix = try XCTUnwrap(after.history.last)
        XCTAssertEqual(fix.taskID, review.id)
        XCTAssertEqual(fix.actor.kind, .system)
        XCTAssertEqual(fix.reason, .reconciledClosedParent)
    }

    func testReconcilingAgainChangesNothing() async throws {
        let store = LiveStore(modelContainer: try LiveStore.inMemory())
        try await store.insert(.sample(now: now, timeZone: .gmt))
        let before = try await store.load()

        let fixes = try await store.reconcile(in: context())

        XCTAssertEqual(fixes, [])
        let after = try await store.load()
        XCTAssertEqual(after, before)
    }

    func testATaskThatSyncedBeforeItsAvenueAppearsOnceTheAvenueArrives() async throws {
        let container = try LiveStore.inMemory()
        let store = LiveStore(modelContainer: container)
        let imported = ModelContext(container)
        let task = StoredTask(id: UUID())
        task.code = "7K2Q"
        task.title = "Arrived first"
        task.status = TaskStatus.todo.rawValue
        task.priority = Priority.none.rawValue
        imported.insert(task)
        try imported.save()

        let early = try await store.load()
        XCTAssertEqual(early.tasks, [:])

        let avenue = StoredAvenue(id: UUID())
        avenue.key = "HOME"
        avenue.name = "Home"
        avenue.agentAccess = AgentAccess.full.rawValue
        imported.insert(avenue)
        task.avenue = avenue
        try imported.save()

        let later = try await store.load()
        XCTAssertEqual(later.tasks[task.id]?.title, "Arrived first")
        XCTAssertEqual(later.displayCode(of: try XCTUnwrap(later.tasks[task.id])), "HOME-7K2Q")
    }
}
