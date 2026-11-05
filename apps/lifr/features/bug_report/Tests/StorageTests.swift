import CryptoKit
import Foundation
import XCTest

@testable import LifrBugReport

final class StorageTests: XCTestCase {
    private let folder = FileManager.default.temporaryDirectory.appending(
        path: "bug-reports-\(UUID())", directoryHint: .isDirectory)
    private let key = SymmetricKey(size: .bits256)

    override func tearDown() async throws {
        try? FileManager.default.removeItem(at: folder)
    }

    private func client(key: SymmetricKey? = nil) -> BugReportsClient {
        let key = key ?? self.key
        return .onDisk(folder: folder) { key }
    }

    func testSavedReportsComeBackNewestFirst() async throws {
        let old = BugReport.sample(includesPersonal: true, id: UUID(1), at: Date(timeIntervalSince1970: 1))
        let new = BugReport.sample(includesPersonal: false, id: UUID(2), at: Date(timeIntervalSince1970: 2))

        try await client().save(old)
        try await client().save(new)

        let saved = try await client().load()
        XCTAssertEqual(saved.reports, [new, old])
        XCTAssertEqual(saved.unreadable, [])
    }

    func testNothingSavedLoadsNoReports() async throws {
        let left = try await client().load()
        XCTAssertEqual(left, SavedReports())
    }

    func testTheFilesAreEncryptedAndOnlyLifrCanReadThem() async throws {
        try await client().save(.sample(includesPersonal: true, id: UUID(1)))

        let files = try FileManager.default.contentsOfDirectory(at: folder, includingPropertiesForKeys: nil)
        XCTAssertEqual(files.count, 1)
        let data = try Data(contentsOf: try XCTUnwrap(files.first))
        XCTAssertNil(data.range(of: Data("The task vanished".utf8)))
        let file = try FileManager.default.attributesOfItem(atPath: try XCTUnwrap(files.first).path)
        XCTAssertEqual(file[.posixPermissions] as? Int, 0o600)
        let directory = try FileManager.default.attributesOfItem(atPath: folder.path)
        XCTAssertEqual(directory[.posixPermissions] as? Int, 0o700)
        XCTAssertEqual(try folder.resourceValues(forKeys: [.isExcludedFromBackupKey]).isExcludedFromBackup, true)
    }

    func testAReportUnderAnotherKeyIsListedAsUnreadableAndCanBeDeleted() async throws {
        try await client(key: SymmetricKey(size: .bits256)).save(.sample(includesPersonal: true, id: UUID(1)))

        let saved = try await client().load()
        XCTAssertEqual(saved.reports, [])
        XCTAssertEqual(saved.unreadable.map(\.id), [UUID(1)])

        try await client().delete(UUID(1))
        let left = try await client().load()
        XCTAssertEqual(left, SavedReports())
    }

    func testDeletingAReportThatIsGoneSucceeds() async throws {
        try await client().delete(UUID(9))
    }

    func testAKeyThatCannotBeHadIsTheError() async throws {
        let client = BugReportsClient.onDisk(folder: folder) { () throws(BugReportError) in
            throw .keyUnavailable(status: -25308, reason: "interaction not allowed")
        }

        do {
            try await client.save(.sample(includesPersonal: true))
            XCTFail("saved without a key")
        } catch let error as BugReportError {
            XCTAssertEqual(error, .keyUnavailable(status: -25308, reason: "interaction not allowed"))
        }
    }
}
