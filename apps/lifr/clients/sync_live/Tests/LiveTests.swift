import Foundation
import LifrCore
import LifrStoreClient
import LifrSyncClient
import XCTest

@testable import LifrSyncLive

final class LiveTests: XCTestCase {
    func testClientWithoutAContainerSaysItIsNotSetUp() async {
        let client = SyncClient.live(container: "", store: .init(), device: "Mac")
        let state = await client.sync()
        XCTAssertEqual(state.status, .failed(.notSetUp))
    }

    // Items other devices saved before Lifr synced through SyncKit.

    func testRemovedRecordSavedBeforeReadsBack() throws {
        let id = try XCTUnwrap(UUID(uuidString: "00000000-0000-0000-0000-000000000001"))
        let json = #"{"record":{"_0":{"id":"00000000-0000-0000-0000-000000000001","kind":"task"}}}"#
        let item = try JSONDecoder().decode(CloudItem.self, from: Data(json.utf8))
        XCTAssertEqual(item, .record(RecordID(.task, id), nil))
        XCTAssertEqual(item.name, "task/00000000-0000-0000-0000-000000000001")
    }

    func testJournalEntryRoundTripsAsBefore() throws {
        let entry = JournalEntry(
            id: UUID(), actor: .user(device: "Mac", surface: .mac), at: Date(timeIntervalSince1970: 1_793_700_000),
            steps: [])
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        let data = try encoder.encode(CloudItem.log(.entry(entry)))
        let json = try XCTUnwrap(String(bytes: data, encoding: .utf8))
        XCTAssertTrue(json.hasPrefix(#"{"entry":{"_0":"#))
        XCTAssertEqual(try JSONDecoder().decode(CloudItem.self, from: data), .log(.entry(entry)))
    }
}
