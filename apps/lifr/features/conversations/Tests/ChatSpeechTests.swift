import ComposableArchitecture
import Foundation
import LifrAssistantClient
import LifrConversations
import LifrCore
import LifrModelsClient
import LifrStoreClient
import XCTest

@MainActor
final class ChatSpeechTests: XCTestCase {
    private let now = Date(timeIntervalSince1970: 1_793_700_000)
    private let reply = UUID(2)

    private func chat(
        _ text: String = "You have **three** tasks due today.",
        speak: @escaping @Sendable (String, String) async throws(SpeechError) -> Void,
        stop: @escaping @Sendable () async -> Void = {}
    ) -> TestStoreOf<ChatFeature> {
        var conversation = Conversation(id: UUID(0), createdAt: now)
        conversation.messages = [
            ChatMessage(id: UUID(1), role: .person, text: "What's due?", at: now),
            ChatMessage(id: reply, role: .assistant, text: text, at: now),
        ]
        let store = TestStore(
            initialState: ChatFeature.State(
                conversation: conversation, workspace: RecordsBuilder(now: now, timeZone: .gmt).workspace)
        ) {
            ChatFeature()
        } withDependencies: {
            $0.speech.speak = speak
            $0.speech.stop = stop
        }
        store.exhaustivity = .off
        return store
    }

    func testSpeakReadsTheReplysWordsWithoutItsMarkdownInItsLanguage() async {
        let said = LockIsolated<[String]>([])
        let store = chat { text, language in said.withValue { $0.append("\(language): \(text)") } }

        await store.send(.speakTapped(messageID: reply)) { $0.speaking = self.reply }
        await store.receive(\.spoke) { $0.speaking = nil }

        XCTAssertEqual(said.value, ["en: You have three tasks due today."])
        XCTAssertNil(store.state.alert)
    }

    func testTappingAgainWhileItsReadStopsIt() async {
        let stopped = LockIsolated(false)
        let store = chat(
            speak: { _, _ throws(SpeechError) in try? await Task.never() },
            stop: { stopped.setValue(true) })

        await store.send(.speakTapped(messageID: reply)) { $0.speaking = self.reply }
        await store.send(.speakTapped(messageID: reply)) { $0.speaking = nil }
        await store.finish()

        XCTAssertTrue(stopped.value)
    }

    func testWhyItCouldntBeReadIsShown() async {
        let store = chat { _, _ throws(SpeechError) in throw .systemVoiceFailed(language: "en") }

        await store.send(.speakTapped(messageID: reply))
        await store.receive(\.spoke) { $0.speaking = nil }

        XCTAssertNotNil(store.state.alert)
    }

    func testAPersonsMessageIsntRead() async {
        let said = LockIsolated<[String]>([])
        let store = chat { text, _ in said.withValue { $0.append(text) } }

        await store.send(.speakTapped(messageID: UUID(1)))
        await store.finish()

        XCTAssertEqual(said.value, [])
    }
}
