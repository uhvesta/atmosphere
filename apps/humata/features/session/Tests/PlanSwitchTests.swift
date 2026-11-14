import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import XCTest

/// Switching an open chat between full access and plan mode from the
/// composer's Plan toggle.
@MainActor
final class PlanSwitchTests: XCTestCase {
    /// A store for `session`, whose host takes plan mode when `plan`,
    /// whose daemon keeps what it's asked in `asked` and answers as
    /// `answer` says, or else with the session switched as asked.
    private func store(
        _ session: Session, plan: Bool = true, asked: LockIsolated<[SessionUpdate]> = LockIsolated([]),
        answer: (@Sendable (Session, SessionUpdate) throws -> Session)? = nil
    ) -> TestStoreOf<SessionFeature> {
        var state = SessionFeature.State(session: session)
        state.chatCommands = ChatCommands(plan: plan)
        let store = TestStore(initialState: state) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, _ in }
            $0.uuid = .incrementing
            $0.daemon.update = { _, update in
                asked.withValue { $0.append(update) }
                if let answer { return try answer(session, update) }
                var changed = session
                if let access = update.access { changed.access = access }
                return changed
            }
        }
        store.exhaustivity = .off
        return store
    }

    private func session(_ id: String) -> Session {
        Session(id: id, name: "first", backend: .claude, workingDirectory: "/work")
    }

    func testTogglingPlanSwitchesTheSessionAndBack() async {
        let asked = LockIsolated<[SessionUpdate]>([])
        let store = store(session("plan-toggle"), asked: asked)

        await store.send(.plan(.toggled))
        await store.receive(\.plan.changed)
        XCTAssertEqual(store.state.session.access, .plan)
        XCTAssertFalse(store.state.planSwitching)

        await store.send(.plan(.toggled))
        await store.receive(\.plan.changed)
        XCTAssertEqual(store.state.session.access, .full)
        XCTAssertEqual(asked.value.map(\.access), [.plan, .full])
        await store.finish()
    }

    func testAHostWithoutPlanModeIsAskedNothing() async {
        let asked = LockIsolated<[SessionUpdate]>([])
        let store = store(session("plan-unoffered"), plan: false, asked: asked)
        XCTAssertFalse(PlanSwitch.offered(for: store.state.session, commands: store.state.chatCommands))

        await store.send(.plan(.toggled))
        await store.finish()
        XCTAssertEqual(asked.value, [])
        XCTAssertEqual(store.state.session.access, .full)
    }

    func testASideChatAndAPlanWithTheCodeAreNeverOffered() {
        var side = session("plan-side")
        side.readOnly = true
        side.access = .sideChat
        var withCode = session("plan-code")
        withCode.planning = true
        withCode.access = .plan
        withCode.followsBase = true
        let commands = ChatCommands(plan: true)
        XCTAssertFalse(PlanSwitch.offered(for: side, commands: commands))
        XCTAssertFalse(PlanSwitch.offered(for: withCode, commands: commands))
        XCTAssertTrue(PlanSwitch.offered(for: session("plan-ordinary"), commands: commands))
    }

    /// A Plan Only session switched into plan mode doesn't follow its
    /// base, so it can be switched back; one planning with the code can't.
    func testOnlyASessionThatFollowsItsBaseLosesTheSwitch() {
        let commands = ChatCommands(plan: true)
        var planOnly = session("plan-only")
        planOnly.planning = true
        planOnly.access = .plan
        XCTAssertTrue(PlanSwitch.offered(for: planOnly, commands: commands))

        var withCode = planOnly
        withCode.followsBase = true
        XCTAssertFalse(PlanSwitch.offered(for: withCode, commands: commands))
    }

    func testAPlanOnlySessionInPlanSwitchesBackToFull() async {
        var planOnly = session("plan-only-back")
        planOnly.planning = true
        planOnly.access = .plan
        let asked = LockIsolated<[SessionUpdate]>([])
        let store = store(planOnly, asked: asked)

        await store.send(.plan(.toggled))
        await store.receive(\.plan.changed)
        XCTAssertEqual(store.state.session.access, .full)
        XCTAssertEqual(asked.value.map(\.access), [.full])
        await store.finish()
    }

    func testAnArchivedSessionIsAskedNothing() async {
        let asked = LockIsolated<[SessionUpdate]>([])
        var archived = session("plan-archived")
        archived.archived = true
        let store = store(archived, asked: asked)

        await store.send(.plan(.toggled))
        await store.finish()
        XCTAssertEqual(asked.value, [])
    }

    /// A humatad from before plan mode answers with the session as it was.
    func testAnOlderHostThatLeavesItAsItWasSaysSo() async {
        let store = store(session("plan-older")) { session, _ in session }

        await store.send(.plan(.toggled))
        await store.receive(\.plan.changed)
        XCTAssertEqual(store.state.session.access, .full)
        XCTAssertNotNil(store.state.alert)
        XCTAssertFalse(store.state.planSwitching)
    }

    func testARefusedSwitchLeavesTheSessionAsItWas() async {
        let store = store(session("plan-refused")) { _, _ in
            throw DaemonError.refused(.invalidRequest, message: "no")
        }

        await store.send(.plan(.toggled))
        await store.receive(\.plan.failed)
        XCTAssertEqual(store.state.session.access, .full)
        XCTAssertNotNil(store.state.alert)
        XCTAssertFalse(store.state.planSwitching)
    }
}
