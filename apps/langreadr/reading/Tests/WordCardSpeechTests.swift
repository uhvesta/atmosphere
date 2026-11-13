import ComposableArchitecture
import Foundation
import LangReadrCore
import LangReadrModels
import LangReadrServices
import XCTest

@testable import LangReadrReading

@MainActor
final class WordCardSpeechTests: XCTestCase {
    private func card(
        speak: @escaping @Sendable (String, String) async throws(SpeechError) -> Void,
        slowly: @escaping @Sendable (String, String) async throws(SpeechError) -> Void = { _, _ throws(SpeechError) in
        }
    ) -> TestStoreOf<WordCardFeature> {
        let word = WordSpan(text: "amigo", start: 5, end: 10)
        let store = TestStore(
            initialState: WordCardFeature.State(
                key: WordKey(paragraph: UUID(0), word: "amigo"), word: word, language: Language("es-MX"),
                sentence: "Hola amigo.")
        ) {
            WordCardFeature()
        } withDependencies: {
            $0.speech.speak = speak
            $0.speech.speakSlowly = slowly
        }
        store.exhaustivity = .off
        return store
    }

    func testTheWordAndItsSentenceAreSaidInTheBooksLanguage() async {
        let said = LockIsolated<[String]>([])
        let store = card { text, language in said.withValue { $0.append("\(language): \(text)") } }

        await store.send(.sayTapped)
        await store.send(.saySentenceTapped)
        await store.finish()

        XCTAssertEqual(said.value, ["es-MX: amigo", "es-MX: Hola amigo."])
        XCTAssertNil(store.state.speechProblem)
    }

    func testTheWordAndItsSentenceCanBeSaidSlowly() async {
        let said = LockIsolated<[String]>([])
        let store = card(
            speak: { _, _ in XCTFail("Said at the usual pace") },
            slowly: { text, _ in said.withValue { $0.append(text) } })

        await store.send(.saySlowlyTapped)
        await store.send(.saySentenceSlowlyTapped)
        await store.finish()

        XCTAssertEqual(said.value, ["amigo", "Hola amigo."])
    }

    func testWhyItCouldntBeSaidIsShown() async {
        let store = card { _, _ throws(SpeechError) in throw .audioUnavailable(reason: "busy") }

        await store.send(.sayTapped)
        await store.receive(\.spoke)

        XCTAssertNotNil(store.state.speechProblem)
    }
}
