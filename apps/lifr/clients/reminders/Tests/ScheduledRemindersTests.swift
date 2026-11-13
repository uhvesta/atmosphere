import ComposableArchitecture
import Foundation
import LifrCore
import LifrRemindersClient
import LifrStoreClient
import XCTest

final class ScheduledRemindersTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private var scheduled: [ScheduledReminder] {
        records.workspace.scheduledReminders(now: now, timeZone: .gmt)
    }

    func testSchedulesUpcomingRemindersSoonestFirst() {
        let run = records.task("Long run", in: health, due: 1, at: "07:00")
        let physio = records.task("Book physio", in: health)
        records.reminder(.beforeDue(seconds: 3600), on: run)
        records.reminder(.at(now + 600), on: physio)

        XCTAssertEqual(scheduled.map(\.title), ["Book physio", "Long run"])
        XCTAssertEqual(scheduled.map(\.fireAt), [now + 600, ISO8601DateFormatter().date(from: "2026-11-04T06:00:00Z")])
        let english = Locale(identifier: "en_US")
        let body = records.workspace.scheduledReminders(now: now, timeZone: .gmt, locale: english).last?.body ?? ""
        XCTAssertTrue(body.contains("HEALTH-0002"), body)
        XCTAssertTrue(body.contains("Nov 4"), body)
        XCTAssertTrue(body.contains("7:00"), body)
        XCTAssertFalse(body.contains("2026-11-04"), body)
    }

    func testTheDailySummaryCountsEachDayFromTheNextTimeAndSkipsEmptyDays() {
        records.task("Pay rent", in: health, due: -1)
        records.task("Long run", in: health, due: 1, at: "07:00")
        records.task("Stretch", in: health, due: 1)
        records.task("Book physio", in: health, due: 3)
        let done = records.task("Done tomorrow", in: health, due: 1)
        records.setStatus(.done, of: done)
        let summaries = records.workspace.scheduledReminders(
            now: now, timeZone: .gmt, locale: Locale(identifier: "en_US"), summaryAt: try? TimeOfDay("08:00")
        )
        .filter(\.isSummary)

        // Today's 08:00 has passed; each later day has the overdue rent.
        let format = ISO8601DateFormatter()
        XCTAssertEqual(summaries.count, DailySummary.days)
        XCTAssertEqual(summaries.first?.fireAt, format.date(from: "2026-11-04T08:00:00Z"))
        XCTAssertEqual(Set(summaries.map(\.id)).count, summaries.count)
        XCTAssertTrue(summaries.first?.body.contains("Stretch") == true)
        XCTAssertTrue(summaries.first?.body.contains("Long run") == true)
        XCTAssertFalse(summaries.first?.body.contains("Done tomorrow") == true)

        let locked = records.workspace.scheduledReminders(
            now: now, timeZone: .gmt, hidingTitles: true, summaryAt: try? TimeOfDay("08:00"))
        XCTAssertFalse(locked.contains { $0.body.contains("Stretch") })
    }

    func testTheDailySummaryIsOffUnlessATimeIsSet() {
        records.task("Pay rent", in: health, due: 1)
        XCTAssertFalse(scheduled.contains(where: \.isSummary))
        XCTAssertNil(DailySummary(rawValue: "").time)
        let raw = DailySummary(time: DailySummary.morning).rawValue
        XCTAssertEqual(DailySummary(rawValue: raw).time, DailySummary.morning)
    }

    func testTheBadgeCountsOverdueAndDueTodayOrOnlyOverdue() {
        records.task("Due yesterday", in: health, due: -1)
        records.task("Due today", in: health, due: 0)
        records.task("Due tomorrow", in: health, due: 1)
        let done = records.task("Done yesterday", in: health, due: -1)
        records.setStatus(.done, of: done)

        XCTAssertEqual(records.workspace.badgeCount(.today, now: now, timeZone: .gmt), 2)
        XCTAssertEqual(records.workspace.badgeCount(.overdue, now: now, timeZone: .gmt), 1)
        XCTAssertEqual(records.workspace.badgeCount(.off, now: now, timeZone: .gmt), 0)
    }

    func testAHiddenTitleIsMarkedSoItOffersOnlySnooze() {
        let run = records.task("Long run", in: health)
        records.reminder(.at(now + 60), on: run)
        let hidden = records.workspace.scheduledReminders(now: now, timeZone: .gmt, hidingTitles: true)
        XCTAssertEqual(hidden.map(\.hidesTitle), [true])
        XCTAssertEqual(scheduled.map(\.hidesTitle), [false])
    }

    func testSkipsPastClosedAndTrashedTasks() {
        let past = records.task("Past", in: health)
        records.reminder(.at(now - 60), on: past)
        let done = records.task("Done", in: health)
        records.reminder(.at(now + 60), on: done)
        records.setStatus(.done, of: done)
        let trashed = records.task("Trashed", in: health)
        records.reminder(.at(now + 60), on: trashed)
        var workspace = records.workspace
        _ = try? workspace.moveToTrash(
            trashed.id, in: Context(actor: .user(device: "Test", surface: .iPhone), now: now))

        XCTAssertEqual(workspace.scheduledReminders(now: now, timeZone: .gmt), [])
    }

    func testKeepsTheSoonestWithinTheSystemLimit() {
        let task = records.task("Busy", in: health)
        for minute in (1...70).reversed() {
            records.reminder(.at(now + Double(minute) * 60), on: task)
        }

        XCTAssertEqual(scheduled.count, Workspace.reminderLimit)
        XCTAssertEqual(scheduled.last?.fireAt, now + 64 * 60)
    }

    func testInMemoryAsksOnceAndRemembersADenial() async throws {
        let allowed = RemindersClient.inMemory(permission: .notDetermined)
        let granted = try await allowed.client.requestPermission()
        XCTAssertTrue(granted)
        XCTAssertEqual(allowed.permission, .allowed)

        let denied = RemindersClient.inMemory(permission: .denied)
        let deniedGranted = try await denied.client.requestPermission()
        XCTAssertFalse(deniedGranted)
    }
}
