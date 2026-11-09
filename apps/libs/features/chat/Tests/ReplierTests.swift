import ChatKit
import ComposableArchitecture
import Foundation
import ModelsClient
import XCTest

@testable import ChatFeatures

/// An app's own message, which keeps a note its replier adds to a reply.
private struct NotedMessage: ThreadEntry {
    var id: UUID
    var role: ChatRole
    var text: String
    var at: Date
    var files: [ThreadFile] = []
    var photos: [ThreadPhoto] = []
    var thinking: ThreadThinking?
    var model: String?
    var speed: ThreadSpeed?
    var note: String?

    init(person id: UUID, text: String, at: Date, files: [ThreadFile], photos: [ThreadPhoto]) {
        self.id = id
        role = .person
        self.text = text
        self.at = at
        self.files = files
        self.photos = photos
    }

    init(
        reply id: UUID, text: String, at: Date, thinking: ThreadThinking?, model: String?, speed: ThreadSpeed?,
        extra: String?
    ) {
        self.id = id
        role = .assistant
        self.text = text
        self.at = at
        self.thinking = thinking
        self.model = model
        self.speed = speed
        note = extra
    }
}

private enum NoteFailure: Error, CustomStringConvertible, Equatable {
    case busy

    var description: String { "The assistant is busy. Send the message again in a moment." }
}

/// Answers each message with the next of `answers`, adding a note naming
/// the model it was asked for, or fails when there are none left. What it
/// sends, it only says once it's written.
private struct NotingReplier: ChatReplier {
    typealias Message = NotedMessage
    typealias Context = NoContext
    typealias Failure = NoteFailure

    let answers: LockIsolated<[String]>
    let asked = LockIsolated<[ReplyRequest<NotingReplier>]>([])

    func ask(_ request: ReplyRequest<NotingReplier>) -> ReplyAsk<String, NoteFailure> {
        asked.withValue { $0.append(request) }
        let answer = answers.withValue { $0.isEmpty ? nil : $0.removeFirst() }
        let note = request.choice.model?.rawValue ?? "standard"
        let prompt = ReplyPrompt(instructions: request.instructions, prompt: "", photos: 0)
        return ReplyAsk(prompt: prompt) { progress throws(NoteFailure) in
            guard let answer else { throw .busy }
            progress(.text(answer))
            let sent = ReplyPrompt(instructions: "Sent: \(request.instructions)", prompt: answer, photos: 0)
            return ChatReply(text: answer, context: ContextUse(used: 10, window: 100), extra: note, prompt: sent)
        }
    }
}

@MainActor
final class ReplierTests: XCTestCase {
    private func chat(
        _ answers: [String], library: InMemoryModelLibrary = InMemoryModelLibrary()
    ) -> (TestStoreOf<ChatCore<NotingReplier>>, NotingReplier) {
        let replier = NotingReplier(answers: LockIsolated(answers))
        let store = TestStore(
            initialState: ChatCore<NotingReplier>.State(
                thread: Conversation(id: UUID(999), createdAt: epoch), use: .chat, instructions: "You keep notes.")
        ) {
            ChatCore(replier: replier)
        } withDependencies: {
            $0.models = library.client
            $0.date.now = epoch
            $0.uuid = .incrementing
        }
        store.exhaustivity = .off
        return (store, replier)
    }

    func testAnAppsReplierWritesTheRepliesAndAddsToThem() async throws {
        let (store, replier) = chat(["Noted."])
        await store.send(.binding(.set(\.input, "Remember milk.")))
        await store.send(.sendTapped)
        await store.receive(\.replied)
        XCTAssertEqual(store.state.thread.messages.map(\.text), ["Remember milk.", "Noted."])
        XCTAssertEqual(store.state.thread.messages.last?.note, "standard")
        XCTAssertEqual(replier.asked.value.first?.instructions, "You keep notes.")
        XCTAssertEqual(replier.asked.value.first?.chat.messages.map(\.text), ["Remember milk."])
    }

    func testWhatTheReplierSaysItSentIsThePromptShown() async throws {
        let (store, _) = chat(["Noted."])
        await store.send(.binding(.set(\.input, "Remember milk.")))
        await store.send(.sendTapped)
        await store.receive(\.replied)
        let reply = try store.state.thread.messages.last.unwrap()
        await store.send(.promptTapped(messageID: reply.id))
        XCTAssertEqual(store.state.shownPromptValue?.instructions, "Sent: You keep notes.")
        XCTAssertEqual(store.state.shownPromptValue?.context, ContextUse(used: 10, window: 100))
    }

    func testAFailedReplySaysWhyInTheRepliersWords() async throws {
        let (store, _) = chat([])
        await store.send(.binding(.set(\.input, "Remember milk.")))
        await store.send(.sendTapped)
        await store.receive(\.replied)
        XCTAssertEqual(store.state.thread.messages.map(\.text), ["Remember milk."])
        XCTAssertEqual(store.state.problem, NoteFailure.busy.description)
        XCTAssertNil(store.state.reply)
    }

    func testKeepingTheSecondModelsAnswerKeepsWhatItsReplierAdded() async throws {
        let qwen = ModelCatalog.qwen35Small.id
        let (store, _) = chat(
            ["Noted.", "Got it."], library: InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]))
        store.state.$compares.withLock { $0 = true }
        await store.send(.rivalTapped)
        await store.send(.rivalSheet(.presented(.task)))
        await store.receive(\.rivalSheet.presented.loaded)
        await store.send(.rivalSheet(.presented(.binding(.set(\.model, qwen)))))
        await store.send(.rivalSheet(.dismiss))
        await store.send(.binding(.set(\.input, "Remember milk.")))
        await store.send(.sendTapped)
        await store.receive(\.replied)
        await store.receive(\.rivalReplied)
        await store.send(.keepTapped(rival: true))
        let kept = try store.state.thread.messages.last.unwrap()
        XCTAssertEqual(kept.text, "Got it.")
        XCTAssertEqual(kept.note, qwen.rawValue)
    }
}
