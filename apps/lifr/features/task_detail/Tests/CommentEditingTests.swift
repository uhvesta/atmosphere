import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import XCTest

@MainActor
final class CommentEditingTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    /// "Buy running shoes" with the person's comment "Trail" and an agent's
    /// comment "Road" on it.
    private func shoes() throws -> TestStoreOf<TaskDetailFeature> {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let health = records.avenue("HEALTH", "Health")
        let task = records.task("Buy running shoes", in: health)
        records.comment("Trail", on: task)
        var workspace = records.workspace
        let agent = Context(actor: .agent("Claude Code", device: "Mac", surface: .mcp), now: now + 30)
        _ = try workspace.addComment("Road", to: task.id, in: agent)
        let state = try XCTUnwrap(TaskDetailFeature.State(id: task.id, workspace: workspace, now: now, timeZone: .gmt))
        let store = TestStore(initialState: state) {
            TaskDetailFeature()
        } withDependencies: { [now, workspace] in
            $0.store = .inMemory(workspace)
            $0.date.now = now + 60
            $0.timeZone = .gmt
            $0.uuid = UUIDGenerator { UUID() }
        }
        store.exhaustivity = .off
        return store
    }

    private func comment(_ text: String, in store: TestStoreOf<TaskDetailFeature>) throws -> TimelineEntry {
        try XCTUnwrap(store.state.timeline.first { $0.isComment && $0.text == text })
    }

    func testOnlyThePersonsOwnCommentsCanBeChanged() async throws {
        let store = try shoes()
        XCTAssertTrue(try comment("Trail", in: store).isMine)
        XCTAssertFalse(try comment("Road", in: store).isMine)

        await store.send(.commentEditTapped(try comment("Road", in: store).id))
        XCTAssertNil(store.state.editingCommentID)
        await store.send(.commentDeleteTapped(try comment("Road", in: store).id))
        XCTAssertNil(store.state.commentDeletion)
    }

    func testEditingACommentSavesTheNewTextAndMarksItEdited() async throws {
        let store = try shoes()
        let trail = try comment("Trail", in: store)

        await store.send(.commentEditTapped(trail.id))
        XCTAssertEqual(store.state.editedComment, "Trail")
        await store.send(.binding(.set(\.editedComment, "Trail, size 44 ")))
        await store.send(.commentEditSaved)
        await store.receive(\.delegate.saved)

        XCTAssertNil(store.state.editingCommentID)
        let edited = try XCTUnwrap(store.state.timeline.first { $0.id == trail.id })
        XCTAssertEqual(edited.text, "Trail, size 44")
        XCTAssertTrue(edited.isEdited)
    }

    func testAnEmptyEditIsntSavedAndCancelKeepsTheComment() async throws {
        let store = try shoes()
        let trail = try comment("Trail", in: store)

        await store.send(.commentEditTapped(trail.id))
        await store.send(.binding(.set(\.editedComment, "  ")))
        await store.send(.commentEditSaved)
        XCTAssertEqual(store.state.editingCommentID, trail.id)
        await store.send(.commentEditCancelled)

        XCTAssertNil(store.state.editingCommentID)
        XCTAssertEqual(try comment("Trail", in: store).isEdited, false)
    }

    func testDeletingACommentAsksFirstThenRemovesIt() async throws {
        let store = try shoes()
        let trail = try comment("Trail", in: store)

        await store.send(.commentDeleteTapped(trail.id))
        XCTAssertNotNil(store.state.commentDeletion)
        await store.send(.commentDeletion(.dismiss))
        XCTAssertTrue(store.state.timeline.contains { $0.id == trail.id })

        await store.send(.commentDeleteTapped(trail.id))
        await store.send(.commentDeletion(.presented(trail.id)))
        await store.receive(\.delegate.saved)
        XCTAssertFalse(store.state.timeline.contains { $0.id == trail.id })
        XCTAssertEqual(store.state.timeline.filter(\.isComment).map(\.text), ["Road"])
    }
}
