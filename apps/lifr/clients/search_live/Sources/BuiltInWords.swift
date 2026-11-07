import Foundation
import NaturalLanguage

/// Related words from the word embeddings built into the system: for each
/// word searched for that names a thing, an action or a quality, the words
/// closest to it, such as "grocery", "shopping" and "store" for
/// "groceries". No model, nothing to download, and quick.
enum BuiltInWords {
    /// At most this many words in all.
    static let limit = 12
    /// At most this many for each word searched for.
    private static let perWord = 8
    /// Words further than this from the one searched for are too loosely
    /// related to help, such as "legislation" for "bills".
    private static let maximumDistance = 1.0

    /// Words close to `query`'s, the closest to each word first, without
    /// its own; empty in a language the system has no word lists for.
    static func related(to query: String) -> [String] {
        let words = meaningful(in: query)
        guard !words.isEmpty, let embedding = embedding(for: query) else { return [] }
        let searched = Set(words)
        let lists = words.map { word in
            embedding.neighbors(for: word, maximumCount: perWord)
                .filter { $0.1 <= maximumDistance }
                .map { $0.0.lowercased() }
                .filter { $0.allSatisfy(\.isLetter) && !searched.contains($0) }
        }
        // A few from each word searched for, rather than all from the first.
        var seen: Set<String> = []
        var related: [String] = []
        for rank in 0..<perWord {
            for list in lists where rank < list.count && seen.insert(list[rank]).inserted {
                related.append(list[rank])
            }
        }
        return Array(related.prefix(limit))
    }

    /// The words in `query` that name things, actions or qualities, in
    /// lowercase: not "the", "for" or "my".
    private static func meaningful(in query: String) -> [String] {
        let tagger = NLTagger(tagSchemes: [.lexicalClass])
        tagger.string = query
        var words: [String] = []
        tagger.enumerateTags(
            in: query.startIndex..<query.endIndex, unit: .word, scheme: .lexicalClass,
            options: [.omitWhitespace, .omitPunctuation, .omitOther]
        ) { tag, range in
            let word = query[range].lowercased()
            if let tag, [.noun, .verb, .adjective].contains(tag), word.count > 2, !words.contains(word) {
                words.append(word)
            }
            return true
        }
        return words
    }

    /// The word embedding for `query`'s language, or the person's, or
    /// English, whichever the system has first.
    private static func embedding(for query: String) -> NLEmbedding? {
        let recognizer = NLLanguageRecognizer()
        recognizer.processString(query)
        let preferred = Locale.current.language.languageCode.map { NLLanguage(rawValue: $0.identifier) }
        let languages = [recognizer.dominantLanguage, preferred, .english].compactMap(\.self)
        return languages.lazy.compactMap { NLEmbedding.wordEmbedding(for: $0) }.first
    }
}
