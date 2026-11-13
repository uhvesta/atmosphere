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
}
