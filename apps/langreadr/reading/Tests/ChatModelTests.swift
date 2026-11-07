import ComposableArchitecture
import Foundation
import InferenceKit
import LangReadrCore
import LangReadrModels
import LangReadrServices
import LangReadrServicesFake
import XCTest

@testable import LangReadrReading

/// Records the use and choice each model is asked for, and runs them all
/// on one scripted model.
private final class Runs: Sendable {
    let runtime: ScriptedRuntime
    let asked = LockIsolated<[ModelUse]>([])
    let choices = LockIsolated<[ModelChoice]>([])

    init(_ steps: [ScriptedRuntime.Step]) {
        runtime = ScriptedRuntime(steps: steps)
    }

    var tutor: Tutor {
        Tutor(
            runtime: { [self] use, choice in
                asked.withValue { $0.append(use) }
                choices.withValue { $0.append(choice) }
                return runtime
            }, reader: Language("en"))
    }
}

@MainActor
final class ChatModelTests: XCTestCase {
    private let qwen = ModelCatalog.qwen35Small.id

    private func chat(
        _ runs: Runs, picked: ChatModel = ChatModel(), library: InMemoryModelLibrary = InMemoryModelLibrary()
    ) throws -> TestStoreOf<PassageChatFeature> {
        let book = try spanish("Hola amigo.")
        let passage = try Passage(in: try book.paragraphs.first.unwrap(), from: 0, to: 4).unwrap()
        var anchored = AnchoredChat(passage: passage, createdAt: epoch)
        anchored.model = picked
        let store = TestStore(
            initialState: PassageChatFeature.State(
                chat: anchored, book: ChatBook(book, chat: anchored), compact: true)
        ) {
            PassageChatFeature()
        } withDependencies: {
            $0.tutor = runs.tutor
            $0.models = library.client
            $0.date.now = epoch
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return store
    }

    private func ask(_ text: String, in store: TestStoreOf<PassageChatFeature>) async {
        await store.send(.binding(.set(\.input, text)))
        await store.send(.sendTapped)
        await store.receive(\.replied)
        await store.settle()
    }

    func testAChatsModelThinkingAndWindowAreKeptAndRunTheNextReply() async throws {
        let runs = Runs([ScriptedRuntime.answer("Hello."), ScriptedRuntime.answer("Friend.")])
        let store = try chat(runs, library: InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]))

        await store.send(.modelTapped)
        await store.send(.model(.presented(.task)))
        await store.receive(\.model.presented.loaded)
        XCTAssertEqual(store.state.model?.models.map(\.id), [.apple, qwen])
        await store.send(.model(.presented(.binding(.set(\.model, qwen)))))
        await store.send(.model(.presented(.binding(.set(\.choice.thinking, .long)))))
        await store.send(.model(.presented(.binding(.set(\.choice.window, 16_384)))))
        await store.send(.model(.dismiss))
        XCTAssertEqual(store.state.chat.model, ChatModel(model: qwen.rawValue, thinking: "long", window: 16_384))
        await ask("Hola?", in: store)

        await store.send(.modelTapped)
        await store.send(.model(.presented(.binding(.set(\.choice.thinking, .off)))))
        await store.send(.model(.dismiss))
        await ask("Amigo?", in: store)

        XCTAssertEqual(runs.asked.value, [.chat, .chat])
        XCTAssertEqual(
            runs.choices.value,
            [
                ModelChoice(model: qwen, thinking: .long, window: 16_384),
                ModelChoice(model: qwen, thinking: .off, window: 16_384),
            ])
        XCTAssertEqual(runs.runtime.turns.map(\.thinkingTokens).first, ThinkingLevel.long.tokens)
        XCTAssertEqual(runs.runtime.turns.map { $0.thinking != nil }, [true, false])
    }

    func testUsingTheDefaultsAgainForgetsWhatWasPicked() async throws {
        let store = try chat(Runs([]), picked: ChatModel(model: qwen.rawValue, thinking: "off", window: 16_384))
        await store.send(.modelTapped)
        await store.send(.model(.presented(.resetTapped)))
        await store.send(.model(.dismiss))
        XCTAssertEqual(store.state.chat.model, ChatModel())
    }

    func testAChatOnAModelThatWasDeletedRunsOnTheDefault() async throws {
        let store = try chat(Runs([]), picked: ChatModel(model: qwen.rawValue))
        await store.send(.modelTapped)
        await store.send(.model(.presented(.task)))
        await store.receive(\.model.presented.loaded)
        XCTAssertNil(store.state.model?.choice.model)
        XCTAssertEqual(store.state.model?.current?.id, .apple)
    }

    func testTranslatingRunsOnTheReadingModelWhateverAChatPicked() async throws {
        let runs = Runs([ScriptedRuntime.answer("Hello.")])
        _ = try await runs.tutor.translate("Hola.", from: Language("es"))
        XCTAssertEqual(runs.asked.value, [.reading])
        XCTAssertEqual(runs.choices.value, [.defaults])
    }

    func testTheLibraryOpensSettingsWithItsModels() async throws {
        let store = TestStore(initialState: LibraryFeature.State(books: [])) {
            LibraryFeature()
        } withDependencies: {
            $0.shelf = .inMemory()
            $0.readerServices = .fake()
            $0.tutor = tutor([])
            $0.models = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]).client
        }
        store.exhaustivity = .off
        await store.send(.settingsTapped)
        await store.send(.settings(.presented(.models(.task))))
        await store.receive(\.settings.presented.models.loaded)
        XCTAssertEqual(store.state.settings?.models.choices.map(\.id), [.apple, qwen])
        await store.send(.settings(.presented(.doneTapped)))
        await store.receive(\.settings.dismiss)
        XCTAssertNil(store.state.settings)
    }
}
