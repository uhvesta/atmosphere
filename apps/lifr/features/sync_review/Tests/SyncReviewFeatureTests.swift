import ComposableArchitecture
import Foundation
import LifrCore
import LifrSyncClient
import LifrSyncReview
import XCTest

@MainActor
final class SyncReviewFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func state() -> SyncReviewFeature.State {
        let sample = Review.sample(now: now, timeZone: .gmt)
        return SyncReviewFeature.State(items: sample.items, workspace: sample.workspace, thisDevice: "This iPhone")
    }

    func testTheRowsSayWhatEachDeviceHas() throws {
        let rows = state().rows
        XCTAssertEqual(rows.count, 2)
        guard case .fields(let questions) = rows[0].kind else { return XCTFail("Expected fields") }
        XCTAssertEqual(questions.map(\.field), [.title, .avenue])
        XCTAssertEqual(questions[0].mine, "Buy race gels and salt")
        XCTAssertEqual(questions[0].theirs, "Buy energy gels")
        XCTAssertEqual(rows[0].device, "Mac")
        XCTAssertEqual(rows[1].kind, .removed(by: .theirs))
    }

    func testTheRowsSayWhenEachDeviceChangedATask() {
        let row = state().rows[0]
        XCTAssertEqual(row.mineChanged, now.addingTimeInterval(-26 * 60 * 60))
        XCTAssertEqual(row.theirsChanged, now.addingTimeInterval(-2 * 60 * 60))
    }

    func testDoneWithNoChoicesKeepsThisDevicesValues() async throws {
        let resolved = LockIsolated<Review.Choices?>(nil)
        let initial = state()
        let store = TestStore(initialState: initial) {
            SyncReviewFeature()
        } withDependencies: {
            $0.sync.resolve = { choices in
                resolved.setValue(choices)
                return SyncState()
            }
        }
        store.exhaustivity = .off
        await store.send(.doneTapped)
        await store.receive(\.resolved)
        await store.receive(\.delegate.finished)
        let gels = initial.items[0].id
        let bag = initial.items[1].id
        XCTAssertEqual(resolved.value, [gels: [.title: .mine, .avenue: .mine], bag: [.deletedAt: .mine]])
    }

    func testChoicesArePassedOnPerField() async throws {
        let resolved = LockIsolated<Review.Choices?>(nil)
        let initial = state()
        let gels = initial.items[0].id
        let bag = initial.items[1].id
        let store = TestStore(initialState: initial) {
            SyncReviewFeature()
        } withDependencies: {
            $0.sync.resolve = { choices in
                resolved.setValue(choices)
                return SyncState()
            }
        }
        store.exhaustivity = .off
        await store.send(.chose(gels, .avenue, .theirs))
        await store.send(.chose(bag, .deletedAt, .theirs))
        await store.send(.doneTapped)
        await store.receive(\.resolved)
        XCTAssertEqual(resolved.value, [gels: [.title: .mine, .avenue: .theirs], bag: [.deletedAt: .theirs]])
    }

    func testLaterResolvesNothing() async {
        let store = TestStore(initialState: state()) { SyncReviewFeature() }
        store.exhaustivity = .off
        await store.send(.laterTapped)
        await store.receive(\.delegate.finished)
    }

    func testAFailedResolveKeepsTheSheetAndSaysWhy() async {
        let initial = state()
        let store = TestStore(initialState: initial) {
            SyncReviewFeature()
        } withDependencies: {
            $0.sync.resolve = { _ in SyncState(status: .failed(.storeFailed("disk full")), review: initial.items) }
        }
        store.exhaustivity = .off
        await store.send(.doneTapped)
        await store.receive(\.resolved) {
            $0.isResolving = false
            $0.error = SyncError.storeFailed("disk full").description
        }
        XCTAssertEqual(store.state.items, initial.items)
    }

    func testConflictsThatArriveWhileChoosingReplaceTheResolvedOnes() async {
        let initial = state()
        let later = Array(initial.items.suffix(1))
        let store = TestStore(initialState: initial) {
            SyncReviewFeature()
        } withDependencies: {
            $0.sync.resolve = { _ in SyncState(review: later) }
        }
        store.exhaustivity = .off
        await store.send(.chose(initial.items[0].id, .title, .theirs))
        await store.send(.doneTapped)
        await store.receive(\.resolved) {
            $0.items = later
            $0.choices = [:]
        }
    }

    func testTheSheetClosesWhenAnotherSyncLeavesNothingToReview() async {
        let initial = state()
        let store = TestStore(initialState: initial) { SyncReviewFeature() }
        store.exhaustivity = .off
        await store.send(.show([], initial.workspace))
        await store.receive(\.delegate.finished)
    }
}
