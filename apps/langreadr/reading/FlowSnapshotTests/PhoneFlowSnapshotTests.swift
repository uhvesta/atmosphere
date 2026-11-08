import ChatFeatures
import ComposableArchitecture
import FlowKit
import LangReadrCore
import LangReadrModels
import LangReadrReading
import ModelsFeatures
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
        reader.send(.destination(.presented(.chat(.thread(.binding(.set(\.input, "What does “vamos al” mean?")))))))
        flow.tap("Send")
        try await Sample.waitFor { reader.destination?.chat?.thread.reply == nil }
        await flow.step("replied")
        flow.tap("Open in chat")
        await flow.step("chat")
        flow.finish()
    }

    /// Adding to a chat about a passage: a file of notes picked with +, the
    /// microphone writing what's said into the message until it's tapped
    /// again, then the reply. + opens the system's file picker, which the
    /// flow doesn't show.
    func testAddingToAChat() async throws {
        try await addingToAChat("adding", style: .light)
    }

    func testAddingToAChatInDarkMode() async throws {
        try await addingToAChat("adding-dark", style: .dark)
    }

    private func addingToAChat(_ name: String, style: UIUserInterfaceStyle, test: String = #function) async throws {
        let library = Sample.library()
        let flow = FlowRecorder(name, LangReadrView(store: library), style: style)
        let reader = try await Sample.importStory(into: library)
        reader.send(.binding(.set(\.isHighlighting, true)))
        await flow.step("highlighting", image: false, testName: test)
        flow.tap("Hola")
        flow.tap("mercado")
        flow.tap("Ask")
        await flow.step("ask", image: false, testName: test)
        flow.tap("Open in chat")
        await flow.step("chat", testName: test)
        let chat = { (action: ChatThreadFeature.Action) in
            _ = reader.send(.destination(.presented(.chat(.thread(action)))))
        }
        chat(.filesPicked(.success([URL(filePath: "/tmp/notes.md")])))
        try await Sample.waitFor { reader.destination?.chat?.thread.pending.isEmpty == false }
        await flow.step("file", testName: test)
        chat(.micTapped)
        try await Sample.waitFor { reader.destination?.chat?.thread.input.isEmpty == false }
        await flow.step("dictating", testName: test)
        chat(.micTapped)
        try await Sample.waitFor { reader.destination?.chat?.thread.dictatedAfter == nil }
        await flow.step("dictated", testName: test)
        chat(.sendTapped)
        try await Sample.waitFor { reader.destination?.chat?.thread.reply == nil }
        await flow.step("replied", testName: test)
        XCTAssertEqual(reader.destination?.chat?.chat.conversation.messages.first?.files.map(\.name), ["notes.md"])
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
        reader.send(.destination(.presented(.chat(.thread(.modelTapped)))))
        try await Sample.waitFor { reader.destination?.chat?.thread.modelSheet?.models.isEmpty == false }
        await flow.step("chat-model")
        let model: (ChatModelFeature.Action) -> ReaderFeature.Action = {
            .destination(.presented(.chat(.thread(.modelSheet(.presented($0))))))
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

    /// Settings › Models › Voices with a Spanish book on the shelf:
    /// downloading a voice model, picking one of its voices for Spanish,
    /// then hearing a word and its sentence from the word card.
    func testPickingAVoice() async throws {
        let said = LockIsolated<[String]>([])
        let library = Sample.library(
            speech: .picks(
                VoiceSelection(file: nil),
                speak: { text, language in
                    said.withValue { $0.append("\(language): \(text)") }
                }))
        let flow = FlowRecorder("voices", LangReadrView(store: library))
        let opened = try await Sample.importStory(into: library)
        let book = opened.book
        // The reader finds its words as it appears; closing it before then
        // would leave that with no reader to reach.
        try await Sample.waitFor { !opened.words.isEmpty }
        // Settings opens from the library, with the Spanish book on its shelf.
        library.send(.reader(.dismiss))
        try await Sample.waitFor { library.reader == nil }
        library.send(.settingsTapped)
        try await Sample.waitFor { library.settings != nil }
        await flow.step("settings")
        flow.tap("Models")
        try await Sample.waitFor { library.settings?.models.choices.count == 2 }
        // Voices end the screen.
        flow.scrollToEnd()
        await flow.step("voices")
        let voices: (VoicesFeature.Action) -> LibraryFeature.Action = {
            .settings(.presented(.models(.voices($0))))
        }
        let supertonic = VoiceCatalog.supertonic3.id
        library.send(voices(.downloadTapped(supertonic)))
        try await Sample.waitFor { library.settings?.models.voices.installed.contains(supertonic) == true }
        library.send(voices(.picked(.model(supertonic, voice: "F2"), language: "es")))
        flow.scrollToEnd()
        await flow.step("voice-picked")
        library.send(.settings(.dismiss))
        try await Sample.waitFor { library.settings == nil }
        library.send(.bookTapped(book.id))
        try await Sample.waitFor { library.reader != nil }
        let reader = try XCTUnwrap(library.scope(state: \.reader, action: \.reader.presented))
        await flow.step("reading")
        flow.tap("mercado")
        try await Sample.waitFor { reader.destination?.word?.note != nil }
        await flow.step("word-card")
        flow.tap("Say it")
        // The sentence's button sits below the half sheet's fold.
        reader.send(.destination(.presented(.word(.saySentenceTapped))))
        try await Sample.waitFor { said.value.count == 3 }
        let language = reader.book.language.rawValue
        XCTAssertEqual(
            said.value, ["mercado", "mercado", "¿Vamos al mercado hoy?"].map { "\(language): \($0)" })
        flow.finish()
    }
}
