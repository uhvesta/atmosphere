import GhosttyKit
import XCTest

final class GhosttyKitTests: XCTestCase {
    /// The library links into the app's tests and answers a call.
    func testTheLibraryReportsItsVersion() throws {
        let info = ghostty_info()
        let version = try XCTUnwrap(info.version)
        XCTAssertGreaterThan(info.version_len, 0)
        XCTAssertFalse(String(cString: version).isEmpty)
        XCTAssertEqual(info.build_mode, GHOSTTY_BUILD_MODE_RELEASE_FAST)
    }
}
