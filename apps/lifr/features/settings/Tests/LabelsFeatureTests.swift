import ComposableArchitecture
import Foundation
import LifrCore
import LifrSettings
import LifrStoreClient
import XCTest

@MainActor
final class LabelsFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    /// Labels "Errand" on two tasks and "Phone" on one, with the editor
    /// open on "Errand".
    private func editing() -> (TestStoreOf<LabelsFeature>, errand: Label, phone: Label) {
        var records = RecordsBuilder(now: now, timeZone: .gmt)
        let home = records.avenue("HOME", "Home")
        let errand = records.label("Errand")
        let phone = records.label("Phone")
        _ = records.task("Post the parcel", in: home, labels: [errand])
        _ = records.task("Buy stamps", in: home, labels: [errand, phone])
        let workspace = records.workspace
        let store = TestStore(initialState: LabelsFeature.State(workspace: workspace)) {
            LabelsFeature()
        } withDependencies: { [now] in
            $0.store = .inMemory(workspace)
            $0.date.now = now
            $0.uuid = UUIDGenerator { UUID() }
        }
        store.exhaustivity = .off
        return (store, errand, phone)
    }

    func testListsLabelsByNameWithHowManyTasksHaveThem() {
        let (store, errand, phone) = editing()
        XCTAssertEqual(store.state.rows.map(\.id), [errand.id, phone.id])
        XCTAssertEqual(store.state.rows.map(\.taskCount), [2, 1])
    }

    func testRenamingALabelKeepsItOnItsTasks() async {
        let (store, errand, _) = editing()
        await store.send(.rowTapped(errand.id))
        XCTAssertEqual(store.state.edit?.others.count, 1)
        await store.send(.edit(.presented(.binding(.set(\.name, "  Errands ")))))
        await store.send(.edit(.presented(.saveTapped)))
        await store.receive(\.delegate.saved)
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.edit)
        XCTAssertEqual(store.state.rows.first { $0.id == errand.id }?.name, "Errands")
        XCTAssertEqual(store.state.rows.first { $0.id == errand.id }?.taskCount, 2)
    }

    func testRecoloringALabelKeepsItsNameAndTasks() async throws {
        let (store, errand, _) = editing()
        await store.send(.rowTapped(errand.id))
        await store.send(.edit(.presented(.binding(.set(\.color, "green")))))
        await store.send(.edit(.presented(.saveTapped)))
        await store.receive(\.delegate.saved)
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.edit)
        let row = try XCTUnwrap(store.state.rows.first { $0.id == errand.id })
        XCTAssertEqual(row.color, "green")
        XCTAssertEqual(row.name, "Errand")
        XCTAssertEqual(row.taskCount, 2)
    }

    func testARefusedNameKeepsTheOldColorToo() async {
        let (store, errand, _) = editing()
        let color = store.state.rows.first { $0.id == errand.id }?.color
        await store.send(.rowTapped(errand.id))
        await store.send(.edit(.presented(.binding(.set(\.name, "phone")))))
        await store.send(.edit(.presented(.binding(.set(\.color, "green")))))
        await store.send(.edit(.presented(.saveTapped)))
        await store.skipReceivedActions(strict: false)
        XCTAssertNotNil(store.state.edit?.error)
        XCTAssertEqual(store.state.rows.first { $0.id == errand.id }?.color, color)
    }

    func testANameAnotherLabelHasIsRefusedAndSaysWhy() async {
        let (store, errand, _) = editing()
        await store.send(.rowTapped(errand.id))
        await store.send(.edit(.presented(.binding(.set(\.name, "phone")))))
        await store.send(.edit(.presented(.saveTapped)))
        await store.skipReceivedActions(strict: false)
        XCTAssertNotNil(store.state.edit?.error)
        XCTAssertEqual(store.state.rows.map(\.name), ["Errand", "Phone"])

        // Typing again clears the reason.
        await store.send(.edit(.presented(.binding(.set(\.name, "Out")))))
        XCTAssertNil(store.state.edit?.error)
    }

    func testMergingMovesItsTasksToTheOtherLabelAfterAsking() async {
        let (store, errand, phone) = editing()
        await store.send(.rowTapped(errand.id))
        await store.send(.edit(.presented(.mergeTapped(phone.id))))
        XCTAssertNotNil(store.state.edit?.confirm)
        await store.send(.edit(.presented(.confirm(.presented(.merge(phone.id))))))
        await store.receive(\.delegate.saved)
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.edit)
        XCTAssertEqual(store.state.rows.map(\.id), [phone.id])
        XCTAssertEqual(store.state.rows.first?.taskCount, 2)
    }

    func testDeletingTakesItOffEveryTaskAfterAsking() async {
        let (store, errand, phone) = editing()
        await store.send(.rowTapped(errand.id))
        await store.send(.edit(.presented(.deleteTapped)))
        XCTAssertNotNil(store.state.edit?.confirm)
        await store.send(.edit(.presented(.confirm(.presented(.delete)))))
        await store.receive(\.delegate.saved)
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.rows.map(\.id), [phone.id])
        XCTAssertEqual(store.state.rows.first?.taskCount, 1)
    }

    func testCancellingTheQuestionChangesNothing() async {
        let (store, errand, _) = editing()
        await store.send(.rowTapped(errand.id))
        await store.send(.edit(.presented(.deleteTapped)))
        await store.send(.edit(.presented(.confirm(.dismiss))))
        await store.send(.edit(.presented(.cancelTapped)))
        await store.skipReceivedActions(strict: false)
        XCTAssertNil(store.state.edit)
        XCTAssertEqual(store.state.rows.map(\.name), ["Errand", "Phone"])
    }
}
