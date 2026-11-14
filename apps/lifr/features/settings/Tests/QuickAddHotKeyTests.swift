import LifrSettings
import XCTest

final class QuickAddHotKeyTests: XCTestCase {
    func testRecordedKeysReadBackAsStored() {
        let keys = QuickAddHotKey(keyCode: 40, modifiers: [.control, .command], keyName: "K")
        XCTAssertEqual(QuickAddHotKey(rawValue: keys.rawValue), keys)
        XCTAssertEqual(QuickAddHotKey(rawValue: QuickAddHotKey.off.rawValue), .off)
    }

    func testKeysPickedBeforeRecordingStillRead() {
        XCTAssertEqual(QuickAddHotKey(rawValue: "controlOptionSpace"), .controlOptionSpace)
        XCTAssertEqual(QuickAddHotKey(rawValue: "off"), .off)
        XCTAssertNil(QuickAddHotKey(rawValue: "nonsense"))
    }

    func testAKeyNeedsControlOptionOrCommandToOpenQuickAdd() {
        XCTAssertFalse(QuickAddHotKey(keyCode: 40, modifiers: [], keyName: "K").isUsable)
        XCTAssertFalse(QuickAddHotKey(keyCode: 40, modifiers: .shift, keyName: "K").isUsable)
        XCTAssertTrue(QuickAddHotKey(keyCode: 40, modifiers: [.shift, .option], keyName: "K").isUsable)
        XCTAssertFalse(QuickAddHotKey.off.isUsable)
    }
}
