import ComposableArchitecture
import Foundation
import LifrCore
import LifrProjectDetail
import LifrStoreClient
import XCTest

@MainActor
final class ProjectDetailFeatureTests: XCTestCase {
    /// 2026-11-03 10:00 UTC, a Tuesday.
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private lazy var records = RecordsBuilder(now: now, timeZone: .gmt)
    private lazy var health = records.avenue("HEALTH", "Health")
    private lazy var home = records.avenue("HOME", "Home")
    private lazy var marathon = records.project("Half marathon", in: health)

    private func detail() -> TestStoreOf<ProjectDetailFeature>? {
        guard let state = ProjectDetailFeature.State(id: marathon.id, workspace: records.workspace) else {
            XCTFail("The Project isn't in the records.")
            return nil
        }
        let store = TestStore(initialState: state) {
            ProjectDetailFeature()
        } withDependencies: {
            $0.store = .inMemory(records.workspace)
            $0.date.now = now
            $0.timeZone = .gmt
            $0.uuid = .incrementing
            $0.withRandomNumberGenerator = WithRandomNumberGenerator(SystemRandomNumberGenerator())
        }
        store.exhaustivity = .off
        return store
    }

    func testCountsTopLevelTasksAndOnlyCompletesOnceTheyAreClosed() async {
        let shoes = records.task("Buy running shoes", in: health, project: marathon)
        records.task("Try them on", in: health, project: marathon, parent: shoes)
        records.task("Sign up", in: health, project: marathon, status: .done)
        guard let store = detail() else { return }

        XCTAssertEqual(store.state.taskCount, 2)
        XCTAssertEqual(store.state.openCount, 1)
        XCTAssertFalse(store.state.canComplete)
    }

    func testRenamesOnSubmitAndIgnoresABlankName() async throws {
        guard let store = detail() else { return }

        await store.send(.binding(.set(\.name, "  ")))
        await store.send(.nameSubmitted) { $0.name = "Half marathon" }

        await store.send(.binding(.set(\.name, " Spring half marathon ")))
        await store.send(.nameSubmitted)
        await store.receive(\.saved.success)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.name, "Spring half marathon")
        XCTAssertEqual(store.state.savedName, "Spring half marathon")
        let workspace = try await store.dependencies.store.load()
        XCTAssertEqual(workspace.projects[marathon.id]?.name, "Spring half marathon")
    }

    func testSetsAndClearsTheDueDate() async throws {
        guard let store = detail() else { return }
        let day = try CalendarDate(year: 2026, month: 12, day: 1)

        await store.send(.dueDatePicked(day))
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.dueDate, day)

        await store.send(.dueDatePicked(nil))
        await store.receive(\.delegate.saved)
        let workspace = try await store.dependencies.store.load()
        XCTAssertNil(workspace.projects[marathon.id]?.dueDate)
    }

    func testMovingToAnotherAvenueTakesTheTasks() async throws {
        let shoes = records.task("Buy running shoes", in: health, project: marathon)
        _ = home
        guard let store = detail() else { return }

        await store.send(.avenuePicked(home.id))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.avenueID, home.id)
        let workspace = try await store.dependencies.store.load()
        XCTAssertEqual(workspace.tasks[shoes.id]?.avenueID, home.id)
    }

    func testCompletesOnceEveryTaskIsClosed() async throws {
        records.task("Sign up", in: health, project: marathon, status: .done)
        guard let store = detail() else { return }

        await store.send(.completeTapped)
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.status, .completed)
        await store.send(.restoreTapped)
        await store.receive(\.delegate.saved)
        XCTAssertEqual(store.state.status, .active)
    }

    func testArchivingWithOpenTasksAsksAndCanMoveThemOut() async throws {
        let shoes = records.task("Buy running shoes", in: health, project: marathon)
        guard let store = detail() else { return }

        await store.send(.archiveTapped)
        XCTAssertNotNil(store.state.archive)

        await store.send(.archive(.presented(.moveToAvenue)))
        await store.receive(\.delegate.saved)

        XCTAssertEqual(store.state.status, .archived)
        let workspace = try await store.dependencies.store.load()
        XCTAssertNil(workspace.tasks[shoes.id]?.projectID)
        XCTAssertEqual(workspace.tasks[shoes.id]?.status, .todo)
    }

    func testArchivingCanCloseTheOpenTasks() async throws {
        let shoes = records.task("Buy running shoes", in: health, project: marathon)
        guard let store = detail() else { return }

        await store.send(.archiveTapped)
        await store.send(.archive(.presented(.closeAsWontDo)))
        await store.receive(\.delegate.saved)

        let workspace = try await store.dependencies.store.load()
        XCTAssertEqual(workspace.tasks[shoes.id]?.status, .wontDo)
        XCTAssertEqual(workspace.tasks[shoes.id]?.projectID, marathon.id)
    }

    func testArchivingWithoutOpenTasksDoesNotAsk() async {
        guard let store = detail() else { return }

        await store.send(.archiveTapped)
        await store.receive(\.delegate.saved)

        XCTAssertNil(store.state.archive)
        XCTAssertEqual(store.state.status, .archived)
    }

    func testAFailedSaveShowsAnAlertAndKeepsTheStoredName() async {
        guard let store = detail() else { return }
        let id = marathon.id.uuidString
        store.dependencies.store.save = { _ in throw LifrError.notFound(.project, id) }

        await store.send(.binding(.set(\.name, "Other")))
        await store.send(.nameSubmitted)
        await store.receive(\.saved.failure)

        XCTAssertNotNil(store.state.alert)
        XCTAssertEqual(store.state.name, "Half marathon")
    }
}
