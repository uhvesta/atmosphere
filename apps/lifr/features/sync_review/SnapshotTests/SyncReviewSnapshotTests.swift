import ComposableArchitecture
import LifrCore
import LifrSnapshotTesting
import LifrSyncClient
import LifrSyncReview
import SnapshotTesting
import SwiftUI
import XCTest

/// The review sheet over the sample review, as of 2026-11-03 10:00 UTC: a
/// task retitled and moved on both devices, and one trashed on the Mac.
@MainActor
final class SyncReviewSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private func state() -> SyncReviewFeature.State {
        let sample = Review.sample(now: now, timeZone: .gmt)
        return SyncReviewFeature.State(items: sample.items, workspace: sample.workspace, thisDevice: "This iPhone")
    }

    private func chosen() -> SyncReviewFeature.State {
        var state = state()
        state.choices = [state.items[0].id: [.avenue: .theirs], state.items[1].id: [.deletedAt: .theirs]]
        return state
    }

    private func failed() -> SyncReviewFeature.State {
        var state = state()
        state.error = SyncError.storeFailed("disk full").description
        return state
    }

    private func screen(_ state: SyncReviewFeature.State) -> some View {
        NavigationStack {
            SyncReviewView(store: Store(initialState: state) { EmptyReducer() })
        }
        .environment(\.timeZone, .gmt)
    }

    func testDisplayData() {
        assertSnapshot(of: state().rows, as: .dump, named: "review")
    }

    func testAccessibilityTree() {
        let size = CGSize(width: 402, height: 874)
        assertSnapshot(of: screen(state()), as: .accessibilityTree(size: size), named: "review")
        assertSnapshot(of: screen(chosen()), as: .accessibilityTree(size: size), named: "chosen")
        assertSnapshot(of: screen(failed()), as: .accessibilityTree(size: size), named: "failed")
    }

    func testImages() {
        assertScreenImages(of: screen(state()), named: "review")
        assertScreenImages(of: screen(chosen()), named: "chosen")
        assertScreenImages(of: screen(failed()), named: "failed")
    }
}
