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
        XCTAssertEqual(scheduled.last?.body, "HEALTH-0002 · due 2026-11-04 at 07:00")
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
        _ = try? workspace.moveToTrash(trashed.id, in: Context(actor: .user(device: "Test"), now: now))

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
