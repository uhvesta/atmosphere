import ChatFeatures
import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrModels
import LangReadrServicesFake
import XCTest

@testable import LangReadrReading

/// Records the use and choice each chat asks for, and runs them all on one
/// scripted model.
private final class Runs: Sendable {
    let runtime: ScriptedRuntime
    let asked = LockIsolated<[ModelUseKey]>([])
    let choices = LockIsolated<[ModelChoice]>([])

    init(_ steps: [ScriptedRuntime.Step]) {
        runtime = ScriptedRuntime(steps: steps)
    }

    var client: ChatRuntimeClient {
        ChatRuntimeClient { [self] use, choice in
            asked.withValue { $0.append(use) }
            choices.withValue { $0.append(choice) }
            return runtime
        }
    }
}

@MainActor
final class PassageChatTests: XCTestCase {
    private let qwen = ModelCatalog.qwen35Small.id

    private func chat(
        _ runs: Runs = Runs([ScriptedRuntime.answer("It means friend.")]), picked: ChatModel = ChatModel(),
        library: InMemoryModelLibrary = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small])
    ) throws -> TestStoreOf<PassageChatFeature> {
        let book = try spanish("Hola amigo.")
        let passage = try Passage(in: try book.paragraphs.first.unwrap(), from: 5, to: 10).unwrap()
        var anchored = AnchoredChat(passage: passage, createdAt: epoch)
        anchored.model = picked
        let store = TestStore(
            initialState: PassageChatFeature.State(
                chat: anchored, book: ChatBook(book, chat: anchored), compact: false, reader: Language("en"))
        ) {
            PassageChatFeature()
        } withDependencies: {
            $0.chatRuntime = runs.client
            $0.models = library.client
            $0.date.now = epoch
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    private func ask(_ text: String, in store: TestStoreOf<PassageChatFeature>) async {
        await store.send(.thread(.binding(.set(\.input, text))))
        await store.send(.thread(.sendTapped))
        await store.receive(\.thread.replied)
        await store.finish()
        await store.skipReceivedActions(strict: false)
    }

    func testAReplyIsKeptWithThePassage() async throws {
        let store = try chat()
        await ask("What does amigo mean?", in: store)
        XCTAssertEqual(
            store.state.chat.conversation.messages.map(\.text), ["What does amigo mean?", "It means friend."])
    }

    func testTheModelIsToldThePassageAndAnswersInTheReadersLanguage() async throws {
        let runs = Runs([ScriptedRuntime.answer("It means friend.")])
        let store = try chat(runs)
        await ask("What does amigo mean?", in: store)
        let turn = try runs.runtime.turns.first.unwrap()
        XCTAssertTrue(turn.instructions.contains("amigo"))
        XCTAssertTrue(turn.prompt.contains("What does amigo mean?"))
        XCTAssertEqual(runs.asked.value, [ModelUseKey(ModelUse.chat)])
    }

    func testAChatRunsOnTheModelItPicked() async throws {
        let runs = Runs([ScriptedRuntime.answer("It means friend.")])
        let store = try chat(runs, picked: ChatModel(model: qwen.rawValue, thinking: "long", window: 8_192))
        await ask("What does amigo mean?", in: store)
        XCTAssertEqual(runs.choices.value, [ModelChoice(model: qwen, thinking: .long, window: 8_192)])
    }

    func testAModelPickedInTheChatIsKeptWithIt() async throws {
        let store = try chat()
        await store.send(.thread(.modelTapped))
        await store.send(.thread(.modelSheet(.presented(.task))))
        await store.receive(\.thread.modelSheet.presented.loaded)
        await store.send(.thread(.modelSheet(.presented(.binding(.set(\.model, qwen))))))
        await store.send(.thread(.modelSheet(.presented(.binding(.set(\.choice.thinking, .long))))))
        await store.skipReceivedActions(strict: false)
        XCTAssertEqual(store.state.chat.model, ChatModel(model: qwen.rawValue, thinking: "long"))
    }
}
