import ComposableArchitecture
import LifrAssistantClient
import LifrConversations
import XCTest

/// Stop in a chat on Gemma 4 E2B: the model stops writing at once, so the
/// next message is answered without waiting for the stopped one to end.
@MainActor
final class StopOnRealModelTests: XCTestCase {
    func testStoppingAReplyFreesTheModel() async throws {
        let model = try RealModel()
        let id = try await model.chat()
        // Loaded and warmed, so the timing below is only the model's.
        try await model.send("Say hi.")

        let chat: (ChatFeature.Action) -> Void = {
            model.store.send(.conversations(.chats(.element(id: id, action: $0))))
        }
        chat(.binding(.set(\.input, "Write a long story about a lighthouse keeper, at least two thousand words.")))
        chat(.sendTapped)
        // A structured answer shows when it's whole, so the story is
        // stopped a few seconds in, while the model writes it.
        try await Task.sleep(for: .seconds(3))
        XCTAssertEqual(model.chatState?.canStop, true, "The story was done in 3 seconds")
        chat(.stopTapped)
        XCTAssertEqual(model.chatState?.canStop, false)

        let clock = ContinuousClock()
        let start = clock.now
        try await model.send("Say hi again.")
        let took = clock.now - start
        XCTAssertLessThan(took, .seconds(20), "The next reply waited for the stopped one: \(took)")
        let texts = model.chatState?.conversation.messages.filter { $0.role == .person }.map(\.text)
        XCTAssertEqual(texts?.last, "Say hi again.")
    }
}
