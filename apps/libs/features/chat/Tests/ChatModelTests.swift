import ComposableArchitecture
import InferenceKit
import ModelsClient
import XCTest

@testable import ChatFeatures

@MainActor
final class ChatModelTests: XCTestCase {
    func testAWindowTheNewModelDoesntOfferGoesBackToItsOwn() async {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small])
        let store = TestStore(initialState: ChatModelFeature.State(choice: .defaults, use: .chat)) {
            ChatModelFeature()
        } withDependencies: {
            $0.models = library.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        await store.send(.binding(.set(\.model, ModelCatalog.qwen35Small.id)))
        await store.send(.binding(.set(\.choice.window, 32_768)))

        await store.send(.binding(.set(\.model, .apple)))

        XCTAssertNil(store.state.choice.window)
    }

    func testPickingSkillsTheWayTheAppDoesFollowsTheAppAgain() async {
        let store = TestStore(
            initialState: ChatModelFeature.State(
                choice: .defaults, use: .chat, picksSkills: false, standardPicksSkills: true)
        ) {
            ChatModelFeature()
        }
        store.exhaustivity = .off

        await store.send(.binding(.set(\.picksSkillsHere, true)))

        XCTAssertNil(store.state.picksSkills)
        XCTAssertTrue(store.state.picksSkillsHere)
    }

    func testAnAppWithoutSkillsNeverPicksThem() {
        let state = ChatModelFeature.State(choice: .defaults, use: .chat)

        XCTAssertFalse(state.picksSkillsHere)
    }
}
