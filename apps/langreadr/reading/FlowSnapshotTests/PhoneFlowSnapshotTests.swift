import ComposableArchitecture
import FlowKit
import LangReadrCore
import LangReadrModels
import LangReadrReading
import SwiftUI
import XCTest

/// Reading a story on the iPhone: one reference per step, the
/// accessibility tree and an image. Words and buttons are tapped; switches
/// and closing sheets go through the store, since a flow's tap can't flip
/// a toolbar toggle or press a sheet's toolbar button.
@MainActor
final class PhoneFlowSnapshotTests: XCTestCase {
    /// Importing pasted text, translating it, looking a word up, and asking
    /// about a passage, then reading the chat on its own.
    func testReading() async throws {
        let library = Sample.library()
        let flow = FlowRecorder("reading", LangReadrView(store: library))
        await flow.step("empty-library")
        flow.tap("Text")
        await flow.step("paste-text")
        let reader = try await Sample.importStory(into: library)
        await flow.step("reading")
        reader.send(.binding(.set(\.showsTranslation, true)))
        try await Sample.waitFor { reader.writing.isEmpty && reader.book.annotations.count == 2 }
        await flow.step("translated")
        flow.tap("mercado")
        try await Sample.waitFor { reader.destination?.word?.note != nil }
        await flow.step("word-card")
        reader.send(.destination(.dismiss))
        reader.send(.binding(.set(\.isHighlighting, true)))
        await flow.step("highlighting")
        flow.tap("Hola")
        flow.tap("mercado")
        await flow.step("passage-picked")
        flow.tap("Ask")
        await flow.step("ask")
        reader.send(.destination(.presented(.chat(.binding(.set(\.input, "What does “vamos al” mean?"))))))
        flow.tap("Send")
        try await Sample.waitFor { reader.destination?.chat?.reply == nil }
        await flow.step("replied")
        flow.tap("Open in chat")
        await flow.step("chat")
        flow.finish()
    }

    /// Settings › Models, then picking a passage chat's own model, how
    /// long it thinks and its window. The toolbar's Settings and Model
    /// buttons go through the store.
    func testPickingModels() async throws {
        let library = Sample.library()
        let flow = FlowRecorder("models", LangReadrView(store: library))
        library.send(.settingsTapped)
        try await Sample.waitFor { library.settings != nil }
        await flow.step("settings")
        flow.tap("Models")
        try await Sample.waitFor { library.settings?.models.choices.count == 2 }
        await flow.step("models")
        library.send(.settings(.dismiss))
        let reader = try await Sample.importStory(into: library)
        reader.send(.binding(.set(\.isHighlighting, true)))
        await flow.step("highlighting")
        flow.tap("Hola")
        flow.tap("mercado")
        await flow.step("passage-picked")
        flow.tap("Ask")
        await flow.step("ask")
        flow.tap("Open in chat")
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

    /// Tuning the downloaded model from Settings › Models: its setups before
    /// anything runs, its other format downloaded on tap, every setup
    /// measured side by side, then one picked for quick tasks and its
    /// window made smaller.
    func testTuning() async throws {
        let library = Sample.library()
        let flow = FlowRecorder("tuning", LangReadrView(store: library))
        func send(_ action: TuneFeature.Action) {
            library.send(.settings(.presented(.models(.tune(.presented(action))))))
        }
        library.send(.settingsTapped)
        try await Sample.waitFor { library.settings != nil }
        await flow.step("settings")
        flow.tap("Models")
        try await Sample.waitFor { library.settings?.models.choices.count == 2 }
        await flow.step("models")
        library.send(.settings(.presented(.models(.tuneTapped(ModelCatalog.qwen35Small.id)))))
        try await Sample.waitFor { library.settings?.models.tune?.candidates.isEmpty == false }
        await flow.step("opened")
        let other = try XCTUnwrap(library.settings?.models.tune?.otherFormats.first)
        let before = library.settings?.models.tune?.candidates.count ?? 0
        send(.downloadTapped(other))
        try await Sample.waitFor { (library.settings?.models.tune?.candidates.count ?? 0) > before }
        await flow.step("downloaded")
        send(.tuneTapped)
        try await Sample.waitFor {
            library.settings?.models.tune?.isRunning == false && library.settings?.models.tune?.results.isEmpty == false
        }
        await flow.step("tuned")
        let candidates = library.settings?.models.tune?.candidates ?? []
        let last = candidates.count - 1
        send(.setupPicked(candidates[last].setup, for: .quick))
        send(.editTapped(last))
        var options = candidates[last].options
        options.window = try XCTUnwrap(ModelCatalog.qwen35Small.windows.first)
        send(.optionsChanged(options, at: last))
        flow.scrollToEnd()
        await flow.step("edited")
        flow.finish()
    }
}
