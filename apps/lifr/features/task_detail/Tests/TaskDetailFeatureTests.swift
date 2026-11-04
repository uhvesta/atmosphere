import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import XCTest

@MainActor
final class TaskDetailFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")
    /// Every change the store made, so tests can undo one.
    private let changes = LockIsolated<[Change]>([])

    private func makeStore(_ task: LifrTask) -> TestStoreOf<TaskDetailFeature> {
        var client = StoreClient.inMemory(records.workspace)
        let setStatus = client.setStatus
        client.setStatus = { [changes] status, id, cascade in
            let saved = try await setStatus(status, id, cascade)
            changes.withValue { $0.append(saved.change) }
            return saved
        }
        let state = TaskDetailFeature.State(id: task.id, workspace: records.workspace, now: now, timeZone: .gmt)!
        let store = TestStore(initialState: state) {
            TaskDetailFeature()
        } withDependencies: { [now] in
            $0.store = client
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testPickingAStatusSavesItAndTellsTheList() async {
        let stretch = records.task("Stretch", in: health)
        let store = makeStore(stretch)

        await store.send(.statusPicked(.inProgress))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .inProgress)
    }

    func testWontDoWithOpenSubtasksAsksThenClosesThemToo() async {
        let review = records.task("Review", in: health)
        records.task("Read", in: health, parent: review)
        let store = makeStore(review)

        await store.send(.statusPicked(.wontDo))
        await store.receive(\.saved)
        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(store.state.row.status, .todo)

        await store.send(.alert(.presented(.closeSubtasksToo)))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .wontDo)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.wontDo])
    }

    func testDoneWithOpenSubtasksIsRefused() async {
        let review = records.task("Review", in: health)
        records.task("Read", in: health, parent: review)
        let store = makeStore(review)

        await store.send(.statusPicked(.done))
        await store.receive(\.saved)

        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(store.state.row.status, .todo)
    }

    func testReopeningASubtaskReopensTheParentWithUndo() async {
        let review = records.task("Review", in: health)
        let read = records.task("Read", in: health, parent: review, status: .done)
        records.setStatus(.done, of: review)
        let store = makeStore(review)

        await store.send(.subtaskToggled(read.id))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.row.status, .inProgress)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.todo])
        XCTAssertNotNil(store.state.alert)

        await store.send(.alert(.presented(.undo(changes.value.last!))))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .done)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.done])
    }
}
