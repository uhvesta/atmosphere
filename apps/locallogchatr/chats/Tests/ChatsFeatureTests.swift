import ChatFeatures
import ChatKit
import ComposableArchitecture
import Foundation
import InferenceKit
import JSONKit
import LocalLogChatrModels
import XCTest

@testable import LocalLogChatrChats

private let epoch = Date(timeIntervalSince1970: 1_793_700_000)

private struct Missing: Error {}

extension Optional {
    fileprivate func unwrap() throws -> Wrapped {
        guard let self else { throw Missing() }
        return self
    }
}

@MainActor
final class ChatsFeatureTests: XCTestCase {
    private func chats(
        _ shelf: ChatShelfClient = .inMemory(), answers: [String] = ["Hello! How can I help?"],
        photos: ChatPhotosClient = .inMemory()
    ) -> TestStoreOf<ChatsFeature> {
        let store = TestStore(initialState: ChatsFeature.State()) {
            ChatsFeature()
        } withDependencies: {
            $0.chatShelf = shelf
            $0.chatRuntime = .always(ScriptedRuntime(steps: answers.map { ScriptedRuntime.answer(.string($0)) }))
            $0.date.now = epoch
            $0.uuid = .incrementing
            $0.chatPhotos = photos
        }
        store.exhaustivity = .off
        return store
    }

    private func saved(_ text: String, at: Date = epoch) -> SavedChat {
        var thread = ChatThread(id: UUID(), createdAt: at)
        thread.append(ThreadMessage(role: .person, text: text, at: at))
        return SavedChat(conversation: thread)
    }

    private func ask(_ text: String, in id: UUID, of store: TestStoreOf<ChatsFeature>) async {
        await store.send(.chats(.element(id: id, action: .binding(.set(\.input, text)))))
        await store.send(.chats(.element(id: id, action: .sendTapped)))
        await store.receive(\.chats[id: id].replied)
        await store.finish()
        await store.skipReceivedActions(strict: false)
    }

    func testTheKeptChatsShowTheOneWrittenInLastFirst() async {
        let old = saved("Old", at: epoch)
        let new = saved("New", at: epoch.addingTimeInterval(60))
        let store = chats(.inMemory([old, new]))
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(store.state.shown.map(\.id), [new.id, old.id])
    }

    func testANewChatIsOpenedAndKeptOnceItHasAMessage() async throws {
        let shelf = ChatShelfClient.inMemory()
        let store = chats(shelf)
        await store.send(.newTapped)
        let id = try store.state.openID.unwrap()
        XCTAssertTrue(shelf.chats().chats.isEmpty, "An empty chat isn't kept")

        await ask("Hi", in: id, of: store)
        let kept = try shelf.chats().chats.first.unwrap()
        XCTAssertEqual(kept.conversation.messages.map(\.text), ["Hi", "Hello! How can I help?"])
    }

    func testNewChatOpensTheEmptyOneRatherThanMakingAnother() async {
        let store = chats()
        await store.send(.newTapped)
        let first = store.state.openID
        await store.send(.closed)
        await store.send(.newTapped)
        XCTAssertEqual(store.state.openID, first)
        XCTAssertEqual(store.state.chats.count, 1)
    }

    func testAChatPicksItsOwnModelAndKeepsIt() async throws {
        let shelf = ChatShelfClient.inMemory()
        let store = chats(shelf)
        await store.send(.newTapped)
        let id = try store.state.openID.unwrap()
        await store.send(.chats(.element(id: id, action: .modelTapped)))
        await store.send(
            .chats(.element(id: id, action: .modelSheet(.presented(.binding(.set(\.choice.thinking, .long)))))))
        XCTAssertEqual(store.state.chats[id: id]?.model.thinking, .long)

        await ask("Hi", in: id, of: store)
        XCTAssertEqual(try shelf.chats().chats.first.unwrap().model.thinking, .long)
    }

    func testARenamedChatKeepsItsNameAndAnEmptyNameGoesBackToItsFirstMessage() async throws {
        let chat = saved("What's a monad?")
        let shelf = ChatShelfClient.inMemory([chat])
        let store = chats(shelf)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.renameTapped(chat.id))
        await store.send(.binding(.set(\.name, "Monads")))
        await store.send(.renameSaved)
        await store.finish()
        XCTAssertEqual(store.state.chats[id: chat.id]?.thread.title, "Monads")
        XCTAssertEqual(try shelf.chats().chats.first.unwrap().conversation.title, "Monads")

        await store.send(.renameTapped(chat.id))
        await store.send(.binding(.set(\.name, "")))
        await store.send(.renameSaved)
        XCTAssertEqual(store.state.chats[id: chat.id]?.thread.title, "What's a monad?")
    }

    func testDeletingAChatWaitsForTheConfirmation() async throws {
        let chat = saved("Hi")
        let shelf = ChatShelfClient.inMemory([chat])
        let store = chats(shelf)
        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.openTapped(chat.id))

        await store.send(.deleteTapped(chat.id))
        await store.send(.deleteCancelled)
        XCTAssertEqual(store.state.chats.count, 1)

        await store.send(.deleteTapped(chat.id))
        await store.send(.deleteConfirmed)
        await store.finish()
        XCTAssertTrue(store.state.chats.isEmpty)
        XCTAssertNil(store.state.openID)
        XCTAssertTrue(shelf.chats().chats.isEmpty)
    }

    func testDeletingAChatDeletesItsPhotos() async throws {
        let photo = ThreadPhoto(id: UUID())
        let chat = saved("Hi")
        let kept = LockIsolated<Set<UUID>>([chat.id])
        let photos = ChatPhotosClient(
            save: { _, _ in photo },
            file: { photo, chat in kept.value.contains(chat) ? URL(filePath: "/photos/\(photo.id)") : nil },
            remove: { _, _ in }, removeAll: { chat in kept.withValue { _ = $0.remove(chat) } })
        let store = chats(.inMemory([chat]), photos: photos)
        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.deleteTapped(chat.id))
        await store.send(.deleteConfirmed)
        await store.finish()
        XCTAssertNil(photos.file(photo, chat.id))
    }

    func testSearchingFindsChatsByWhatWasWritten() async {
        let monads = saved("What's a monad?")
        let pasta = saved("A quick pasta recipe")
        let store = chats(.inMemory([monads, pasta]))
        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.binding(.set(\.query, "monad")))
        XCTAssertEqual(store.state.shown.map(\.id), [monads.id])
    }

    func testAChatThatCantBeSavedSaysWhy() async throws {
        let store = chats(.full())
        await store.send(.newTapped)
        let id = try store.state.openID.unwrap()
        await ask("Hi", in: id, of: store)
        XCTAssertNotNil(store.state.problem)
    }

    func testAKeptChatRoundTripsOnDisk() throws {
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: folder) }
        let shelf = ChatShelf(folder: folder)
        var chat = saved("Hi")
        chat.model = ModelChoice(thinking: .on)
        try shelf.save(chat)
        XCTAssertEqual(shelf.chats().chats, [chat])
        try shelf.delete(chat)
        XCTAssertTrue(shelf.chats().chats.isEmpty)
    }
}
