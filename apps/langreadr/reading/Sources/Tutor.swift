import ChatKit
import Foundation
import InferenceKit
import JSONKit
import LangReadrCore
import LangReadrModels

/// What the reader asks the model: translations, readings, what words mean
/// and replies about passages. Each answer is held to the shape it needs.
public struct Tutor: Sendable {
    /// What each use runs on, or what a chat picked in place of it.
    package let runtime: @Sendable (ModelUse, ModelChoice) -> any ModelRuntime
    /// The person's own language, which the model writes in.
    public let reader: Language

    package init(runtime: @escaping @Sendable (ModelUse, ModelChoice) -> any ModelRuntime, reader: Language) {
        self.runtime = runtime
        self.reader = reader
    }

    /// A tutor that runs everything on `runtime`.
    public init(runtime: any ModelRuntime, reader: Language) {
        self.init(runtime: { _, _ in runtime }, reader: reader)
    }

    /// What translating, readings and words run on.
    private var reading: any ModelRuntime { runtime(.reading, .defaults) }

    /// `text`, written in `language`, in the reader's language.
    public func translate(_ text: String, from language: Language) async throws(ModelRuntimeError) -> String {
        let turn = ModelTurn(
            instructions: """
                You translate \(name(language)) into \(name(reader)). Keep the meaning, tone and names. \
                Write only the translation, with no notes.
                """,
            prompt: text)
        return try await Self.text(reading.respond(to: turn) { _ in })
    }

    /// `text` in Latin letters, read as a native speaker of `language`
    /// would: Hepburn for Japanese.
    public func romanize(_ text: String, from language: Language) async throws(ModelRuntimeError) -> String {
        let turn = ModelTurn(
            instructions: """
                You write \(name(language)) text in Latin letters, as a native speaker reads it aloud\
                \(language.base == "ja" ? Self.hepburn : ""). \
                Keep the punctuation. Write only the romanization.
                """,
            prompt: text)
        return try await Self.text(reading.respond(to: turn) { _ in })
    }

    /// What `word` means, and what it means in `sentence`.
    public func lookUp(_ word: String, in sentence: String, language: Language) async throws(ModelRuntimeError)
        -> WordNote
    {
        let turn = ModelTurn(
            instructions: """
                You're a \(name(language)) dictionary for someone who speaks \(name(reader)). For the word \
                asked, give its reading in Latin letters, its usual meanings in a few words, and in one \
                sentence what it means where it's used. Write the meanings and the sentence in \(name(reader)).
                """,
            prompt: "Word: \(word)\nUsed in: \(sentence)",
            schema: .object(
                name: "Word",
                [
                    ModelProperty("reading", "The word in Latin letters"),
                    ModelProperty("meaning", "Its usual meanings, a few words each, separated by semicolons"),
                    ModelProperty("inSentence", "What it means in this sentence, in one sentence"),
                ]))
        let answer = try await reading.respond(to: turn) { _ in }
        guard case .object(let fields) = answer, case .string(let meaning)? = fields["meaning"] else {
            throw .failed("the model's answer had no meaning for \(word)")
        }
        let reading = Self.string(fields["reading"])
        let inSentence = Self.string(fields["inSentence"])
        return WordNote(
            word: word, transliteration: reading.flatMap { $0.isEmpty || $0 == word ? nil : $0 }, meaning: meaning,
            inSentence: inSentence ?? "")
    }

    /// The reply to the last message of `chat`, about a passage of `book`,
    /// and how much of the model's window it took. `partial` gets the reply
    /// as it's written.
    public func reply(
        in chat: AnchoredChat, about book: ChatBook, partial: @escaping @Sendable (String) -> Void
    ) async throws(ModelRuntimeError) -> TutorReply {
        let choice = ModelChoice(chat.model)
        let model = runtime(.chat, choice)
        let budget = model.budget.room * 3
        let context = book.paragraph ?? chat.passage.text
        let transcript = chat.conversation.messages.newest(fitting: max(400, budget - context.count - 600))
            .map { "\($0.role == .person ? "Reader" : "You"): \($0.text)" }
            .joined(separator: "\n\n")
        let instructions = """
            You help someone who speaks \(name(reader)) read \(name(book.language)). They're reading \
            “\(book.title)” and asking about this passage: “\(chat.passage.text)”, from the paragraph: \
            “\(context)”. Answer in \(name(reader)), briefly and plainly. Explain grammar and words \
            where they ask, and quote the \(name(book.language)) you explain.
            """
        var turn = ModelTurn(instructions: instructions, prompt: transcript)
        if let tokens = choice.thinkingLevel.tokens {
            // The chat shows only the reply, so what it thinks isn't kept.
            turn.thinking = { _ in }
            turn.thinkingTokens = tokens
        }
        let text = try await Self.text(
            model.respond(to: turn) { value in
                if case .string(let text) = value { partial(text) }
            })
        // About three bytes a token, as the transcript was cut to fit.
        let used = (instructions.utf8.count + transcript.utf8.count + text.utf8.count) / 3
        return TutorReply(text: text, context: ContextUse(used: used, window: model.budget.window))
    }

    private static let hepburn = ", in Hepburn romanization, with each kanji read as it is in its word"

    private func name(_ language: Language) -> String { language.name(in: Locale(identifier: "en")) }

    private static func string(_ value: JSONValue?) -> String? {
        guard case .string(let text)? = value else { return nil }
        return text
    }

    private static func text(_ answer: JSONValue) throws(ModelRuntimeError) -> String {
        guard case .string(let text) = answer else { throw .failed("the model answered with a value, not text") }
        return text.trimmingCharacters(in: .whitespacesAndNewlines)
    }
}

/// A reply in a chat about a passage.
public struct TutorReply: Hashable, Sendable {
    public var text: String
    /// About how much of the model's window the chat took, with the reply.
    public var context: ContextUse

    public init(text: String, context: ContextUse) {
        self.text = text
        self.context = context
    }
}

/// What a chat about a passage knows of its book.
public struct ChatBook: Hashable, Sendable {
    public var title: String
    public var language: Language
    /// The paragraph the passage is in.
    public var paragraph: String?

    public init(title: String, language: Language, paragraph: String?) {
        self.title = title
        self.language = language
        self.paragraph = paragraph
    }

    public init(_ book: Book, chat: AnchoredChat) {
        self.init(
            title: book.title, language: book.language, paragraph: book.paragraph(chat.passage.paragraph)?.text)
    }
}
