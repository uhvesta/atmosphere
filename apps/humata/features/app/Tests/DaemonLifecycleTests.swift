import ComposableArchitecture
import HumataAppFeature
import HumataDaemonClient
import XCTest

@MainActor
final class DaemonLifecycleTests: XCTestCase {
    private let working = Session(
        id: "s1", name: "busy", backend: .scripted, workingDirectory: "/work", state: .working)

    private func service(current: Bool, restarts: LockIsolated<Int>) -> DaemonService {
        DaemonService(
            start: {}, isCurrent: { current && restarts.value == 0 }, restart: { restarts.withValue { $0 += 1 } },
            openLoginItems: {})
    }

    func testTheAppsOwnDaemonIsUsedAsItIs() async {
        let restarts = LockIsolated(0)
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.daemonService = service(current: true, restarts: restarts)
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.daemonChecked)
        await store.receive(\.sidebar.refresh)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(restarts.value, 0)
    }

    func testAnOlderDaemonWithNothingInProgressIsReplacedAtOnce() async {
        let restarts = LockIsolated(0)
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.daemonService = service(current: false, restarts: restarts)
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.daemonChecked)
        await store.receive(\.restarted)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(restarts.value, 1)
    }

    func testAnOlderDaemonMidTurnWaitsUntilTheTurnEnds() async {
        let restarts = LockIsolated(0)
        let sessions = LockIsolated([working])
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.daemonService = service(current: false, restarts: restarts)
            $0.daemon.sessions = { _ in sessions.value }
            $0.daemon.projects = { [] }
            $0.daemon.inbox = { [] }
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.daemonChecked)
        XCTAssertEqual(store.state.daemon, .updatePending)

        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        XCTAssertEqual(store.state.daemon, .updatePending)
        XCTAssertEqual(restarts.value, 0)

        sessions.setValue([Session(id: "s1", name: "busy", backend: .scripted, workingDirectory: "/work")])
        await store.send(\.sidebar.refresh)
        await store.receive(\.sidebar.loaded)
        await store.receive(\.restarted)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(restarts.value, 1)
    }

    func testThePersonCanReplaceItMidTurn() async {
        let restarts = LockIsolated(0)
        var state = AppFeature.State()
        state.daemon = .updatePending
        let store = TestStore(initialState: state) {
            AppFeature()
        } withDependencies: {
            $0.daemonService = service(current: false, restarts: restarts)
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off
        await store.send(.restartTapped)
        await store.receive(\.restarted)
        XCTAssertEqual(store.state.daemon, .running)
        XCTAssertEqual(restarts.value, 1)
    }

    func testALoginItemThatIsOffOffersSystemSettingsThenTryingAgain() async {
        let approved = LockIsolated(false)
        let opened = LockIsolated(false)
        let store = TestStore(initialState: AppFeature.State()) {
            AppFeature()
        } withDependencies: {
            $0.daemonService = DaemonService(
                start: { if !approved.value { throw DaemonServiceError.needsApproval } }, isCurrent: { true },
                restart: {}, openLoginItems: { opened.setValue(true) })
            $0.daemon = InMemoryDaemon().client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.daemonFailed)
        guard case .failed(_, needsLoginItems: true) = store.state.daemon else {
            return XCTFail("expected it to offer Login Items, got \(store.state.daemon)")
        }
        await store.send(.openLoginItemsTapped)
        await store.finish()
        XCTAssertTrue(opened.value)

        approved.setValue(true)
        await store.send(.task)
        await store.receive(\.daemonChecked)
        XCTAssertEqual(store.state.daemon, .running)
    }
}
