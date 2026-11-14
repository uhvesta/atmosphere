import ComposableArchitecture
import HumataDaemonClient
import HumataProjects
import XCTest

@MainActor
final class ModelDefaultsFeatureTests: XCTestCase {
    private func store(_ daemon: InMemoryDaemon) -> TestStoreOf<ModelDefaultsFeature> {
        let store = TestStore(initialState: ModelDefaultsFeature.State()) {
            ModelDefaultsFeature()
        } withDependencies: {
            $0.daemon = daemon.client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        return store
    }

    func testEveryProjectsDefaultsAreSavedAsTheyChange() async throws {
        let daemon = InMemoryDaemon()
        daemon.setModels([ModelChoice(id: "opus", name: "Opus", efforts: ["low", "high"])], for: .claude)
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.backendPicked(.claude))
        await store.receive(\.saved)
        await store.send(.modelTyped("opus"))
        await store.receive(\.saved)
        await store.send(.effortPicked("high"))
        await store.receive(\.saved)

        XCTAssertFalse(store.state.saving)
        let saved = try await daemon.client.sessionDefaults()
        XCTAssertEqual(saved, SessionDefaults(backend: .claude, model: "opus", effort: "high"))
    }

    func testAProjectStartsFromEveryProjectsAndGoesBackToThemWhenItsOwnAreOff() async throws {
        let daemon = InMemoryDaemon()
        _ = try await daemon.client.setSessionDefaults(nil, SessionDefaults(backend: .claude, model: "opus"))
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.projectPicked(project.id))
        XCTAssertFalse(store.state.editable)
        XCTAssertEqual(store.state.defaults.model, "opus")

        await store.send(.ownToggled(true))
        await store.receive(\.saved)
        await store.send(.backendPicked(.copilot))
        await store.receive(\.saved)
        await store.send(.experimentalToggled(true))
        await store.receive(\.saved)
        var saved = try await daemon.client.projects().first { $0.id == project.id }
        XCTAssertEqual(saved?.defaults, SessionDefaults(backend: .copilot, experimental: true))

        await store.send(.ownToggled(false))
        await store.receive(\.saved)
        saved = try await daemon.client.projects().first { $0.id == project.id }
        XCTAssertNil(saved?.defaults)
        let every = try await daemon.client.sessionDefaults()
        XCTAssertEqual(every.model, "opus")
    }

    func testReappearingWhileEditingDoesNotPutTheOldDefaultsBack() async throws {
        let daemon = InMemoryDaemon()
        daemon.setModels([ModelChoice(id: ModelChoice.fusion, name: "Copilot Fusion")], for: .copilot)
        _ = try await daemon.client.setSessionDefaults(nil, SessionDefaults(backend: .claude, model: "opus"))
        let project = try await daemon.client.createProject("code", [Repository(path: "/code")], "")
        let store = store(daemon)
        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.projectPicked(project.id))

        // The tab shows again, and its load is still on its way when the
        // project gets its own agent.
        await store.send(.task)
        await store.send(.ownToggled(true))
        await store.send(.backendPicked(.copilot))
        await store.receive(\.modelsLoaded)
        await store.finish()

        XCTAssertEqual(store.state.defaults.backend, .copilot)
        XCTAssertEqual(store.state.models.map(\.id), [ModelChoice.fusion])
        let saved = try await daemon.client.projects().first { $0.id == project.id }
        XCTAssertEqual(saved?.defaults?.backend, .copilot)
    }

    func testAListThatComesLateForAnAgentNoLongerPickedIsNotShown() async throws {
        let daemon = InMemoryDaemon()
        daemon.setModels([ModelChoice(id: ModelChoice.fusion, name: "Copilot Fusion")], for: .copilot)
        let (claudeAsked, asked) = AsyncStream.makeStream(of: Void.self)
        let (claudeAnswers, answer) = AsyncStream.makeStream(of: Void.self)
        var client = daemon.client
        let listed = client.models
        client.models = { host, backend, config in
            if backend == .claude {
                asked.yield()
                for await _ in claudeAnswers { break }
                return [ModelChoice(id: "opus", name: "Opus")]
            }
            return try await listed(host, backend, config)
        }
        let store = TestStore(initialState: ModelDefaultsFeature.State()) {
            ModelDefaultsFeature()
        } withDependencies: {
            $0.daemon = client
            $0.continuousClock = ImmediateClock()
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.backendPicked(.claude))
        XCTAssertTrue(store.state.modelsLoading)
        for await _ in claudeAsked { break }
        await store.send(.backendPicked(.copilot))
        await store.receive(\.modelsLoaded)
        XCTAssertFalse(store.state.modelsLoading)
        answer.yield()
        await store.finish()

        XCTAssertEqual(store.state.defaults.backend, .copilot)
        XCTAssertEqual(store.state.models.map(\.id), [ModelChoice.fusion])
    }
}
