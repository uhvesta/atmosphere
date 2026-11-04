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

    private func makeStore(_ task: LifrTask) -> TestStoreOf<TaskDetailFeature> {
        let client = StoreClient.inMemory(records.workspace)
        let state = TaskDetailFeature.State(id: task.id, workspace: records.workspace, now: now, timeZone: .gmt)!
        let store = TestStore(initialState: state) {
            TaskDetailFeature()
        } withDependencies: { [now] in
            // A minute passes between changes, as it would for a person.
            let clock = LockIsolated(now)
            $0.store = client
            $0.date = DateGenerator {
                clock.withValue {
                    $0 += 60
                    return $0
                }
            }
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
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
        XCTAssertNotNil(store.state.prompt)
        XCTAssertEqual(store.state.row.status, .todo)

        await store.send(.prompt(.presented(.dropWithSubtasks(task: review.id))))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .wontDo)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.wontDo])
    }

    func testDoneWithOpenSubtasksAsksThenFinishesDroppingThem() async {
        let review = records.task("Review", in: health)
        records.task("Read", in: health, parent: review)
        records.task("Reply", in: health, parent: review)
        let store = makeStore(review)

        await store.send(.statusPicked(.done))
        await store.receive(\.saved)
        XCTAssertNotNil(store.state.prompt)
        XCTAssertEqual(store.state.row.status, .todo)

        await store.send(.prompt(.presented(.finishDroppingSubtasks(task: review.id))))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .done)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.wontDo, .wontDo])
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
        XCTAssertNotNil(store.state.banner?.undo)

        await store.send(.bannerUndoTapped)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.status, .done)
        XCTAssertEqual(store.state.subtasks.map(\.status), [.done])
        XCTAssertNil(store.state.banner)
    }

    func testAddingASubtaskToADoneTaskReopensItUntilUndone() async {
        let shoes = records.task("Buy running shoes", in: health, status: .done)
        let store = makeStore(shoes)

        await store.send(.binding(.set(\.newSubtaskTitle, "Buy socks")))
        await store.send(.addSubtaskSubmitted)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.subtasks.map(\.title), ["Buy socks"])
        XCTAssertEqual(store.state.row.status, .inProgress)
        XCTAssertEqual(store.state.newSubtaskTitle, "")

        await store.send(.bannerUndoTapped)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.subtasks, [])
        XCTAssertEqual(store.state.row.status, .done)
    }

    func testABlankSubtaskTitleAddsNothing() async {
        let shoes = records.task("Buy running shoes", in: health)
        let store = makeStore(shoes)

        await store.send(.binding(.set(\.newSubtaskTitle, "   ")))
        await store.send(.addSubtaskSubmitted)

        XCTAssertEqual(store.state.subtasks, [])
    }

    func testACommentShowsInTheTimelineAndTheCommentsFilter() async {
        let shoes = records.task("Buy running shoes", in: health)
        let store = makeStore(shoes)

        await store.send(.binding(.set(\.newComment, "Trail **and** road")))
        await store.send(.commentSubmitted)
        await store.receive(\.delegate.saved)
        await store.send(.statusPicked(.inProgress))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(
            store.state.timeline.map(\.text), ["To Do -> In Progress", "Trail **and** road", "Created"])
        XCTAssertEqual(store.state.timeline.first?.author.hasPrefix("You on"), true)
        await store.send(.binding(.set(\.showsCommentsOnly, true)))
        XCTAssertEqual(store.state.visibleTimeline.map(\.text), ["Trail **and** road"])
    }

    func testPickingAPriorityRecordsIt() async {
        let shoes = records.task("Buy running shoes", in: health)
        let store = makeStore(shoes)

        await store.send(.priorityPicked(.urgent))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.row.priority, .urgent)
    }
}
