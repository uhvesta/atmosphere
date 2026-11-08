import ComposableArchitecture
import Foundation
import LifrModelsClient
import LifrSettings
import XCTest

@MainActor
final class VoicesFeatureTests: XCTestCase {
    private let supertonic = VoiceCatalog.supertonic3

    private func voices(
        _ library: InMemoryModelLibrary = InMemoryModelLibrary(), selection: VoiceSelection = VoiceSelection(file: nil)
    ) -> TestStoreOf<VoicesFeature> {
        let store = TestStore(initialState: VoicesFeature.State()) {
            VoicesFeature()
        } withDependencies: {
            $0.models = library.client
            $0.speech = .picks(selection)
        }
        store.exhaustivity = .off
        return store
    }

    func testRepliesAreReadInTheSystemVoiceUntilAVoiceIsPicked() async {
        let store = voices()

        await store.send(.task)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.standard, .system)
        XCTAssertEqual(store.state.installed, [])
        await store.skipInFlightEffects()
    }

    func testAVoiceModelDownloadsOnlyWhenTapped() async {
        let library = InMemoryModelLibrary()
        let store = voices(library)
        await store.send(.task)
        await store.receive(\.loaded)
        XCTAssertEqual(library.installed, [])

        await store.send(.downloadTapped(supertonic.id))
        await store.receive(\.downloaded)

        XCTAssertEqual(library.installed, [supertonic.id])
        XCTAssertEqual(store.state.installed, [supertonic.id])
        await store.skipInFlightEffects()
    }

    func testThePickedVoiceIsKept() async {
        let selection = VoiceSelection(file: nil)
        let store = voices(InMemoryModelLibrary(installed: [supertonic.spec]), selection: selection)

        await store.send(.picked(.model(supertonic.id, voice: "M3"), language: nil))
        await store.finish()

        XCTAssertEqual(selection.choice(for: "en"), .model(supertonic.id, voice: "M3"))
    }

    func testRemovingTheVoiceModelGoesBackToTheSystemVoice() async {
        let selection = VoiceSelection(file: nil)
        selection.choose(.model(supertonic.id, voice: "F1"))
        let library = InMemoryModelLibrary(installed: [supertonic.spec])
        let store = voices(library, selection: selection)
        await store.send(.task)
        await store.receive(\.loaded)

        await store.send(.removeTapped(supertonic.id))
        await store.send(.remove(.presented(.confirmed(supertonic.id))))
        await store.receive(\.removed)
        await store.receive(\.loaded)

        XCTAssertEqual(store.state.standard, .system)
        XCTAssertEqual(library.installed, [])
        await store.skipInFlightEffects()
    }
}
