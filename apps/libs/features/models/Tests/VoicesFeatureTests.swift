import ComposableArchitecture
import Foundation
import ModelsClient
import ModelsFeatures
import XCTest

@MainActor
final class VoicesFeatureTests: XCTestCase {
    private let supertonic = VoiceCatalog.supertonic3

    private func voices(
        languages: [String] = ["es", "ja"], library: InMemoryModelLibrary = InMemoryModelLibrary(),
        selection: VoiceSelection = VoiceSelection(file: nil)
    ) -> TestStoreOf<VoicesFeature> {
        let store = TestStore(initialState: VoicesFeature.State(languages: languages)) {
            VoicesFeature()
        } withDependencies: {
            $0.models = library.client
            $0.speech = .picks(selection)
        }
        store.exhaustivity = .off
        return store
    }

    func testEveryLanguageStartsOnTheSystemVoiceWithNothingDownloaded() async {
        let store = voices()

        await store.send(.task)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.standard, .system)
        XCTAssertEqual(store.state.own, [:])
        XCTAssertEqual(store.state.installed, [])
        XCTAssertEqual(store.state.models.map(\.id), [supertonic.id])
        await store.skipInFlightEffects()
    }

    func testAVoiceModelDownloadsOnlyWhenTapped() async {
        let library = InMemoryModelLibrary()
        let store = voices(library: library)
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(library.installed, [])

        await store.send(.downloadTapped(supertonic.id))
        await store.receive(\.downloaded)

        XCTAssertEqual(store.state.installed, [supertonic.id])
        XCTAssertEqual(library.installed, [supertonic.id])
        await store.skipInFlightEffects()
    }

    func testPickingAVoiceForEveryLanguageAndOneLanguageIsKept() async {
        let selection = VoiceSelection(file: nil)
        let store = voices(library: InMemoryModelLibrary(installed: [supertonic.spec]), selection: selection)

        await store.send(.picked(.model(supertonic.id, voice: "F2"), language: nil))
        await store.send(.picked(.system, language: "ja"))
        await store.finish()

        XCTAssertEqual(store.state.standard, .model(supertonic.id, voice: "F2"))
        XCTAssertEqual(store.state.own, ["ja": .system])
        XCTAssertEqual(selection.choice(for: "es"), .model(supertonic.id, voice: "F2"))
        XCTAssertEqual(selection.choice(for: "ja"), .system)
    }

    func testALanguageCanGoBackToFollowingTheOthers() async {
        let selection = VoiceSelection(file: nil)
        selection.choose(.system, for: "ja")
        let store = voices(selection: selection)

        await store.send(.picked(nil, language: "ja"))
        await store.finish()

        XCTAssertNil(store.state.own["ja"])
        XCTAssertFalse(selection.hasOwnChoice(for: "ja"))
    }

    func testRemovingAVoiceModelSendsItsPicksBackToTheSystemVoice() async {
        let selection = VoiceSelection(file: nil)
        selection.choose(.model(supertonic.id, voice: "M1"))
        let library = InMemoryModelLibrary(installed: [supertonic.spec])
        let store = voices(library: library, selection: selection)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.removeTapped(supertonic.id))
        await store.send(.remove(.presented(.confirmed(supertonic.id))))
        await store.receive(\.removed)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.installed, [])
        XCTAssertEqual(store.state.standard, .system)
        XCTAssertEqual(library.installed, [])
        await store.skipInFlightEffects()
    }
}
