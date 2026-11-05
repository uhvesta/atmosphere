import CloudKit
import Foundation
import LifrCore
import LifrSyncClient
import XCTest

@testable import LifrSyncLive

final class CloudRecordTests: XCTestCase {
    private let item = CloudItem.record(RecordID(.task, UUID()), nil)

    func testRecordRoundTrips() throws {
        let deviceID = UUID()
        let record = try CloudRecord.record(item, device: "Mac", deviceID: deviceID, systemFields: nil)
        XCTAssertEqual(record.recordID, CloudRecord.id(item.name))
        XCTAssertEqual(record.recordType, CloudRecord.type)

        let (change, sender) = try CloudRecord.change(record)
        XCTAssertEqual(change, CloudChange(item, from: "Mac"))
        XCTAssertEqual(sender, deviceID)
    }

    func testRecordKeepsItsSystemFields() throws {
        let first = try CloudRecord.record(item, device: "Mac", deviceID: UUID(), systemFields: nil)
        let fields = CloudRecord.systemFields(of: first)
        let again = try CloudRecord.record(item, device: "Mac", deviceID: UUID(), systemFields: fields)
        XCTAssertEqual(again.recordID, first.recordID)
        XCTAssertEqual(again.creationDate, first.creationDate)
    }

    func testRecordWithNothingInItIsRejected() {
        let record = CKRecord(recordType: CloudRecord.type, recordID: CloudRecord.id(item.name))
        XCTAssertThrowsError(try CloudRecord.change(record)) { error in
            guard case .noPayload = error as? CloudRecordError else { return XCTFail("\(error)") }
        }
    }

    func testRecordHoldingAnotherItemIsRejected() throws {
        let record = try CloudRecord.record(item, device: "Mac", deviceID: UUID(), systemFields: nil)
        let other = CKRecord(recordType: CloudRecord.type, recordID: CloudRecord.id("task/other"))
        other.encryptedValues["payload"] = record.encryptedValues["payload"]
        XCTAssertThrowsError(try CloudRecord.change(other)) { error in
            guard case .misnamed = error as? CloudRecordError else { return XCTFail("\(error)") }
        }
    }

    func testUnreadablePayloadIsRejected() {
        let record = CKRecord(recordType: CloudRecord.type, recordID: CloudRecord.id(item.name))
        record.encryptedValues["payload"] = Data("not json".utf8)
        XCTAssertThrowsError(try CloudRecord.change(record)) { error in
            guard case .decodeFailed = error as? CloudRecordError else { return XCTFail("\(error)") }
        }
    }

    func testCloudKitErrorsMapToWhatThePersonCanDo() {
        XCTAssertEqual(SyncError(CKError(.notAuthenticated)), .noAccount)
        XCTAssertEqual(SyncError(CKError(.quotaExceeded)), .quotaExceeded)
        XCTAssertEqual(SyncError(CKError(.missingEntitlement)), .notSetUp)
        guard case .unreachable = SyncError(CKError(.networkUnavailable)) else {
            return XCTFail("A network failure should be unreachable")
        }
        guard case .refused = SyncError(CKError(.permissionFailure)) else {
            return XCTFail("A permission failure should be refused")
        }
        guard case .refused = SyncError(CocoaError(.fileWriteUnknown)) else {
            return XCTFail("A non-CloudKit error should be refused")
        }
    }
}

final class SyncFilesTests: XCTestCase {
    private var folder: URL!

    override func setUpWithError() throws {
        folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: folder)
    }

    func testNothingSavedLoadsEmpty() throws {
        let files = try SyncFiles(folder: folder)
        XCTAssertEqual(try files.ledger.load(), SyncLedger())
        XCTAssertTrue(try files.engine.load().systemFields.isEmpty)
    }

    func testLedgerAndEngineRoundTrip() throws {
        let files = try SyncFiles(folder: folder)
        var ledger = SyncLedger()
        ledger.lastSynced = Date(timeIntervalSince1970: 1_793_700_000)
        ledger.sent = [UUID()]
        try files.ledger.save(ledger)

        var engine = EngineFile()
        engine.systemFields = ["task/1": Data([1, 2, 3])]
        try files.engine.save(engine)

        let reopened = try SyncFiles(folder: folder)
        XCTAssertEqual(try reopened.ledger.load(), ledger)
        let loaded = try reopened.engine.load()
        XCTAssertEqual(loaded.deviceID, engine.deviceID)
        XCTAssertEqual(loaded.systemFields, engine.systemFields)
    }

    func testFilesAreReadableOnlyByLifr() throws {
        let files = try SyncFiles(folder: folder)
        try files.ledger.save(SyncLedger())
        let attributes = try FileManager.default.attributesOfItem(atPath: folder.appending(path: "Ledger.json").path)
        XCTAssertEqual(attributes[.posixPermissions] as? Int, 0o600)
        let folderAttributes = try FileManager.default.attributesOfItem(atPath: folder.path)
        XCTAssertEqual(folderAttributes[.posixPermissions] as? Int, 0o700)
    }

    func testDamagedFileFailsInsteadOfStartingOver() throws {
        let files = try SyncFiles(folder: folder)
        try Data("{".utf8).write(to: folder.appending(path: "Ledger.json"))
        XCTAssertThrowsError(try files.ledger.load()) { error in
            guard case .ledgerFailed = error as? SyncError else { return XCTFail("\(error)") }
        }
    }

    func testClientWithoutAContainerSaysItIsNotSetUp() async {
        let client = SyncClient.live(container: "", store: .init(), device: "Mac")
        let state = await client.sync()
        XCTAssertEqual(state.status, .failed(.notSetUp))
    }
}
