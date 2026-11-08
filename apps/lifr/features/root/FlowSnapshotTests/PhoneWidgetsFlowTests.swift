import ComposableArchitecture
import FlowKit
import LifrAPI
import LifrAssistantClient
import LifrAssistantLive
import LifrBugReport
import LifrConversations
import LifrCore
import LifrGlance
import LifrGlanceClient
import LifrLockClient
import LifrModelsClient
import LifrRemindersClient
import LifrRoot
import LifrSearchClient
import LifrSessionClient
import LifrShortcuts
import LifrStoreClient
import LifrSyncClient
import LifrWidgetViews
import SwiftUI
import XCTest

/// Shortcuts and the widgets on the iPhone.
@MainActor
final class PhoneWidgetsFlowTests: PhoneFlowTestCase {
    /// Shortcuts and Siri while the app is open: a task added there shows on
    /// Today, one finished there is ticked off, Activity names Shortcuts,
    /// and with the lock on, Siri says how many but not what.
    func testShortcuts() async throws {
        let (store, flow) = app("shortcuts")
        let added = try await shortcut { try await $0.add("Buy oat milk", avenue: nil, due: .today) }
        XCTAssertEqual(added.name, "Buy oat milk")
        await flow.step("added")
        let water = try task("Water the plants", in: store)
        _ = try await shortcut { try await $0.finish(water) }
        await flow.step("finished")
        store.isSettingsShown = true
        store.send(.settings(.activityTapped))
        await flow.step("activity")
        store.send(.settings(.activity(.dismiss)))
        store.isSettingsShown = false

        @Shared(.locksApp) var locksApp
        $locksApp.withLock { $0 = true }
        let today = try await shortcut { try await $0.today() }
        let summary = TodaySummary(today).text
        XCTAssertGreaterThan(today.count, 0)
        XCTAssertTrue(today.titles.isEmpty)
        XCTAssertFalse(summary.contains("Buy oat milk"))
        do {
            _ = try await shortcut { try await $0.finish(water) }
            XCTFail("Expected the lock to stop it")
        } catch {
            XCTAssertEqual(error as? ShortcutError, .locked)
        }
        flow.finish()
    }

    /// The widgets follow Today as it changes in the app and from Shortcuts,
    /// show only how many while the lock is on, and turn over at midnight
    /// with the app closed.
    func testWidgets() async throws {
        let (store, flow) = app("widgets")
        let home = FlowRecorder("widgets-home", WidgetBoardView(board: widgets), size: CGSize(width: 402, height: 874))
        await flow.step("today", image: false)
        await home.step("today")
        _ = try await shortcut { try await $0.add("Buy oat milk", avenue: nil, due: .today) }
        await flow.step("added-by-shortcut", image: false)
        await home.step("added-by-shortcut")
        store.send(.settings(.lockToggled(true)))
        await home.step("locked")
        XCTAssertEqual(widgets.glance?.rows, [])
        store.send(.settings(.lockToggled(false)))
        await home.step("unlocked")
        let midnight = try XCTUnwrap(widgets.glance?.until)
        widgets.date = midnight
        await home.step("after-midnight")
        let milk = try XCTUnwrap(widgets.glance?.at(midnight)?.rows.first { $0.title == "Buy oat milk" })
        XCTAssertTrue(milk.isOverdue)
        home.finish()
        flow.finish()
    }
}

/// The Today widget, small and medium, as they'd sit on the Home Screen.
private struct WidgetBoardView: View {
    var board: WidgetBoard

    var body: some View {
        VStack(alignment: .leading, spacing: 20) {
            HStack(spacing: 20) {
                widget(.small, width: 170)
                Spacer(minLength: 0)
            }
            widget(.medium, width: 364)
            Spacer(minLength: 0)
        }
        .padding(19)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Color(white: 0.85))
    }

    private func widget(_ size: TodayWidgetView.Size, width: CGFloat) -> some View {
        TodayWidgetView(glance: board.date.map { board.glance?.at($0) } ?? board.glance, size: size)
            .padding(16)
            .frame(width: width, height: 170)
            .background(Color.white, in: RoundedRectangle(cornerRadius: 22))
    }
}
