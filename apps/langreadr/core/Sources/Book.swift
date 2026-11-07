import ChatKit
import Foundation

/// A language, by its BCP 47 code, such as `ja` or `zh-Hans`.
public struct Language: RawRepresentable, Hashable, Codable, Sendable {
    public var rawValue: String

    public init(rawValue: String) { self.rawValue = rawValue }
    public init(_ code: String) { self.init(rawValue: code) }

    /// The language without its script or region: `zh` for `zh-Hans`.
    public var base: String {
        String(rawValue.split(whereSeparator: { $0 == "-" || $0 == "_" }).first ?? Substring(rawValue)).lowercased()
    }

    /// Its name as `locale` writes it, such as "Japanese".
    public func name(in locale: Locale = .current) -> String {
        locale.localizedString(forIdentifier: rawValue) ?? rawValue
    }
}

/// Where a book came from.
public enum BookSource: Hashable, Codable, Sendable {
    case text
    case photo
    case web(URL)
    case file(name: String)
}

/// A paragraph of the original, as written.
public struct Paragraph: Identifiable, Hashable, Codable, Sendable {
    public var id: UUID
    public var text: String

    public init(id: UUID = UUID(), text: String) {
        self.id = id
        self.text = text
    }
}

/// A part of a book under one heading. The first may have none.
public struct Chapter: Identifiable, Hashable, Codable, Sendable {
    public var id: UUID
    public var title: String?
    public var paragraphs: [Paragraph]

    public init(id: UUID = UUID(), title: String? = nil, paragraphs: [Paragraph]) {
        self.id = id
        self.title = title
        self.paragraphs = paragraphs
    }
}

/// What the app wrote under a paragraph. Each is written once and kept.
public struct Annotation: Hashable, Codable, Sendable {
    /// The paragraph in the reader's language.
    public var translation: String?
    /// The paragraph in Latin letters, for a language written in another
    /// script.
    public var transliteration: String?

    public init(translation: String? = nil, transliteration: String? = nil) {
        self.translation = translation
        self.transliteration = transliteration
    }
}

/// What a word means where it was read.
public struct WordNote: Hashable, Codable, Sendable {
    public var word: String
    public var transliteration: String?
    /// Its meanings, short, as a dictionary gives them.
    public var meaning: String
    /// What it means in the sentence it was read in.
    public var inSentence: String

    public init(word: String, transliteration: String?, meaning: String, inSentence: String) {
        self.word = word
        self.transliteration = transliteration
        self.meaning = meaning
        self.inSentence = inSentence
    }
}

/// Anything the person imported, to read in the language it's written in.
public struct Book: Identifiable, Hashable, Codable, Sendable {
    public var id: UUID
    public var title: String
    /// The language it's written in.
    public var language: Language
    public var source: BookSource
    public var importedAt: Date
    public var chapters: [Chapter]
    /// What the app wrote under each paragraph, by paragraph.
    public var annotations: [UUID: Annotation] = [:]
    /// The language `annotations`' translations are in. Changing it drops
    /// them, so they're written again.
    public var translatedInto: Language?
    /// The words looked up, by paragraph and word.
    public var words: [WordKey: WordNote] = [:]
    /// The conversations about passages of it.
    public var chats: [AnchoredChat] = []
    /// The paragraph the person read last.
    public var lastRead: UUID?

    public init(
        id: UUID = UUID(), title: String, language: Language, source: BookSource, importedAt: Date,
        chapters: [Chapter]
    ) {
        self.id = id
        self.title = title
        self.language = language
        self.source = source
        self.importedAt = importedAt
        self.chapters = chapters
    }

    public var paragraphs: [Paragraph] { chapters.flatMap(\.paragraphs) }

    public func paragraph(_ id: UUID) -> Paragraph? { paragraphs.first { $0.id == id } }

    /// How many words it has, counting each character of a script written
    /// without spaces as one.
    public var wordCount: Int { paragraphs.reduce(0) { $0 + Self.roughWords(in: $1.text) } }

    /// About how long it takes to read, at 200 words a minute; at least one.
    public var minutes: Int { max(1, (wordCount + 199) / 200) }

    /// Translations into `language` from now on. Those into another are
    /// dropped; transliterations stay, as they don't depend on it.
    public mutating func translate(into language: Language) {
        guard language != translatedInto else { return }
        translatedInto = language
        for id in annotations.keys { annotations[id]?.translation = nil }
        words = [:]
    }

    /// Keeps what the app wrote for `paragraph`, beside what it wrote before.
    public mutating func annotate(_ paragraph: UUID, translation: String? = nil, transliteration: String? = nil) {
        var annotation = annotations[paragraph] ?? Annotation()
        if let translation { annotation.translation = translation }
        if let transliteration { annotation.transliteration = transliteration }
        annotations[paragraph] = annotation
    }

    /// The chat about `passage`, made when there's none.
    public mutating func chat(about passage: Passage, at date: Date) -> AnchoredChat {
        if let chat = chats.first(where: { $0.passage == passage }) { return chat }
        let chat = AnchoredChat(passage: passage, createdAt: date)
        chats.append(chat)
        return chat
    }

    /// Keeps `chat` in place of the one with its id.
    public mutating func save(_ chat: AnchoredChat) {
        if let index = chats.firstIndex(where: { $0.id == chat.id }) {
            chats[index] = chat
        } else {
            chats.append(chat)
        }
    }

    /// The chats about passages in `paragraph`, in the order they were made.
    public func chats(in paragraph: UUID) -> [AnchoredChat] {
        chats.filter { $0.passage.paragraph == paragraph }
    }

    private static func roughWords(in text: String) -> Int {
        var count = 0
        var inWord = false
        for character in text {
            if character.isWhitespace || character.isPunctuation {
                inWord = false
            } else if Script.isWrittenWithoutSpaces(character) {
                count += 1
                inWord = false
            } else if !inWord {
                count += 1
                inWord = true
            }
        }
        return count
    }
}

/// A word looked up in a paragraph.
public struct WordKey: Hashable, Codable, Sendable, CustomStringConvertible, CodingKeyRepresentable {
    public var paragraph: UUID
    public var word: String

    public init(paragraph: UUID, word: String) {
        self.paragraph = paragraph
        self.word = word
    }

    public var description: String { "\(paragraph.uuidString) \(word)" }

    public var codingKey: any CodingKey { Key(stringValue: description) }

    public init?<T: CodingKey>(codingKey: T) {
        let parts = codingKey.stringValue.split(separator: " ", maxSplits: 1)
        guard parts.count == 2, let paragraph = UUID(uuidString: String(parts[0])) else { return nil }
        self.init(paragraph: paragraph, word: String(parts[1]))
    }

    private struct Key: CodingKey {
        var stringValue: String
        var intValue: Int? { nil }
        init(stringValue: String) { self.stringValue = stringValue }
        init?(intValue: Int) { nil }
    }
}
