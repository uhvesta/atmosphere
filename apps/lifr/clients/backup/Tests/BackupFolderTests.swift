import Foundation
import LifrBackupClient
import LifrCore
import XCTest

final class BackupFolderTests: XCTestCase {
    private var folder = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)

    override func tearDown() {
        try? FileManager.default.removeItem(at: folder)
    }

    private func day(_ day: Int) throws -> CalendarDate {
        try CalendarDate(year: 2026, month: 11, day: day)
    }

    func testKeepsTheLastSevenDaysNewestFirst() async throws {
        let backups = BackupClient.inFolder { [folder] in folder }
        let export = Workspace().exported(at: Date(timeIntervalSince1970: 1_793_664_000))
        for day in 1...9 {
            try await backups.save(export, try self.day(day))
        }
        let kept = await backups.list().map(\.day)
        XCTAssertEqual(kept, try (3...9).reversed().map(day))
    }

    func testSavingTwiceInADayKeepsOneBackupOfTheLater() async throws {
        let backups = BackupClient.inFolder { [folder] in folder }
        let first = Workspace().exported(at: Date(timeIntervalSince1970: 1_793_664_000))
        let later = Workspace().exported(at: Date(timeIntervalSince1970: 1_793_700_000))
        try await backups.save(first, try day(3))
        try await backups.save(later, try day(3))
        let kept = await backups.list().map(\.day)
        XCTAssertEqual(kept, [try day(3)])
        let data = try await backups.read(try day(3))
        XCTAssertEqual(try LifrExport.decoded(from: data), later)
    }

    func testABackupOnlyLifrCanRead() async throws {
        let backups = BackupClient.inFolder { [folder] in folder }
        try await backups.save(Workspace().exported(at: .now), try day(3))
        let name = try XCTUnwrap(FileManager.default.contentsOfDirectory(atPath: folder.path).first)
        let file = folder.appending(path: name)
        let permissions = try FileManager.default.attributesOfItem(atPath: file.path)[.posixPermissions] as? Int
        XCTAssertEqual(permissions, 0o600)
    }

    func testReadingADayWithNoBackupFails() async throws {
        let backups = BackupClient.inFolder { [folder] in folder }
        do {
            _ = try await backups.read(try day(3))
            XCTFail("read a backup that isn't there")
        } catch {}
    }
}
