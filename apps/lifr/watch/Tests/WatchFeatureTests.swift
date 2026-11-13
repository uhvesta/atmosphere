import ComposableArchitecture
import Foundation
import LifrGlance
import LifrRemindersClient
import LifrWatchFeature
import LifrWatchLink
import XCTest

@MainActor
final class WatchFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private let water = UUID()
    private let run = UUID()

    private func glance(asOf: Date, rows: [UUID]? = nil, locked: Bool = false) -> TodayGlance {
        let start = Date(timeIntervalSince1970: 1_793_664_000)
        let all = [
            TodayGlance.Row(id: run, title: "Long run", code: "HEALTH-0004", isOverdue: true),
            TodayGlance.Row(id: water, title: "Water the plants", code: "HOME-0010", isOverdue: false),
        ]
        return TodayGlance(
            asOf: asOf, from: start, until: start + 86_400,
            rows: rows.map { ids in all.filter { ids.contains($0.id) } } ?? all, locked: locked)
    }

    private func makeStore(
        _ client: WatchClient, state: WatchFeature.State = WatchFeature.State()
    ) -> TestStoreOf<WatchFeature> {
        let store = TestStore(initialState: state) {
            WatchFeature()
        } withDependencies: { [now] in
            $0.watch = client
            $0.date.now = now
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    func testOpeningShowsTheKeptTodayThenEachOneTheIPhoneSends() async {
        let kept = glance(asOf: now - 60)
        let sent = glance(asOf: now, rows: [water])
        let store = makeStore(WatchClient.inMemory(saved: kept, glances: [sent]).client)
        await store.send(.task)
        XCTAssertEqual(store.state.glance, kept)
        await store.receive(\.glanceReceived)
        XCTAssertEqual(store.state.glance, sent)
    }

    func testTappingATaskOnlyShowsItsStatusesAndSendsNothing() async {
        let (client, sent) = WatchClient.inMemory()
        let store = makeStore(client, state: WatchFeature.State(glance: glance(asOf: now - 60)))
        await store.send(.rowTapped(water))
        XCTAssertEqual(store.state.chosen?.title, "Water the plants")
        await store.send(.choiceDismissed)
        XCTAssertNil(store.state.chosen)
        XCTAssertEqual(sent.value, [])
    }

    func testAPickedStatusGoesToTheIPhoneAndShowsUntilATodayWrittenAfterIt() async {
        let (client, sent) = WatchClient.inMemory()
        let store = makeStore(client, state: WatchFeature.State(glance: glance(asOf: now - 60)))
        await store.send(.rowTapped(water))
        await store.send(.statusPicked(.done))
        await store.finish()
        XCTAssertEqual(sent.value, [.setStatus(water, .done)])
        XCTAssertEqual(store.state.sent[water]?.status, .done)

        // One the iPhone wrote before saving it leaves the mark.
        await store.send(.glanceReceived(glance(asOf: now - 1)))
        XCTAssertEqual(store.state.sent[water]?.status, .done)
        await store.send(.glanceReceived(glance(asOf: now + 1, rows: [run])))
        XCTAssertNil(store.state.sent[water])
    }

    func testANewTaskGoesToTheIPhonesInboxAndClearsTheField() async {
        let (client, sent) = WatchClient.inMemory()
        let store = makeStore(client)
        await store.send(.binding(.set(\.draft, "  Call Mum tomorrow 6pm ")))
        await store.send(.addSubmitted)
        await store.finish()
        XCTAssertEqual(sent.value, [.add("Call Mum tomorrow 6pm")])
        XCTAssertEqual(store.state.draft, "")
        XCTAssertEqual(store.state.added.map(\.title), ["Call Mum tomorrow 6pm"])
    }

    func testABlankNewTaskSendsNothing() async {
        let (client, sent) = WatchClient.inMemory()
        let store = makeStore(client)
        await store.send(.binding(.set(\.draft, "   ")))
        await store.send(.addSubmitted)
        await store.finish()
        XCTAssertEqual(sent.value, [])
        XCTAssertEqual(store.state.added, [])
    }

    func testAChangeThatCantBeSentSaysSoAndDropsItsMark() async {
        struct Unreachable: Error {}
        var client = WatchClient.inMemory().client
        client.send = { _ in throw Unreachable() }
        let store = makeStore(client, state: WatchFeature.State(glance: glance(asOf: now - 60)))
        await store.send(.rowTapped(run))
        await store.send(.statusPicked(.inProgress))
        await store.receive(\.sendFailed)
        XCTAssertNil(store.state.sent[run])
        XCTAssertNotNil(store.state.alert)
    }

    func testAReminderOnTheWatchCanAskForItsTasksStatuses() async {
        let store = makeStore(WatchClient.inMemory().client, state: WatchFeature.State(glance: glance(asOf: now)))
        await store.send(.reminderStatusRequested(run))
        XCTAssertEqual(store.state.chosen?.title, "Long run")
    }
}

final class WatchLinkTests: XCTestCase {
    func testTheWatchOffersTheIPhonesReminderButtons() {
        XCTAssertEqual(
            [WatchReminder.done, WatchReminder.snooze, WatchReminder.changeStatus],
            ReminderAction.allCases.map(\.rawValue))
        XCTAssertEqual(WatchReminder.category, ReminderAction.category)
        XCTAssertEqual(WatchReminder.hiddenCategory, ReminderAction.hiddenCategory)
    }

    func testARequestReadsBackAsItWasSent() throws {
        let id = UUID()
        for request in [WatchRequest.setStatus(id, .wontDo), .add("Run"), .snooze(id)] {
            let message: [String: Any] = [WatchMessage.requestKey: try WatchMessage.encode(request)]
            XCTAssertEqual(try WatchMessage.request(in: message), request)
        }
        XCTAssertNil(try WatchMessage.request(in: [:]))
    }

    func testARequestFromANewerLifrIsUnreadable() {
        let message: [String: Any] = [WatchMessage.requestKey: Data(#"{"archive":{"_0":"x"}}"#.utf8)]
        XCTAssertThrowsError(try WatchMessage.request(in: message))
    }

    func testTheWatchGetsTodayAndTomorrowWithAtMostTheLimitOfTasks() throws {
        let start = Date(timeIntervalSince1970: 1_793_664_000)
        let rows = (0..<5).map { TodayGlance.Row(id: UUID(), title: "Task \($0)", code: "T-\($0)", isOverdue: false) }
        let days = (1...3).map { day in
            TodayGlance(
                asOf: start, from: start + Double(day) * 86_400, until: start + Double(day + 1) * 86_400,
                rows: rows, locked: false)
        }
        let full = TodayGlance(
            asOf: start, from: start, until: start + 86_400, rows: rows, locked: false, later: days,
            avenues: [], dueCounts: ["2026-11-03": 5])
        let watch = full.forWatch(limit: 2)
        XCTAssertEqual(watch.days.map(\.from), [start, start + 86_400])
        XCTAssertEqual(watch.days.map(\.rows.count), [2, 2])
        XCTAssertEqual(watch.count, 5)
        XCTAssertNil(watch.avenues)
        XCTAssertNil(watch.dueCounts)

        let message: [String: Any] = [WatchMessage.glanceKey: try WatchMessage.encode(watch)]
        XCTAssertEqual(try WatchMessage.glance(in: message), watch)
    }
}
