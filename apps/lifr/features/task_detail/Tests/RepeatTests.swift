import ComposableArchitecture
import Foundation
import LifrCore
import LifrStoreClient
import LifrTaskDetail
import XCTest

@MainActor
final class RepeatTests: XCTestCase {
    /// Tuesday, 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func makeStore(_ task: LifrTask) throws -> TestStoreOf<TaskDetailFeature> {
        let client = StoreClient.inMemory(records.workspace)
        let state = try XCTUnwrap(
            TaskDetailFeature.State(id: task.id, workspace: records.workspace, now: now, timeZone: .gmt))
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

    func testTheUsualRepeatsAreReadFromTheDueDay() throws {
        let presets = RepeatPreset.presets(for: try CalendarDate("2026-11-11"))
        XCTAssertTrue(presets.contains(RepeatPreset(rule: .weekly(weekdays: [4]), interval: 1)))
        XCTAssertTrue(presets.contains(RepeatPreset(rule: .monthly(day: 11), interval: 1)))
        XCTAssertTrue(presets.contains(RepeatPreset(rule: .monthlyWeekday(ordinal: 2, weekday: 4), interval: 1)))
        XCTAssertTrue(presets.contains(RepeatPreset(rule: .yearly(month: 11, day: 11), interval: 1)))
    }

    func testPickingARepeatSavesItAndNeverStopsIt() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.repeatPicked(RepeatPreset(rule: .daily, interval: 1)))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.recurrence?.rule, .daily)

        await store.send(.repeatPicked(nil))
        await store.receive(\.delegate.saved)
        XCTAssertNil(store.state.recurrence)
    }

    func testACustomRepeatIsSavedOnlyWhenItCanHappen() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.repeatCustomTapped)
        XCTAssertEqual(store.state.repeatDraft?.weekdays, [3])
        var draft = try XCTUnwrap(store.state.repeatDraft)
        draft.weekdays = []
        await store.send(.binding(.set(\.repeatDraft, draft)))
        await store.send(.repeatCustomSaved)
        XCTAssertNotNil(store.state.repeatDraft)

        draft.kind = .afterClosing
        draft.unit = .day
        draft.interval = 3
        await store.send(.binding(.set(\.repeatDraft, draft)))
        await store.send(.repeatCustomSaved)
        await store.receive(\.delegate.saved)
        XCTAssertNil(store.state.repeatDraft)
        XCTAssertEqual(store.state.recurrence?.rule, .afterClosing(.day))
        XCTAssertEqual(store.state.recurrence?.interval, 3)
    }

    func testCancellingACustomRepeatChangesNothing() async throws {
        let stretch = records.task("Stretch", in: health)
        let store = try makeStore(stretch)

        await store.send(.repeatCustomTapped)
        await store.send(.repeatCustomCancelled)
        XCTAssertNil(store.state.repeatDraft)
        XCTAssertNil(store.state.recurrence)
    }

    func testADraftReadsBackTheRepeatItEdits() throws {
        let anchor = try CalendarDate("2026-11-11")
        let recurrence = Recurrence(
            rule: .monthlyWeekday(ordinal: 2, weekday: 4), interval: 2, seriesID: UUID())
        let draft = RepeatDraft(anchor: anchor, from: recurrence)
        XCTAssertEqual(draft.rule, recurrence.rule)
        XCTAssertEqual(draft.interval, 2)
    }
}
