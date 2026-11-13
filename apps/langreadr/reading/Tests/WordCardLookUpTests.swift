import ComposableArchitecture
import Foundation
import InferenceKit
import JSONKit
import LangReadrCore
import LangReadrServices
import XCTest

@testable import LangReadrReading

@MainActor
final class WordCardLookUpTests: XCTestCase {
    private let meaning: JSONValue = .object(["reading": "amigo", "meaning": "friend", "inSentence": "Who's greeted."])

    private func card(known: WordNote? = nil, steps: [ScriptedRuntime.Step]) -> TestStoreOf<WordCardFeature> {
        let store = TestStore(
            initialState: WordCardFeature.State(
                key: WordKey(paragraph: UUID(0), word: "amigo"), word: WordSpan(text: "amigo", start: 5, end: 10),
                language: Language("es"), sentence: "Hola amigo.", known: known)
        ) {
            WordCardFeature()
        } withDependencies: {
            $0.tutor = tutor(steps)
        }
        store.exhaustivity = .off
        return store
    }

    func testAFailedLookUpSaysWhyAndTryingAgainLooksItUp() async {
        let store = card(steps: [ScriptedRuntime.fail(.busy), ScriptedRuntime.answer(meaning)])
        await store.send(.task)
        await store.receive(\.lookedUp)
        XCTAssertNotNil(store.state.problem)
        XCTAssertNil(store.state.note)

        await store.send(.lookUpTapped)
        XCTAssertNil(store.state.problem)
        await store.receive(\.lookedUp)
        XCTAssertEqual(store.state.note?.meaning, "friend")
        XCTAssertFalse(store.state.isLookingUp)
    }

    func testAMeaningKnownFromElsewhereIsLookedUpHereOnlyWhenAsked() async {
        let known = WordNote(word: "amigo", transliteration: nil, meaning: "pal", inSentence: "")
        let store = card(known: known, steps: [ScriptedRuntime.answer(meaning)])
        await store.send(.task)
        XCTAssertNil(store.state.note)
        XCTAssertFalse(store.state.isLookingUp)

        await store.send(.lookUpTapped)
        await store.receive(\.lookedUp)
        XCTAssertEqual(store.state.note?.meaning, "friend")
        XCTAssertNil(store.state.known)
    }
}
