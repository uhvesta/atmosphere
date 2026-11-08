import ComposableArchitecture
import FlowKit
import LangReadrCore
import LangReadrModels
import LangReadrReading
import SwiftUI
import XCTest

/// Reading a story on the Mac, the library beside it. The Mac flow drives
/// the store, since it can't tap.
@MainActor
final class MacFlowSnapshotTests: XCTestCase {
    func testReading() async throws {
        let library = Sample.library()
        let flow = FlowRecorder("reading", LangReadrView(store: library))
        await flow.step("empty-library")
        let reader = try await Sample.importStory(into: library)
        await flow.step("reading")
        reader.send(.binding(.set(\.showsTranslation, true)))
        try await Sample.waitFor { reader.writing.isEmpty && reader.book.annotations.count == 2 }
        await flow.step("translated")
        let paragraph = try XCTUnwrap(reader.book.paragraphs.first)
        let words = reader.words[paragraph.id] ?? []
        let mercado = try XCTUnwrap(words.first { $0.text == "mercado" })
        reader.send(.wordTapped(paragraph: paragraph.id, mercado))
        try await Sample.waitFor { reader.destination?.word?.note != nil }
        await flow.step("word-card")
        reader.send(.destination(.dismiss))
        reader.send(.binding(.set(\.isHighlighting, true)))
        reader.send(.wordTapped(paragraph: paragraph.id, try XCTUnwrap(words.first)))
        reader.send(.wordTapped(paragraph: paragraph.id, mercado))
        await flow.step("passage-picked")
        reader.send(.askTapped)
        await flow.step("ask")
        reader.send(.destination(.presented(.chat(.binding(.set(\.input, "What does “vamos al” mean?"))))))
        reader.send(.destination(.presented(.chat(.sendTapped))))
        try await Sample.waitFor { reader.destination?.chat?.reply == nil }
        await flow.step("replied")
        flow.finish()
    }

    /// Settings › Models, then a passage chat's own model.
    func testPickingModels() async throws {
        let library = Sample.library()
        let flow = FlowRecorder("models", LangReadrView(store: library))
        library.send(.settingsTapped)
        try await Sample.waitFor { library.settings != nil }
        await flow.step("settings")
        library.send(.settings(.dismiss))
        let reader = try await Sample.importStory(into: library)
        let paragraph = try XCTUnwrap(reader.book.paragraphs.first)
        let words = reader.words[paragraph.id] ?? []
        reader.send(.binding(.set(\.isHighlighting, true)))
        reader.send(.wordTapped(paragraph: paragraph.id, try XCTUnwrap(words.first)))
        reader.send(.wordTapped(paragraph: paragraph.id, try XCTUnwrap(words.first { $0.text == "mercado" })))
        reader.send(.askTapped)
        reader.send(.destination(.presented(.chat(.modelTapped))))
        try await Sample.waitFor { reader.destination?.chat?.model?.models.isEmpty == false }
        await flow.step("chat-model")
        let model: (ChatModelFeature.Action) -> ReaderFeature.Action = {
            .destination(.presented(.chat(.model(.presented($0)))))
        }
        reader.send(model(.binding(.set(\.model, ModelCatalog.qwen35Small.id))))
        reader.send(model(.binding(.set(\.choice.thinking, .long))))
        await flow.step("chat-model-picked")
        flow.finish()
    }

    /// The Tune screen for the downloaded model, as Settings › Models opens
    /// it: its setups, its other format downloaded on tap, every setup
    /// measured side by side, then one picked for quick tasks and its
    /// window made smaller.
    func testTuning() async throws {
        let library = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small])
        let tune = Store(initialState: TuneFeature.State(model: ModelCatalog.qwen35Small)) {
            TuneFeature()
        } withDependencies: {
            $0.models = library.client
            $0.date.now = Date(timeIntervalSince1970: 1_793_700_000)
        }
        let state: () -> TuneFeature.State? = { tune.state }
        let flow = FlowRecorder("tuning", NavigationStack { TuneView(store: tune) })
        try await Sample.waitFor { state()?.candidates.isEmpty == false }
        await flow.step("opened")
        let other = try XCTUnwrap(state()?.otherFormats.first)
        let before = state()?.candidates.count ?? 0
        tune.send(.downloadTapped(other))
        try await Sample.waitFor { (state()?.candidates.count ?? 0) > before }
        await flow.step("downloaded")
        tune.send(.tuneTapped)
        try await Sample.waitFor { state()?.isRunning == false && state()?.results.isEmpty == false }
        await flow.step("tuned")
        let candidates = state()?.candidates ?? []
        let last = candidates.count - 1
        tune.send(.setupPicked(candidates[last].setup, for: .quick))
        tune.send(.editTapped(last))
        var options = candidates[last].options
        options.window = try XCTUnwrap(ModelCatalog.qwen35Small.windows.first)
        tune.send(.optionsChanged(options, at: last))
        await flow.step("edited")
        flow.finish()
    }
}
