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

/// Today, Upcoming, Procrastinated and Search on the iPhone.
@MainActor
final class PhoneTodayFlowTests: PhoneFlowTestCase {
    /// Today, finishing a task with Undo, then a task with open Sub tasks.
    func testToday() async throws {
        let (store, flow) = app("today")
        await flow.step("launch")
        let water = try task("Water the plants", in: store)
        store.send(.today(.list(.rowAction(water, .status(.done)))))
        await flow.step("finished-with-undo")
        let review = try task("Review Q4 roadmap", in: store)
        store.send(.today(.list(.rowAction(review, .status(.done)))))
        await flow.step("finish-with-open-subtasks")
        flow.finish()
    }

    /// Upcoming.
    func testUpcoming() async {
        let (store, flow) = app("upcoming")
        store.tab = .upcoming
        await flow.step("opened")
        flow.finish()
    }

    /// Upcoming as a calendar: the month over today's tasks, another day,
    /// and the next month.
    func testCalendar() async {
        let (store, flow) = app("calendar")
        store.tab = .upcoming
        store.send(.upcomingCalendarShown(true))
        await flow.step("opened")
        if let day = store.calendar.weeks.joined().first(where: { !$0.colors.isEmpty && !$0.isToday }) {
            store.send(.calendar(.daySelected(day.date)))
        }
        await flow.step("day-selected")
        store.send(.calendar(.monthStepped(1)))
        await flow.step("next-month")
        flow.finish()
    }

    /// Procrastinated, from Today.
    func testProcrastinated() async throws {
        let (store, flow) = app("procrastinated")
        await flow.step("launch", image: false)
        store.send(.today(.procrastinatedTapped))
        await flow.step("opened")
        flow.finish()
    }

    /// Search: words, then a due filter.
    func testSearch() async {
        let (store, flow) = app("search", tab: .search)
        await flow.step("empty")
        store.send(.search(.binding(.set(\.filter.text, "run"))))
        await flow.step("words")
        store.send(.search(.binding(.set(\.filter.due, .overdue))))
        await flow.step("overdue")
        flow.finish()
    }
}
