import FlowKit
import Foundation
import LifrGlance
import LifrWidgetViews
import SnapshotTesting
import SwiftUI
import XCTest

/// The Today widget in each size, as the Home Screen draws it.
@MainActor
final class WidgetSnapshotTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)

    private var rows: [TodayGlance.Row] {
        ["Renew passport", "Water the plants", "Call the dentist", "Pay rent", "Book flights"].enumerated().map {
            TodayGlance.Row(
                id: UUID(uuidString: String(format: "00000000-0000-0000-0000-%012d", $0.offset)) ?? UUID(),
                title: $0.element, code: String(format: "HOME-%04d", $0.offset + 1), isOverdue: $0.offset == 0)
        }
    }

    /// The widget sizes on iPhone 17.
    private func size(_ size: TodayWidgetView.Size) -> CGSize {
        switch size {
        case .small: CGSize(width: 170, height: 170)
        case .medium: CGSize(width: 364, height: 170)
        case .large: CGSize(width: 364, height: 382)
        }
    }

    private func widget(_ glance: TodayGlance?, _ size: TodayWidgetView.Size) -> some View {
        TodayWidgetView(glance: glance, size: size)
            .padding(16)
            .frame(width: self.size(size).width, height: self.size(size).height)
            .background(Color(white: 0.95))
    }

    private func assertWidget(
        _ glance: TodayGlance?, _ size: TodayWidgetView.Size, named name: String, file: StaticString = #filePath,
        testName: String = #function, line: UInt = #line
    ) {
        assertSnapshot(
            of: widget(glance, size),
            as: .image(layout: .fixed(width: self.size(size).width, height: self.size(size).height)),
            named: name, file: file, testName: testName, line: line)
        assertSnapshot(
            of: widget(glance, size), as: .accessibilityTree(size: self.size(size)), named: name, file: file,
            testName: testName, line: line)
    }

    func testToday() {
        let glance = TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: false)
        assertWidget(glance, .small, named: "small")
        assertWidget(glance, .medium, named: "medium")
        assertWidget(glance, .large, named: "large")
    }

    func testLocked() {
        let glance = TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: true)
        assertWidget(glance, .small, named: "small")
        assertWidget(glance, .medium, named: "medium")
    }

    func testNothingDue() {
        assertWidget(
            TodayGlance(asOf: now, from: now, until: now + 86_400, rows: [], locked: false), .medium, named: "medium")
    }

    /// A week from `now`: overdue work on today, nothing on Saturday's
    /// stand-in, and a few things on the rest.
    private func week(locked: Bool) -> [TodayGlance] {
        (0..<7).map { day in
            let start = now + TimeInterval(day) * 86_400
            let due: [TodayGlance.Row] =
                day == 3
                ? []
                : rows.dropFirst(day % 3 + 1).map { row in
                    var row = row
                    row.isOverdue = false
                    return row
                }
            let overdue: [TodayGlance.Row] = day == 0 ? [rows[0]] : []
            return TodayGlance(asOf: now, from: start, until: start + 86_400, rows: overdue + due, locked: locked)
        }
    }

    private func assertUpcoming(_ days: [TodayGlance]?, _ size: TodayWidgetView.Size, named name: String) {
        let view = UpcomingWidgetView(days: days, size: size)
            .padding(16)
            .frame(width: self.size(size).width, height: self.size(size).height)
            .background(Color(white: 0.95))
        assertSnapshot(
            of: view, as: .image(layout: .fixed(width: self.size(size).width, height: self.size(size).height)),
            named: name, testName: "testUpcoming")
        assertSnapshot(of: view, as: .accessibilityTree(size: self.size(size)), named: name, testName: "testUpcoming")
    }

    func testUpcoming() {
        assertUpcoming(week(locked: false), .small, named: "small")
        assertUpcoming(week(locked: false), .medium, named: "medium")
        assertUpcoming(week(locked: false), .large, named: "large")
        assertUpcoming(week(locked: true), .large, named: "locked")
        assertUpcoming(
            week(locked: false).map { TodayGlance(asOf: now, from: $0.from, until: $0.until, rows: [], locked: false) },
            .medium, named: "nothing-due")
        assertUpcoming(nil, .small, named: "before-lifr-wrote")
    }

    private func avenues(locked: Bool) -> [AvenueGlance] {
        [("Home", "house.fill", "teal", 4, 1), ("Health", "heart.fill", "pink", 2, 0),
         ("Work", "briefcase.fill", "indigo", 0, 0), ("Admin", "doc.text.fill", "orange", 7, 0)]
            .enumerated().map { index, avenue in
                AvenueGlance(
                    id: UUID(uuidString: String(format: "10000000-0000-0000-0000-%012d", index)) ?? UUID(),
                    name: avenue.0, symbol: avenue.1, color: avenue.2, count: avenue.3, overdue: avenue.4,
                    rows: locked ? [] : Array(rows.prefix(avenue.3)))
            }
    }

    private func assertAvenues(_ avenues: [AvenueGlance]?, _ size: TodayWidgetView.Size, named name: String) {
        let view = AvenuesWidgetView(avenues: avenues, size: size)
            .padding(16)
            .frame(width: self.size(size).width, height: self.size(size).height)
            .background(Color(white: 0.95))
        assertSnapshot(
            of: view, as: .image(layout: .fixed(width: self.size(size).width, height: self.size(size).height)),
            named: name, testName: "testAvenues")
        assertSnapshot(of: view, as: .accessibilityTree(size: self.size(size)), named: name, testName: "testAvenues")
    }

    func testAvenues() {
        assertAvenues(avenues(locked: false), .small, named: "small")
        assertAvenues(avenues(locked: false), .medium, named: "medium")
        assertAvenues(avenues(locked: false), .large, named: "large")
        assertAvenues(avenues(locked: true), .large, named: "locked")
        assertAvenues([], .small, named: "none")
        assertAvenues(nil, .medium, named: "before-lifr-wrote")
    }

    func testBeforeLifrWroteToday() {
        assertWidget(nil, .small, named: "small")
        assertLockScreen(nil, named: "lock-screen")
    }

    func testLockScreen() {
        assertLockScreen(
            TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: false), named: "today")
        assertLockScreen(
            TodayGlance(asOf: now, from: now, until: now + 86_400, rows: rows, locked: true), named: "locked")
        assertLockScreen(
            TodayGlance(asOf: now, from: now, until: now + 86_400, rows: [], locked: false), named: "nothing-due")
    }

    /// The three Lock Screen shapes, as on iPhone 17, one above the other.
    private func assertLockScreen(
        _ glance: TodayGlance?, named name: String, file: StaticString = #filePath, testName: String = #function,
        line: UInt = #line
    ) {
        let size = CGSize(width: 220, height: 200)
        let view = VStack(alignment: .leading, spacing: 12) {
            TodayAccessoryView(glance: glance, shape: .circular).frame(width: 72, height: 72)
            TodayAccessoryView(glance: glance, shape: .rectangular).frame(width: 172, height: 76)
            TodayAccessoryView(glance: glance, shape: .inline).frame(width: 200, height: 20, alignment: .leading)
        }
        .padding(8)
        .frame(width: size.width, height: size.height, alignment: .topLeading)
        .foregroundStyle(.white)
        .background(Color(white: 0.2))
        assertSnapshot(
            of: view, as: .image(layout: .fixed(width: size.width, height: size.height)), named: name, file: file,
            testName: testName, line: line)
        assertSnapshot(
            of: view, as: .accessibilityTree(size: size), named: name, file: file, testName: testName, line: line)
    }
}
