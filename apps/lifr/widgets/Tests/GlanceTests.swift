import Foundation
import LifrGlance
import LifrGlanceClient
import XCTest

final class GlanceTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private let rows = [
        TodayGlance.Row(
            id: UUID(uuidString: "00000000-0000-0000-0000-000000000001") ?? UUID(), title: "Renew passport",
            code: "ADMIN-0001", isOverdue: true),
        TodayGlance.Row(
            id: UUID(uuidString: "00000000-0000-0000-0000-000000000002") ?? UUID(), title: "Water the plants",
            code: "HOME-0002", isOverdue: false),
    ]

    private func folder() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        addTeardownBlock { try? FileManager.default.removeItem(at: url) }
        return url
    }

    func testCountsTodayAndWhatsOverdue() {
        let glance = TodayGlance(asOf: now, rows: rows, locked: false)
        XCTAssertEqual(glance.count, 2)
        XCTAssertEqual(glance.overdue, 1)
        XCTAssertEqual(glance.rows, rows)
        XCTAssertFalse(glance.isLocked)
    }

    func testLockedKeepsOnlyTheCounts() {
        let glance = TodayGlance(asOf: now, rows: rows, locked: true)
        XCTAssertEqual(glance.count, 2)
        XCTAssertEqual(glance.overdue, 1)
        XCTAssertEqual(glance.rows, [])
        XCTAssertTrue(glance.isLocked)
    }

    func testTheFileReadsBackWhatWasWritten() throws {
        let file = GlanceFile(url: try folder().appendingPathComponent("Widgets/Today.json"))
        let glance = TodayGlance(asOf: now, rows: rows, locked: false)
        try file.write(glance)
        XCTAssertEqual(try file.read(), glance)
    }

    func testALockedFileHoldsNoTitles() throws {
        let file = GlanceFile(url: try folder().appendingPathComponent("Widgets/Today.json"))
        try file.write(TodayGlance(asOf: now, rows: rows, locked: true))
        let text = try String(contentsOf: file.url, encoding: .utf8)
        XCTAssertFalse(text.contains("Renew passport"))
        XCTAssertFalse(text.contains("ADMIN-0001"))
    }

    func testOnlyThisUserCanReadTheFile() throws {
        let file = GlanceFile(url: try folder().appendingPathComponent("Widgets/Today.json"))
        try file.write(TodayGlance(asOf: now, rows: rows, locked: false))
        let attributes = try FileManager.default.attributesOfItem(atPath: file.url.path)
        XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, 0o600)
    }

    func testNothingWrittenYetReadsAsNothing() throws {
        let file = GlanceFile(url: try folder().appendingPathComponent("Widgets/Today.json"))
        XCTAssertNil(try file.read())
    }

    func testAFileThatIsntTodayFailsToRead() throws {
        let url = try folder().appendingPathComponent("Today.json")
        try FileManager.default.createDirectory(
            at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data("not json".utf8).write(to: url)
        do {
            _ = try GlanceFile(url: url).read()
            XCTFail("Expected it to fail")
        } catch {
            guard case .readFailed = error else { return XCTFail("Expected readFailed, got \(error)") }
        }
    }

    func testTheLiveClientWritesTodayAndSkipsWhatsUnchanged() async throws {
        let file = GlanceFile(url: try folder().appendingPathComponent("Widgets/Today.json"))
        let client = GlanceClient.live(file: file)
        await client.write(TodayGlance(asOf: now, rows: rows, locked: false))
        XCTAssertEqual(try file.read()?.asOf, now)
        // The same Today later doesn't write again.
        await client.write(TodayGlance(asOf: now.addingTimeInterval(60), rows: rows, locked: false))
        XCTAssertEqual(try file.read()?.asOf, now)
        // A change does.
        await client.write(TodayGlance(asOf: now.addingTimeInterval(120), rows: rows, locked: true))
        XCTAssertEqual(try file.read()?.isLocked, true)
        XCTAssertEqual(try file.read()?.asOf, now.addingTimeInterval(120))
    }

    func testTheLiveClientWithNoFileDoesNothing() async {
        await GlanceClient.live(file: nil).write(TodayGlance(asOf: now, rows: rows, locked: false))
    }
}
