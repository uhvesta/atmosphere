import CaptureKit
import ComposableArchitecture
import Foundation
import InferenceKit
import JSONKit
import LangReadrCore
import LangReadrModels
import LangReadrReading
import LangReadrServicesFake

/// A short story in Spanish, and a model that knows it: what each
/// paragraph says in English, what its words mean, and the reply about it.
enum Sample {
    static let story = """
        # El mercado

        Hola amigo. ¿Vamos al mercado hoy?

        El pan está caliente y huele muy bien.
        """

    private static let translations = [
        "Hola amigo. ¿Vamos al mercado hoy?": "Hello, friend. Shall we go to the market today?",
        "El pan está caliente y huele muy bien.": "The bread is warm and smells very good.",
    ]

    private static let words: [String: JSONValue] = [
        "mercado": [
            "reading": "mer-KAH-doh", "meaning": "market; marketplace",
            "inSentence": "The place they're going today.",
        ],
        "pan": ["reading": "pahn", "meaning": "bread; loaf", "inSentence": "What's warm and smells good."],
    ]

    /// In Markdown, as a model writes it.
    static let reply = """
        **Vamos** is “we go” of *ir*, and as a question it means “shall we go?”.

        - **Al** is *a* + *el*: “to the”.
        - *Hoy* means “today”.
        """

    /// Answers each turn by what it asks.
    private static let answer: ScriptedRuntime.Step = { turn, partial throws(ModelRuntimeError) in
        let value: JSONValue
        if turn.prompt.hasPrefix("Word: ") {
            let word = turn.prompt.dropFirst(6).prefix { $0 != "\n" }
            value = words[String(word)] ?? ["meaning": "a word of the story", "inSentence": ""]
        } else if turn.prompt.hasPrefix("Reader: ") {
            value = .string(reply)
        } else {
            value = .string(translations[turn.prompt] ?? turn.prompt)
        }
        partial(value)
        return value
    }

    static var runtime: ScriptedRuntime { ScriptedRuntime(steps: Array(repeating: answer, count: 100)) }

    /// The files a flow can pick, by name: notes on the story's words.
    static var files: [String: PickedFile] {
        guard
            let notes = try? FileText(
                name: "notes.md", text: "Vamos: we go. Al: to the. Mercado: market.", limit: 8_000)
        else { return [:] }
        return ["notes.md": .text(notes)]
    }

    /// A library with nothing in it yet, reading in English, with one
    /// model downloaded beside Apple's.
    @MainActor
    static func library(speech: SpeechClient = .inMemory) -> StoreOf<LibraryFeature> {
        Store(initialState: LibraryFeature.State()) {
            LibraryFeature()
        } withDependencies: {
            $0.shelf = .inMemory()
            $0.tutor = Tutor(runtime: runtime, reader: Language("en"))
            $0.readerServices = .fake()
            $0.speech = speech
            $0.models = InMemoryModelLibrary(installed: [ModelCatalog.qwen35Small]).client
            $0.date.now = Date(timeIntervalSince1970: 1_793_700_000)
            $0.uuid = .incrementing
            $0.dictation = .scripted()
            $0.fileReader = .scripted(files)
        }
    }

    /// Imports the story into `library`, and waits for it to open.
    @MainActor
    static func importStory(into library: StoreOf<LibraryFeature>) async throws -> StoreOf<ReaderFeature> {
        if library.importing == nil { library.send(.importTapped(.text)) }
        library.send(.importing(.presented(.binding(.set(\.text, story)))))
        library.send(.importing(.presented(.importTapped)))
        try await waitFor { library.reader != nil }
        guard let reader = library.scope(state: \.reader, action: \.reader.presented) else { throw Unopened() }
        return reader
    }

    /// Waits for what the model is writing to land, failing after five
    /// seconds.
    @MainActor
    static func waitFor(_ done: @MainActor () -> Bool) async throws {
        for _ in 0..<500 {
            if done() { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        throw Unsettled()
    }

    struct Unopened: Error {}
    struct Unsettled: Error {}
}
