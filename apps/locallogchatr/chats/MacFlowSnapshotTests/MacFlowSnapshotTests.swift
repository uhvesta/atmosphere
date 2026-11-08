import ChatFeatures
import ComposableArchitecture
import FlowKit
import InferenceKit
import LocalLogChatrChats
import LocalLogChatrModels
import ModelsFeatures
import SwiftUI
import XCTest

/// Chatting on the Mac, the chats beside the open one. The Mac flow drives
/// the store, since it can't tap.
@MainActor
final class MacFlowSnapshotTests: XCTestCase {
    func testChatting() async throws {
        let chats = Sample.chats()
        let flow = FlowRecorder("chatting", ChatsView(store: chats))
        await flow.step("empty")
        chats.send(.newTapped)
        await flow.step("new-chat")
        try await Sample.ask("A quick pasta recipe?", in: chats)
        await flow.step("replied")
        chats.send(.newTapped)
        try await Sample.ask("What's a monad?", in: chats)
        await flow.step("second-chat")
        flow.finish()
    }

    /// A photo attached from + on a model that reads photos, then what the
    /// model says about it.
    func testAddingAPhoto() async throws {
        let chats = Sample.chats()
        let flow = FlowRecorder("photo", ChatsView(store: chats))
        let id = try await Sample.seeingChat(in: chats)
        chats.send(.chats(.element(id: id, action: .photoPicked(Sample.recents.first?.thumbnail))))
        try await Sample.waitFor { chats.chats[id: id]?.pendingPhotos.isEmpty == false }
        await flow.step("attached")
        try await Sample.ask("What's in this photo?", in: chats)
        await flow.step("replied")
        flow.finish()
    }

    /// A chat's own model, thinking long, then what it thought.
    func testPickingAChatsModel() async throws {
        let chats = Sample.chats()
        let flow = FlowRecorder("model", ChatsView(store: chats))
        chats.send(.newTapped)
        guard let id = chats.openID else { throw Sample.Unopened() }
        let chat = { (action: ChatThreadFeature.Action) in _ = chats.send(.chats(.element(id: id, action: action))) }
        chat(.modelTapped)
        try await Sample.waitFor { chats.chats[id: id]?.modelSheet?.models.isEmpty == false }
        chat(.modelSheet(.presented(.binding(.set(\.model, ModelCatalog.qwen35Small.id)))))
        chat(.modelSheet(.presented(.binding(.set(\.choice.thinking, .long)))))
        await flow.step("picked")
        chat(.modelSheet(.dismiss))
        try await Sample.ask("A quick pasta recipe?", in: chats)
        chat(.thinkingToggled(messageID: chats.chats[id: id]?.thread.messages.last?.id))
        await flow.step("thought")
        flow.finish()
    }
}
