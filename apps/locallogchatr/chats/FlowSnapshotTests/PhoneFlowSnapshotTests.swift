import ChatFeatures
import ComposableArchitecture
import FlowKit
import InferenceKit
import LocalLogChatrChats
import LocalLogChatrModels
import ModelsFeatures
import SwiftUI
import XCTest

/// Chatting on the iPhone: one reference per step, the accessibility tree
/// and an image. Buttons are tapped; typing, closing sheets, picking in
/// them and the toolbar's buttons go through the store, since a flow's tap
/// can't press a toolbar button.
@MainActor
final class PhoneFlowSnapshotTests: XCTestCase {
    /// No chats, a new one, a question and its reply, then back to the
    /// list with a second chat beside it.
    func testChatting() async throws {
        try await chatting("chatting", style: .light)
    }

    func testChattingInDarkMode() async throws {
        try await chatting("chatting-dark", style: .dark)
    }

    private func chatting(_ name: String, style: UIUserInterfaceStyle, test: String = #function) async throws {
        let chats = Sample.chats()
        let flow = FlowRecorder(name, ChatsView(store: chats), style: style)
        await flow.step("empty", testName: test)
        chats.send(.newTapped)
        await flow.step("new-chat", testName: test)
        try await Sample.ask("A quick pasta recipe?", in: chats)
        await flow.step("replied", testName: test)
        chats.send(.closed)
        try await Sample.ask("What's a monad?", in: chats)
        chats.send(.closed)
        await flow.step("chats", testName: test)
        flow.finish()
    }

    /// A chat's own model from its model button: a downloaded one that
    /// thinks, thinking long, then what it thought above its reply.
    func testPickingAChatsModel() async throws {
        let chats = Sample.chats()
        let flow = FlowRecorder("model", ChatsView(store: chats))
        chats.send(.newTapped)
        guard let id = chats.openID else { throw Sample.Unopened() }
        let chat = { (action: ChatThreadFeature.Action) in _ = chats.send(.chats(.element(id: id, action: action))) }
        chat(.modelTapped)
        try await Sample.waitFor { chats.chats[id: id]?.modelSheet?.models.isEmpty == false }
        await flow.step("sheet")
        chat(.modelSheet(.presented(.binding(.set(\.model, ModelCatalog.qwen35Small.id)))))
        chat(.modelSheet(.presented(.binding(.set(\.choice.thinking, .long)))))
        await flow.step("picked")
        chat(.modelSheet(.dismiss))
        try await Sample.ask("A quick pasta recipe?", in: chats)
        chat(.thinkingToggled(messageID: chats.chats[id: id]?.thread.messages.last?.id))
        await flow.step("thought")
        flow.finish()
    }

    /// A photo from +: the sheet with the camera and recent photos, one
    /// attached, then what the model says about it.
    func testAddingAPhoto() async throws {
        let chats = Sample.chats()
        let flow = FlowRecorder("photo", ChatsView(store: chats))
        let id = try await Sample.seeingChat(in: chats)
        let chat = { (action: ChatThreadFeature.Action) in _ = chats.send(.chats(.element(id: id, action: action))) }
        flow.tap("Add")
        try await Sample.waitFor { chats.chats[id: id]?.recentPhotos.isEmpty == false }
        await flow.step("sheet")
        flow.tap("Photo")
        try await Sample.waitFor { chats.chats[id: id]?.pendingPhotos.isEmpty == false }
        await flow.step("picked")
        chat(.binding(.set(\.addsContext, false)))
        await flow.step("attached")
        try await Sample.ask("What's in this photo?", in: chats)
        await flow.step("replied")
        flow.finish()
    }

    /// Settings › Models, from the list.
    func testSettings() async throws {
        let chats = Sample.chats()
        let flow = FlowRecorder("settings", ChatsView(store: chats))
        chats.send(.settingsTapped)
        await flow.step("settings")
        flow.tap("Models")
        try await Sample.waitFor { chats.settings?.models.choices.count == 3 }
        await flow.step("models")
        flow.finish()
    }
}
