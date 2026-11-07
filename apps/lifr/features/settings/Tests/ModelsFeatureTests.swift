import ComposableArchitecture
import Foundation
import LifrModelsClient
import LifrSettings
import XCTest

@MainActor
final class ModelsFeatureTests: XCTestCase {
    private let pasted = ModelSpec(
        id: "someone/tiny-litert-lm", name: "tiny-litert-lm", summary: "From Hugging Face", runtime: .liteRT,
        source: ModelSource(
            repo: "someone/tiny-litert-lm", commit: String(repeating: "a", count: 40),
            files: [ModelFile(path: "tiny.litertlm", size: 1_000, digest: .sha256(String(repeating: "b", count: 64)))]
        ), contextTokens: 4_096, memoryBytes: 0, license: "MIT")

    private let link = "https://huggingface.co/someone/tiny-litert-lm"

    private func models(_ library: InMemoryModelLibrary) async -> TestStoreOf<ModelsFeature> {
        let store = TestStore(initialState: ModelsFeature.State()) {
            ModelsFeature()
        } withDependencies: {
            $0.models = library.client
        }
        store.exhaustivity = .off
        await store.send(.task)
        await store.receive(\.loaded)
        await store.receive(\.downloadsChanged)
        return store
    }

    func testListsTheCatalogAndWhatEachUseRunsOn() async {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B])
        await library.client.select(ModelCatalog.gemma4E2B.id, .assistant)
        let store = await models(library)

        XCTAssertEqual(store.state.downloadable.map(\.id), [ModelCatalog.gemma4E2B.id, ModelCatalog.gemma4E4B.id])
        XCTAssertEqual(store.state.choices.map(\.id), [.apple, ModelCatalog.gemma4E2B.id])
        XCTAssertEqual(store.state.selected(for: .assistant), ModelCatalog.gemma4E2B.id)
        XCTAssertEqual(store.state.selected(for: .titles), ModelCatalog.gemma4E2B.id)
        await store.skipInFlightEffects()
    }

    func testUsesNotPickedFollowTheAssistantsModel() async {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B])
        let store = await models(library)
        store.exhaustivity = .off

        await store.send(.picked(ModelCatalog.gemma4E2B.id, for: .assistant))
        await store.receive(\.selectionChanged)
        XCTAssertEqual(store.state.selected(for: .titles), ModelCatalog.gemma4E2B.id)

        await store.send(.picked(.apple, for: .titles))
        await store.receive(\.selectionChanged)
        XCTAssertEqual(store.state.selected(for: .titles), .apple)
        XCTAssertEqual(store.state.selected(for: .skills), ModelCatalog.gemma4E2B.id)
        await store.skipInFlightEffects()
    }

    func testPicksTheModelForAUse() async {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B])
        let store = await models(library)

        await store.send(.picked(ModelCatalog.gemma4E2B.id, for: .skills))
        for _ in 0..<100 where library.client.selection(.skills) == .apple { await Task.yield() }

        XCTAssertEqual(library.client.selection(.skills), ModelCatalog.gemma4E2B.id)
        XCTAssertEqual(library.client.selection(.assistant), .apple)
        await store.skipReceivedActions()
        await store.skipInFlightEffects()
    }

    func testPicksTheWindowADownloadedModelRunsAt() async {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B])
        let store = await models(library)
        XCTAssertEqual(store.state.windows[ModelCatalog.gemma4E2B.id], ModelCatalog.gemma4E2B.contextTokens)

        await store.send(.windowPicked(32_768, for: ModelCatalog.gemma4E2B.id))
        let window = { await library.client.installed().first?.window }
        for _ in 0..<100 where await window() != 32_768 { await Task.yield() }

        XCTAssertEqual(store.state.windows[ModelCatalog.gemma4E2B.id], 32_768)
        let saved = await window()
        XCTAssertEqual(saved, 32_768)
        await store.skipInFlightEffects()
    }

    func testADownloadFromALinkLeftUnfinishedCanBeResumedAfterARelaunch() async {
        let library = InMemoryModelLibrary()
        library.leaveUnfinished(pasted, received: 300)
        let store = await models(library)

        XCTAssertEqual(store.state.downloads[pasted.id], .paused(received: 300, total: 1_000))
        await store.send(.downloadTapped(pasted.id))
        await store.receive(\.downloaded)

        XCTAssertTrue(store.state.installed.contains(pasted.id))
        await store.skipInFlightEffects()
    }

    func testADownloadThatFinishesWithoutTheScreenShowsAsDownloaded() async {
        let library = InMemoryModelLibrary()
        library.show(.downloading(received: 10, total: 100), for: ModelCatalog.gemma4E2B.id)
        let store = await models(library)

        _ = try? await library.client.download(ModelCatalog.gemma4E2B)
        await store.receive(\.downloadsChanged)
        await store.receive(\.loaded)

        XCTAssertTrue(store.state.installed.contains(ModelCatalog.gemma4E2B.id))
        await store.skipInFlightEffects()
    }

    func testDownloadsAModelThatThenRunsAUse() async {
        let library = InMemoryModelLibrary()
        let store = await models(library)

        await store.send(.downloadTapped(ModelCatalog.gemma4E4B.id))
        await store.receive(\.downloaded)

        XCTAssertTrue(store.state.installed.contains(ModelCatalog.gemma4E4B.id))
        XCTAssertTrue(store.state.choices.map(\.id).contains(ModelCatalog.gemma4E4B.id))
        XCTAssertEqual(library.installed, [ModelCatalog.gemma4E4B.id])
        await store.skipInFlightEffects()
    }

    func testAFailedDownloadShowsWhyAndCanBeTriedAgain() async {
        let library = InMemoryModelLibrary()
        library.failNextDownload(of: ModelCatalog.gemma4E2B.id, with: .hub(.rateLimited))
        let store = await models(library)

        await store.send(.downloadTapped(ModelCatalog.gemma4E2B.id))
        await store.receive(\.downloaded)
        if store.state.downloads[ModelCatalog.gemma4E2B.id] == nil { await store.receive(\.downloadsChanged) }
        XCTAssertEqual(store.state.downloads[ModelCatalog.gemma4E2B.id], .failed(.hub(.rateLimited)))
        XCTAssertFalse(store.state.installed.contains(ModelCatalog.gemma4E2B.id))

        await store.send(.downloadTapped(ModelCatalog.gemma4E2B.id))
        await store.receive(\.downloaded)
        XCTAssertTrue(store.state.installed.contains(ModelCatalog.gemma4E2B.id))
        await store.skipInFlightEffects()
    }

    func testPausesADownload() async {
        let library = InMemoryModelLibrary()
        library.show(.downloading(received: 10, total: 100), for: ModelCatalog.gemma4E2B.id)
        let store = await models(library)
        XCTAssertEqual(store.state.downloads[ModelCatalog.gemma4E2B.id], .downloading(received: 10, total: 100))

        await store.send(.pauseTapped(ModelCatalog.gemma4E2B.id))
        await store.receive(\.downloadsChanged)

        XCTAssertEqual(store.state.downloads[ModelCatalog.gemma4E2B.id], .paused(received: 10, total: 100))
        await store.skipInFlightEffects()
    }

    func testRemovingAModelPutsItsUsesBackOnApple() async {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.gemma4E2B])
        await library.client.select(ModelCatalog.gemma4E2B.id, .assistant)
        let store = await models(library)

        await store.send(.removeTapped(ModelCatalog.gemma4E2B.id))
        XCTAssertNotNil(store.state.remove)
        await store.send(.remove(.presented(.confirmed(ModelCatalog.gemma4E2B.id))))
        await store.receive(\.removed)
        await store.receive(\.loaded)

        XCTAssertFalse(store.state.installed.contains(ModelCatalog.gemma4E2B.id))
        XCTAssertEqual(store.state.selected(for: .assistant), .apple)
        XCTAssertEqual(store.state.choices.map(\.id), [.apple])
        XCTAssertEqual(library.installed, [])
        await store.skipInFlightEffects()
    }

    func testAddsAModelFromALinkAndDownloadsIt() async {
        let library = InMemoryModelLibrary(links: [link: .success(pasted)])
        let store = await models(library)

        await store.send(.binding(.set(\.link, link)))
        XCTAssertTrue(store.state.canAdd)
        await store.send(.addTapped)
        await store.receive(\.resolved)
        await store.receive(\.downloaded)

        XCTAssertEqual(store.state.downloadable.last?.id, pasted.id)
        XCTAssertTrue(store.state.installed.contains(pasted.id))
        XCTAssertEqual(store.state.link, "")
        XCTAssertFalse(store.state.isAdding)
        await store.skipInFlightEffects()
    }

    func testALinkThatCantBeAddedSaysWhyAndKeepsTheLink() async {
        let library = InMemoryModelLibrary(links: [link: .failure(.gated(repo: "someone/tiny-litert-lm"))])
        let store = await models(library)

        await store.send(.binding(.set(\.link, link)))
        await store.send(.binding(.set(\.sha256, "abc")))
        await store.send(.addTapped)
        await store.receive(\.resolved)

        XCTAssertNotNil(store.state.addError)
        XCTAssertEqual(store.state.link, link)
        XCTAssertEqual(library.installed, [])

        await store.send(.binding(.set(\.link, link + "/")))
        XCTAssertNil(store.state.addError)
        await store.skipInFlightEffects()
    }

    func testAnEmptyLinkCantBeAdded() async {
        let store = await models(InMemoryModelLibrary())

        await store.send(.binding(.set(\.link, "   ")))
        XCTAssertFalse(store.state.canAdd)
        await store.send(.addTapped)
        XCTAssertFalse(store.state.isAdding)
        await store.skipInFlightEffects()
    }
}
