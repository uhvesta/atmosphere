import ComposableArchitecture
import HumataDaemonClient
import HumataSession
import XCTest

/// Changing an open chat's model from the composer's pill.
@MainActor
final class ModelSwitchTests: XCTestCase {
    private let models = [
        ModelChoice(id: "opus", name: "Opus", efforts: ["low", "high"]),
        ModelChoice(id: "sonnet", name: "Sonnet", efforts: ["low", "high"]),
        ModelChoice(id: "haiku", name: "Haiku"),
    ]

    /// A store for `session` whose daemon lists `models` for it, keeps
    /// what it is asked to change in `asked`, and answers as `answer`
    /// says, or else with the session changed as asked.
    private func store(
        _ session: Session, asked: LockIsolated<[SessionUpdate]> = LockIsolated([]),
        answer: (@Sendable (Session, SessionUpdate) -> Session)? = nil
    ) -> TestStoreOf<SessionFeature> {
        let models = models
        let store = TestStore(initialState: SessionFeature.State(session: session)) {
            SessionFeature()
        } withDependencies: {
            $0.daemon.markSeen = { _, _ in }
            $0.uuid = .incrementing
            $0.daemon.sessionModels = { _ in models }
            $0.daemon.update = { _, update in
                asked.withValue { $0.append(update) }
                if let answer { return answer(session, update) }
                var changed = session
                if let model = update.model { changed.model = model.isEmpty ? nil : model }
                if let effort = update.effort { changed.effort = effort.isEmpty ? nil : effort }
                return changed
            }
        }
        store.exhaustivity = .off
        return store
    }

    private func session(
        _ id: String, backend: Backend = .claude, model: String? = "opus", effort: String? = nil
    ) -> Session {
        Session(id: id, name: "first", backend: backend, workingDirectory: "/work", model: model, effort: effort)
    }

    private func event(_ id: String, _ kind: SessionEvent.Kind) -> SessionEvent {
        SessionEvent(id: id, time: Date(timeIntervalSince1970: 0), kind: kind)
    }

    func testPickingAModelSendsItAndTheSessionHasIt() async {
        let asked = LockIsolated<[SessionUpdate]>([])
        let store = store(session("model-pick"), asked: asked)

        await store.send(.model(.pillTapped))
        await store.receive(\.model.listed)
        XCTAssertTrue(store.state.modelSwitch.isPicking)
        XCTAssertEqual(store.state.modelSwitch.models?.map(\.id), ["opus", "sonnet", "haiku"])

        await store.send(.model(.picked("sonnet")))
        await store.receive(\.model.changed)
        XCTAssertEqual(asked.value.map(\.model), ["sonnet"])
        XCTAssertEqual(store.state.session.model, "sonnet")
        XCTAssertFalse(store.state.modelSwitch.isPicking)
        await store.finish()
    }

    func testPickingTheModelItHasSendsNothing() async {
        let asked = LockIsolated<[SessionUpdate]>([])
        let store = store(session("model-same"), asked: asked)
        await store.send(.model(.pillTapped))
        await store.receive(\.model.listed)

        await store.send(.model(.picked("opus")))
        XCTAssertFalse(store.state.modelSwitch.isPicking)
        await store.finish()
        XCTAssertEqual(asked.value, [])
    }

    /// The daemon holds the change for the turn after the running one, so
    /// it is sent at once: a queued message's turn starts on it too.
    func testPickingWhileATurnRunsSendsItAndLeavesTheTurnRunning() async {
        let asked = LockIsolated<[SessionUpdate]>([])
        let store = store(session("model-running"), asked: asked)
        await store.send(.received(event("e1", .userMessage("write the notes"))))
        await store.send(.received(event("e2", .turnStarted)))
        XCTAssertTrue(store.state.isWorking)

        await store.send(.model(.pillTapped))
        await store.receive(\.model.listed)
        await store.send(.model(.picked("sonnet")))
        await store.receive(\.model.changed)

        XCTAssertEqual(asked.value.map(\.model), ["sonnet"])
        XCTAssertEqual(store.state.session.model, "sonnet")
        XCTAssertTrue(store.state.isWorking)
        XCTAssertTrue(store.state.canStop)
        await store.finish()
    }

    func testAnEffortTheNewModelDoesntTakeGoesBackToItsDefault() async {
        let asked = LockIsolated<[SessionUpdate]>([])
        let store = store(session("model-effort", effort: "high"), asked: asked)
        await store.send(.model(.pillTapped))
        await store.receive(\.model.listed)

        await store.send(.model(.picked("sonnet")))
        await store.receive(\.model.changed)
        XCTAssertEqual(store.state.session.effort, "high")

        await store.send(.model(.pillTapped))
        await store.receive(\.model.listed)
        await store.send(.model(.picked("haiku")))
        await store.receive(\.model.changed)
        XCTAssertEqual(asked.value.map(\.effort), [nil, ""])
        XCTAssertNil(store.state.session.effort)
        await store.finish()
    }

    func testAHostThatLeavesTheModelAsItWasSaysSoAndTheSessionKeepsIts() async {
        let store = store(session("model-old-host")) { session, _ in session }
        await store.send(.model(.pillTapped))
        await store.receive(\.model.listed)

        await store.send(.model(.picked("sonnet")))
        await store.receive(\.model.changed)
        XCTAssertEqual(store.state.session.model, "opus")
        XCTAssertNotNil(store.state.modelSwitch.failure)
        XCTAssertTrue(store.state.modelSwitch.isPicking)
        await store.finish()
    }

    func testAChangeTheDaemonRefusesSaysWhyAndCanBeTriedAgain() async {
        struct Refused: Error {}
        let store = store(session("model-refused"))
        store.dependencies.daemon.update = { _, _ in throw Refused() }
        await store.send(.model(.pillTapped))
        await store.receive(\.model.listed)

        await store.send(.model(.picked("sonnet")))
        await store.receive(\.model.changeFailed)
        XCTAssertEqual(store.state.session.model, "opus")
        XCTAssertNotNil(store.state.modelSwitch.failure)
        XCTAssertFalse(store.state.modelSwitch.changing)
        await store.finish()
    }

    func testModelsThatCantBeListedSayWhyAndAreAskedForAgain() async {
        struct Unreachable: Error {}
        let store = store(session("model-unlisted"))
        store.dependencies.daemon.sessionModels = { _ in throw Unreachable() }
        await store.send(.model(.pillTapped))
        await store.receive(\.model.listFailed)
        XCTAssertNil(store.state.modelSwitch.models)
        XCTAssertNotNil(store.state.modelSwitch.failure)

        let models = models
        store.dependencies.daemon.sessionModels = { _ in models }
        await store.send(.model(.tryAgainTapped))
        await store.receive(\.model.listed)
        XCTAssertEqual(store.state.modelSwitch.models?.count, 3)
        XCTAssertNil(store.state.modelSwitch.failure)
        await store.finish()
    }

    func testAModelIsOfferedOnlyWhereItCanChange() async {
        XCTAssertTrue(ModelSwitch.offered(for: session("a")))
        XCTAssertTrue(ModelSwitch.offered(for: session("b", backend: .copilot)))
        XCTAssertTrue(ModelSwitch.offered(for: session("c", backend: .local)))
        XCTAssertFalse(ModelSwitch.offered(for: session("d", backend: .scripted)))
        var archived = session("e")
        archived.archived = true
        XCTAssertFalse(ModelSwitch.offered(for: archived))
        var away = session("f")
        away.hostOffline = true
        XCTAssertFalse(ModelSwitch.offered(for: away))

        let store = store(archived)
        await store.send(.model(.pillTapped))
        XCTAssertFalse(store.state.modelSwitch.isPicking)
        await store.finish()
    }

    func testChangingToALocalModelThatCantSeeImagesStopsTakingThem() async {
        let seeing = ModelChoice(id: "llava", name: "llava", readsImages: true)
        let blind = ModelChoice(id: "qwen", name: "qwen", readsImages: false)
        let store = store(session("model-images", backend: .local, model: "llava"))
        store.dependencies.daemon.sessionModels = { _ in [seeing, blind] }
        store.dependencies.daemon.models = { _, _, _ in [seeing, blind] }
        await store.send(.readsImagesFound(true))
        XCTAssertTrue(store.state.takesImages)

        await store.send(.model(.pillTapped))
        await store.receive(\.model.listed)
        await store.send(.model(.picked("qwen")))
        await store.receive(\.model.changed)
        await store.receive(\.readsImagesFound)
        XCTAssertFalse(store.state.takesImages)
        await store.finish()
    }
}
