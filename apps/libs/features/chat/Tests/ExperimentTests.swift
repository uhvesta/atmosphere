import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit
import ModelsClient
import XCTest

@testable import ChatFeatures

@MainActor
final class ExperimentTests: XCTestCase {
    private let qwen = ModelCatalog.qwen35Small.id

    /// Answers `text`, counting the tokens it read and wrote as an engine
    /// does.
    private func counted(_ text: String) -> ScriptedRuntime.Step {
        { turn, _ throws(ModelRuntimeError) in
            turn.speed?(TurnSpeed(read: 30, written: 6, reading: .seconds(1), writing: .seconds(2)))
            return .string(text)
        }
    }

    func testAReplyKeepsItsModelAndHowFastAnEngineCountedIt() async throws {
        let store = chat(Runs([counted("Friend.")]))
        await store.send(.task)
        await store.receive(\.modelsLoaded)
        await store.ask("Amigo?")
        let reply = try store.state.thread.messages.last.unwrap()
        XCTAssertNotNil(reply.model)
        XCTAssertEqual(reply.model, store.state.current?.name)
        let speed = try reply.speed.unwrap()
        XCTAssertEqual(speed, ThreadSpeed(read: 30, written: 6, reading: .seconds(1), writing: .seconds(2)))
        XCTAssertEqual(speed.writtenPerSecond, 3)
    }

    func testAModelThatDoesntCountItsTokensHasThemGuessed() async throws {
        let store = chat(Runs([ScriptedRuntime.answer("It means friend, or pal.")]))
        await store.ask("Amigo?")
        let speed = try store.state.thread.messages.last?.speed.unwrap()
        XCTAssertEqual(speed?.estimated, true)
        XCTAssertGreaterThan(speed?.read ?? 0, 0)
        XCTAssertGreaterThan(speed?.written ?? 0, 0)
    }

    func testAStoppedReplyHasNoSpeed() async throws {
        let writing: ScriptedRuntime.Step = { _, partial throws(ModelRuntimeError) in
            partial(.string("It means"))
            try? await Task.sleep(for: .seconds(30))
            return .string("It means")
        }
        let store = chat(Runs([writing]))
        await store.send(.binding(.set(\.input, "Amigo?")))
        await store.send(.sendTapped)
        await store.receive(\.progressed)
        await store.send(.stopTapped)
        await store.settle()
        XCTAssertNil(store.state.thread.messages.last?.speed)
    }

    func testEachReplyShowsWhatItsModelWasSent() async throws {
        let store = chat(Runs([ScriptedRuntime.answer("Friend.")]))
        await store.ask("Amigo?")
        let reply = try store.state.thread.messages.last.unwrap()
        XCTAssertNil(store.state.shownPromptValue)

        await store.send(.promptTapped(messageID: reply.id))
        let prompt = try store.state.shownPromptValue.unwrap()
        XCTAssertEqual(prompt.instructions, "You help with Spanish.")
        XCTAssertTrue(prompt.prompt.contains("Amigo?"))
        XCTAssertEqual(prompt.photos, 0)
        XCTAssertNotNil(prompt.context)

        await store.send(.promptTapped(messageID: nil))
        XCTAssertNil(store.state.shownPromptValue)
    }

    /// A chat comparing with Qwen, whose first model answers "Friend." and
    /// Qwen "Pal.".
    private func comparing(_ runs: Runs) async -> TestStoreOf<ChatThreadFeature> {
        let store = chat(runs, library: InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]))
        store.state.$compares.withLock { $0 = true }
        await store.send(.rivalTapped)
        await store.send(.rivalSheet(.presented(.task)))
        await store.receive(\.rivalSheet.presented.loaded)
        await store.send(.rivalSheet(.presented(.binding(.set(\.model, qwen)))))
        await store.send(.rivalSheet(.dismiss))
        return store
    }

    func testComparingAsksTheSecondModelTooAndKeepsTheAnswerPicked() async throws {
        let runs = Runs([ScriptedRuntime.answer("Friend."), ScriptedRuntime.answer("Pal.")])
        let store = await comparing(runs)
        XCTAssertEqual(store.state.rivalCurrent?.id, qwen)
        await store.ask("Amigo?")
        XCTAssertEqual(runs.choices.value.map(\.model), [nil, qwen])
        XCTAssertEqual(store.state.rival?.text, "Pal.")
        XCTAssertEqual(store.state.rival?.done, true)
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])

        await store.send(.keepTapped(rival: true))
        XCTAssertNil(store.state.rival)
        XCTAssertEqual(store.texts, ["Amigo?", "Pal."])
        let kept = try store.state.thread.messages.last.unwrap()
        XCTAssertEqual(kept.model, ModelCatalog.qwen35Small.name)
        XCTAssertEqual(Set(store.state.thread.branches(at: kept.id).map(\.text)), ["Friend.", "Pal."])
        XCTAssertNotNil(store.state.prompts[kept.id])
    }

    func testKeepingTheFirstAnswerDropsTheSecond() async throws {
        let store = await comparing(Runs([ScriptedRuntime.answer("Friend."), ScriptedRuntime.answer("Pal.")]))
        await store.ask("Amigo?")
        await store.send(.keepTapped(rival: false))
        XCTAssertNil(store.state.rival)
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])
        let reply = try store.state.thread.messages.last.unwrap()
        XCTAssertEqual(store.state.thread.branches(at: reply.id).count, 1)
    }

    func testSendingAgainGoesOnFromTheFirstAnswer() async throws {
        let store = await comparing(
            Runs([
                ScriptedRuntime.answer("Friend."), ScriptedRuntime.answer("Pal."), ScriptedRuntime.answer("Hello."),
                ScriptedRuntime.answer("Hi."),
            ]))
        await store.ask("Amigo?")
        await store.ask("Hola?")
        XCTAssertEqual(store.texts, ["Amigo?", "Friend.", "Hola?", "Hello."])
        XCTAssertEqual(store.state.rival?.text, "Hi.")
    }

    func testNothingIsComparedWhenTheExperimentIsOff() async throws {
        let runs = Runs([ScriptedRuntime.answer("Friend."), ScriptedRuntime.answer("Pal.")])
        let store = await comparing(runs)
        store.state.$compares.withLock { $0 = false }
        await store.ask("Amigo?")
        XCTAssertNil(store.state.rival)
        XCTAssertEqual(runs.choices.value.count, 1)
    }

    func testStoppingTheSecondModelKeepsWhatItWrote() async throws {
        let writing: ScriptedRuntime.Step = { _, partial throws(ModelRuntimeError) in
            partial(.string("Pal"))
            try? await Task.sleep(for: .seconds(30))
            return .string("Pal")
        }
        let store = await comparing(Runs([ScriptedRuntime.answer("Friend."), writing]))
        await store.send(.binding(.set(\.input, "Amigo?")))
        await store.send(.sendTapped)
        await store.receive(\.replied)
        await store.receive(\.rivalProgressed)
        XCTAssertFalse(store.state.canGo)
        await store.send(.stopTapped)
        await store.settle()
        XCTAssertEqual(store.state.rival?.text, "Pal")
        XCTAssertEqual(store.state.rival?.done, true)
        XCTAssertNil(store.state.rival?.speed)
        XCTAssertTrue(store.state.canGo)
    }

    func testTakingOffTheSecondModelAsksOnlyTheFirst() async throws {
        let runs = Runs([ScriptedRuntime.answer("Friend.")])
        let store = await comparing(runs)
        await store.send(.rivalRemoved)
        XCTAssertNil(store.state.rivalCurrent)

        await store.ask("Amigo?")
        XCTAssertEqual(runs.choices.value.map(\.model), [nil])
        XCTAssertNil(store.state.rival)
        XCTAssertEqual(store.texts, ["Amigo?", "Friend."])
    }
}
