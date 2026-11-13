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
        let glance = TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: false)
        XCTAssertEqual(glance.count, 2)
        XCTAssertEqual(glance.overdue, 1)
        XCTAssertEqual(glance.rows, rows)
        XCTAssertFalse(glance.isLocked)
    }

    func testLockedKeepsOnlyTheCounts() {
        let glance = TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: true)
        XCTAssertEqual(glance.count, 2)
        XCTAssertEqual(glance.overdue, 1)
        XCTAssertEqual(glance.rows, [])
        XCTAssertTrue(glance.isLocked)
    }

    func testEachMomentGetsTheDayItFallsIn() {
        let midnight = now + 86_400
        let tomorrow = TodayGlance(asOf: now, from: midnight, until: midnight + 86_400, rows: [rows[0]], locked: false)
        let glance = TodayGlance(asOf: now, from: now, until: midnight, rows: rows, locked: false, later: [tomorrow])

        XCTAssertEqual(glance.at(now)?.count, 2)
        XCTAssertEqual(glance.at(midnight - 1)?.count, 2)
        XCTAssertEqual(glance.at(midnight)?.count, 1)
        XCTAssertEqual(glance.at(midnight)?.rows, [rows[0]])
        XCTAssertNil(glance.at(midnight + 86_400), "past the last day only the app knows")
        XCTAssertNil(glance.at(now - 1))
        XCTAssertEqual(glance.days.map(\.from), [now, midnight])
    }

    func testADaysOwnTasksLeaveOutThoseOverdueFromBefore() {
        let glance = TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: false)
        XCTAssertEqual(glance.dueRows, [rows[1]])
        XCTAssertEqual(glance.dueCount, 1)

        let locked = TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: true)
        XCTAssertEqual(locked.dueRows, [])
        XCTAssertEqual(locked.dueCount, 1, "the lock keeps how many")
    }

    func testTheDaysFromAMomentStartWithTheOneItFallsIn() {
        let midnight = now + 86_400
        let tomorrow = TodayGlance(asOf: now, from: midnight, until: midnight + 86_400, rows: [], locked: false)
        let glance = TodayGlance(asOf: now, from: now, until: midnight, rows: rows, locked: false, later: [tomorrow])

        XCTAssertEqual(glance.days(from: now).map(\.from), [now, midnight])
        XCTAssertEqual(glance.days(from: midnight + 1).map(\.from), [midnight])
        XCTAssertEqual(glance.days(from: midnight + 86_400), [])
    }

    func testAvenuesKeepOnlyCountsWhileLockedAndCarryOverMidnight() {
        let avenue = AvenueGlance(
            id: UUID(), name: "Home", symbol: "house", color: "teal", count: 2, overdue: 1, rows: rows)
        let midnight = now + 86_400
        let tomorrow = TodayGlance(asOf: now, from: midnight, until: midnight + 86_400, rows: [], locked: false)

        let glance = TodayGlance(
            asOf: now, from: now, until: midnight, rows: rows, locked: false, later: [tomorrow], avenues: [avenue])
        XCTAssertEqual(glance.at(midnight)?.avenues, [avenue])

        let locked = TodayGlance(asOf: now, from: now, until: midnight, rows: rows, locked: true, avenues: [avenue])
        XCTAssertEqual(locked.avenues?.map(\.count), [2])
        XCTAssertEqual(locked.avenues?.map(\.rows), [[]])
    }

    func testProjectsKeepOnlyCountsWhileLockedAndCarryOverMidnight() {
        let project = AvenueGlance(
            id: UUID(), name: "Move house", symbol: "folder.fill", color: "teal", count: 2, overdue: 1, rows: rows)
        let midnight = now + 86_400
        let tomorrow = TodayGlance(asOf: now, from: midnight, until: midnight + 86_400, rows: [], locked: false)

        let glance = TodayGlance(
            asOf: now, from: now, until: midnight, rows: rows, locked: false, later: [tomorrow], projects: [project])
        XCTAssertEqual(glance.at(midnight)?.projects, [project])

        let locked = TodayGlance(asOf: now, from: now, until: midnight, rows: rows, locked: true, projects: [project])
        XCTAssertEqual(locked.projects?.map(\.count), [2])
        XCTAssertEqual(locked.projects?.map(\.rows), [[]])
    }

    func testARowWrittenBeforeRowsHadDetailsReadsAsPlain() throws {
        let json = Data(
            #"""
            {"id": "00000000-0000-0000-0000-000000000001", "title": "Renew passport", "code": "ADMIN-0001",
             "isOverdue": true}
            """#.utf8)
        let row = try JSONDecoder().decode(TodayGlance.Row.self, from: json)
        XCTAssertEqual(row, rows[0])
        XCTAssertFalse(row.isInProgress)
        XCTAssertNil(row.dueMinutes)
    }

    func testTheFileReadsBackWhatWasWritten() throws {
        let file = GlanceFile(url: try folder().appendingPathComponent("Widgets/Today.json"))
        let tomorrow = TodayGlance(asOf: now, from: now + 86_400, until: now + 172_800, rows: [], locked: false)
        let glance = TodayGlance(
            asOf: now, from: now, until: now + 86_400, rows: rows, locked: false, later: [tomorrow])
        try file.write(glance)
        XCTAssertEqual(try file.read(), glance)
    }

    func testALockedFileHoldsNoTitles() throws {
        let file = GlanceFile(url: try folder().appendingPathComponent("Widgets/Today.json"))
        try file.write(TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: true))
        let text = try String(contentsOf: file.url, encoding: .utf8)
        XCTAssertFalse(text.contains("Renew passport"))
        XCTAssertFalse(text.contains("ADMIN-0001"))
    }

    func testOnlyThisUserCanReadTheFile() throws {
        let file = GlanceFile(url: try folder().appendingPathComponent("Widgets/Today.json"))
        try file.write(TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: false))
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
        await client.write(TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: false))
        XCTAssertEqual(try file.read()?.asOf, now)
        // The same Today later doesn't write again.
        await client.write(
            TodayGlance(asOf: now.addingTimeInterval(60), from: now, until: now + 86_400, rows: rows, locked: false))
        XCTAssertEqual(try file.read()?.asOf, now)
        // A change does.
        await client.write(
            TodayGlance(asOf: now.addingTimeInterval(120), from: now, until: now + 86_400, rows: rows, locked: true))
        XCTAssertEqual(try file.read()?.isLocked, true)
        XCTAssertEqual(try file.read()?.asOf, now.addingTimeInterval(120))
    }

    func testATapMarksATaskAndAnotherUnmarksIt() throws {
        let file = MarksFile(url: try folder().appendingPathComponent("Widgets/Marked.json"))
        XCTAssertEqual(try file.read(), [])
        XCTAssertTrue(try file.toggle(rows[0].id))
        XCTAssertTrue(try file.toggle(rows[1].id))
        XCTAssertFalse(try file.toggle(rows[1].id))
        XCTAssertEqual(try file.read(), [rows[0].id])
    }

    func testTakingTheMarksLeavesNone() async throws {
        let file = MarksFile(url: try folder().appendingPathComponent("Widgets/Marked.json"))
        try file.toggle(rows[0].id)
        let client = GlanceClient.live(file: nil, marks: file)
        let taken = try await client.takeMarked()
        XCTAssertEqual(taken, [rows[0].id])
        let again = try await client.takeMarked()
        XCTAssertEqual(again, [])
    }

    func testMarkedRowsStayButAreCountedNoMore() {
        let tomorrow = TodayGlance(asOf: now, from: now + 86_400, until: now + 2 * 86_400, rows: rows, locked: false)
        let glance = TodayGlance(
            asOf: now, from: now, until: now + 86_400, rows: rows, locked: false, later: [tomorrow])
        let marked = glance.marking([rows[0].id])
        XCTAssertEqual(marked.rows, rows)
        XCTAssertEqual(marked.count, 1)
        XCTAssertEqual(marked.overdue, 0)
        XCTAssertEqual(marked.later.first?.count, 1)
    }

    func testTheLiveClientWithNoFileDoesNothing() async {
        await GlanceClient.live(file: nil).write(
            TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: false))
    }
}
