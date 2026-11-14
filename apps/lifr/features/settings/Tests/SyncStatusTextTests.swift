import LifrSettings
import LifrSyncClient
import XCTest

final class SyncStatusTextTests: XCTestCase {
    func testOfflineSaysHowManyChangesAreWaiting() {
        let none = SyncStatusText.status(SyncState(status: .failed(.unreachable("offline"))))
        let three = SyncStatusText.status(SyncState(status: .failed(.unreachable("offline")), pending: 3))
        XCTAssertFalse(none.contains("3"))
        XCTAssertTrue(three.contains("3"))
    }

    func testChangesNotYetSentSayHowManyAreWaiting() {
        let last = Date(timeIntervalSince1970: 1_793_700_000)
        let none = SyncStatusText.status(SyncState(lastSynced: last), now: last + 600)
        let three = SyncStatusText.status(SyncState(lastSynced: last, pending: 3), now: last + 600)
        XCTAssertFalse(none.contains("3"))
        XCTAssertTrue(three.contains("3"))
    }

    func testALastSyncReadsAsHowLongAgoItWas() {
        let last = Date(timeIntervalSince1970: 1_793_700_000)
        let state = SyncState(lastSynced: last)
        let soon = SyncStatusText.status(state, now: last + 10)
        XCTAssertEqual(SyncStatusText.status(state, now: last + 40), soon, "under a minute reads the same")
        let later = SyncStatusText.status(state, now: last + 7_200)
        XCTAssertNotEqual(later, soon)
        XCTAssertNotEqual(SyncStatusText.status(state, now: last + 600), later)
    }

    func testOnlyAFailureWithICloudSignedInAsksForThePerson() {
        XCTAssertFalse(SyncStatusText.needsAttention(SyncState()))
        XCTAssertFalse(SyncStatusText.needsAttention(SyncState(status: .syncing)))
        XCTAssertFalse(SyncStatusText.needsAttention(SyncState(status: .failed(.notSetUp))))
        XCTAssertFalse(SyncStatusText.needsAttention(SyncState(status: .failed(.noAccount))))
        XCTAssertTrue(SyncStatusText.needsAttention(SyncState(status: .failed(.unreachable("offline")))))
    }

    func testBeingOfflineNeedsNoFixInSettings() {
        XCTAssertFalse(SyncStatusText.needsFix(SyncState(status: .failed(.unreachable("offline")))))
        XCTAssertFalse(SyncStatusText.needsFix(SyncState(status: .failed(.notSetUp))))
        XCTAssertTrue(SyncStatusText.needsFix(SyncState(status: .failed(.quotaExceeded))))
        XCTAssertNil(SyncStatusText.reason(SyncState()))
        XCTAssertNotNil(SyncStatusText.reason(SyncState(status: .failed(.quotaExceeded))))
    }
}
