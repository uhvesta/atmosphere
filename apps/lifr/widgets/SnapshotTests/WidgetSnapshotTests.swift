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

    func testBeforeLifrWroteToday() {
        assertWidget(nil, .small, named: "small")
    }
}
