import ComposableArchitecture
import Foundation
import LifrCore
import LifrSearch
import LifrStoreClient
import XCTest

@MainActor
final class SearchFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")

    private func load() async -> TestStoreOf<SearchFeature> {
        let store = TestStore(initialState: SearchFeature.State()) {
            SearchFeature()
        } withDependencies: { [now, records] in
            $0.store = .inMemory(records.workspace)
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        await store.send(.list(.task))
        await store.receive(\.list.loaded)
        return store
    }

    private func titles(_ store: TestStoreOf<SearchFeature>) -> [String] {
        store.state.list.rows.map(\.title)
    }

    func testShowsNothingUntilThereIsSomethingToSearchFor() async {
        records.task("Long run", in: health)

        let store = await load()

        XCTAssertTrue(store.state.list.isEmpty)
    }

    func testMatchesEveryWordInTitlesDetailsAndComments() async {
        records.task("Long run", in: health)
        let physio = records.task("Book physio", in: health)
        records.comment("Ask about the knee after the long run", on: physio)
        records.task("Water the plants", in: health)

        let store = await load()
        await store.send(.binding(.set(\.filter.text, "LONG run")))

        XCTAssertEqual(titles(store), ["Long run", "Book physio"])

        await store.send(.binding(.set(\.filter.text, "knee run")))

        XCTAssertEqual(titles(store), ["Book physio"])
    }

    func testLeavesOutClosedTasksUnlessAStatusIsPicked() async {
        records.task("Run, open", in: health)
        records.task("Run, done", in: health, status: .done)

        let store = await load()
        await store.send(.binding(.set(\.filter.text, "run")))

        XCTAssertEqual(titles(store), ["Run, open"])

        await store.send(.binding(.set(\.filter.statuses, [.done])))

        XCTAssertEqual(titles(store), ["Run, done"])
    }

    func testFiltersWithoutWords() async {
        let errand = records.label("errand")
        records.task("Buy shoes", in: health, priority: .high, due: 1, labels: [errand])
        records.task("Buy socks", in: health, priority: .high, labels: [errand])
        records.task("Stretch", in: health, priority: .high, due: -1)
        records.task("Rest", in: health, due: 0)

        let store = await load()
        await store.send(.binding(.set(\.filter.priorities, [.high])))
        XCTAssertEqual(Set(titles(store)), ["Buy shoes", "Buy socks", "Stretch"])

        await store.send(.binding(.set(\.filter.labels, ["errand"])))
        XCTAssertEqual(Set(titles(store)), ["Buy shoes", "Buy socks"])

        await store.send(.binding(.set(\.filter.due, .nextSevenDays)))
        XCTAssertEqual(titles(store), ["Buy shoes"])

        await store.send(.clearFiltersTapped)
        XCTAssertTrue(store.state.list.isEmpty)
    }

    func testMatchesDueDates() async {
        records.task("Overdue", in: health, due: -1)
        records.task("Today", in: health, due: 0)
        records.task("Someday", in: health)

        let store = await load()
        await store.send(.binding(.set(\.filter.due, .overdue)))
        XCTAssertEqual(titles(store), ["Overdue"])

        await store.send(.binding(.set(\.filter.due, .today)))
        XCTAssertEqual(titles(store), ["Today"])

        await store.send(.binding(.set(\.filter.due, SearchFilter.Due.none)))
        XCTAssertEqual(titles(store), ["Someday"])
    }

    func testFindsSubtasks() async {
        let race = records.task("Race day", in: health)
        records.task("Pin the bib", in: health, parent: race)

        let store = await load()
        await store.send(.binding(.set(\.filter.text, "bib")))

        XCTAssertEqual(titles(store), ["Pin the bib"])
    }

    func testFinishingAResultKeepsItWhenDoneIsPicked() async {
        let shoes = records.task("Buy shoes", in: health)
        let store = await load()
        await store.send(.binding(.set(\.filter.text, "shoes")))

        await store.send(.list(.toggled(shoes.id)))
        await store.receive(\.delegate.saved)

        XCTAssertTrue(store.state.list.isEmpty)

        await store.send(.binding(.set(\.filter.statuses, [.done])))
        XCTAssertEqual(titles(store), ["Buy shoes"])
    }
}
