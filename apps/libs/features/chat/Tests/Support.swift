import CaptureKit
import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit
import ModelsClient

@testable import ChatFeatures

/// What a test app asks models to do: chatting, and naming chats, which
/// follows the chat's model.
enum TestUse: String, ModelUseCase {
    case chat, titles

    func model(in selection: [TestUse: ModelID]) -> ModelID {
        selection[self] ?? (self == .chat ? .apple : TestUse.chat.model(in: selection))
    }

    var standard: ModelID? { self == .chat ? .apple : nil }
}

extension ModelUseKey {
    static let chat = Self(TestUse.chat)
}

let epoch = Date(timeIntervalSince1970: 0)

/// Records the use and choice each reply is asked for, and runs them all
/// on one scripted model.
final class Runs: Sendable {
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

extension InMemoryModelLibrary {
    convenience init(installed: [ModelSpec] = []) {
        self.init(uses: TestUse.self, catalog: ModelCatalog.all, installed: installed, links: [:])
    }
}

@MainActor
func chat(
    _ runs: Runs, thread: ChatThread = ChatThread(id: UUID(999), createdAt: epoch), model: ModelChoice = .defaults,
    library: InMemoryModelLibrary = InMemoryModelLibrary(), dictation: DictationClient = .scripted(),
    files: [String: PickedFile] = [:], speech: SpeechClient = .inMemory
) -> TestStoreOf<ChatThreadFeature> {
    let store = TestStore(
        initialState: ChatThreadFeature.State(
            thread: thread, model: model, use: .chat, instructions: "You help with Spanish.")
    ) {
        ChatThreadFeature()
    } withDependencies: {
        $0.chatRuntime = runs.client
        $0.models = library.client
        $0.date.now = epoch
        $0.uuid = .incrementing
        $0.dictation = dictation
        $0.fileReader = .scripted(files)
        $0.speech = speech
    }
    store.exhaustivity = .off
    return store
}

extension TestStoreOf<ChatThreadFeature> {
    /// Sends `text` and waits for its reply.
    func ask(_ text: String) async {
        await send(.binding(.set(\.input, text)))
        await send(.sendTapped)
        await receive(\.replied)
        await settle()
    }

    var texts: [String] { state.thread.messages.map(\.text) }
}

struct Missing: Error {}

extension Optional {
    func unwrap() throws -> Wrapped {
        guard let self else { throw Missing() }
        return self
    }
}

extension TestStore {
    /// Runs every effect to its end, and the actions they sent.
    func settle() async {
        await finish()
        await skipReceivedActions(strict: false)
    }
}
