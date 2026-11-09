import Foundation

/// Supertonic's text: cleaned up the way Supertone's own reference code
/// does, cut into pieces short enough for the Core ML export's fixed 128
/// characters, and turned into the ids its unicode table gives.
enum SupertonicText {
    /// The characters the text encoder and duration predictor read at
    /// once; shorter text is padded and masked.
    static let width = 128
    /// The most characters of text in a piece, leaving room for the
    /// language's tags, such as "<en>" and "</en>", and a full stop.
    static let pieceLength = width - 12

    /// `text` as pieces of at most `pieceLength` characters: by
    /// paragraph, then by sentence, then by word, and a word longer than
    /// a piece is cut.
    static func pieces(of text: String, length: Int = pieceLength) -> [String] {
        var pieces: [String] = []
        for paragraph in text.components(separatedBy: "\n\n") {
            var current = ""
            for sentence in sentences(of: paragraph) {
                for part in fitting(sentence, length: length) {
                    if current.isEmpty {
                        current = part
                    } else if current.count + 1 + part.count <= length {
                        current += " " + part
                    } else {
                        pieces.append(current)
                        current = part
                    }
                }
            }
            if !current.isEmpty { pieces.append(current) }
        }
        return pieces
    }

    /// `text`'s ids, a piece at a time, each piece at most `width` ids
    /// once it's cleaned up and tagged. A piece whose characters come
    /// apart into more, as Korean syllables do into their letters, is cut
    /// again until it fits.
    static func encoded(_ text: String, language: String, indexer: [Int32]) -> [[Int32]] {
        pieces(of: text).flatMap { encoded(piece: $0, language: language, indexer: indexer) }
    }

    private static func encoded(piece: String, language: String, indexer: [Int32]) -> [[Int32]] {
        let ids = ids(of: prepared(piece, language: language), indexer: indexer)
        guard ids.count > width else { return [ids] }
        let half = max(piece.count / 2, 1)
        guard half < piece.count else { return [Array(ids.prefix(width))] }
        return pieces(of: piece, length: half).flatMap { encoded(piece: $0, language: language, indexer: indexer) }
    }

    /// `text` cleaned up and in its language's tags, as the model reads it.
    static func prepared(_ text: String, language: String) -> String {
        var text = text.decomposedStringWithCompatibilityMapping
        text = String(String.UnicodeScalarView(text.unicodeScalars.filter { !isEmoji($0) }))
        for (from, to) in replacements { text = text.replacingOccurrences(of: from, with: to) }
        for symbol in ["♥", "☆", "♡", "©", "\\"] { text = text.replacingOccurrences(of: symbol, with: "") }
        for (from, to) in [("@", " at "), ("e.g.,", "for example, "), ("i.e.,", "that is, ")] {
            text = text.replacingOccurrences(of: from, with: to)
        }
        for mark in [",", ".", "!", "?", ";", ":", "'"] {
            text = text.replacingOccurrences(of: " " + mark, with: mark)
        }
        while text.contains("\"\"") { text = text.replacingOccurrences(of: "\"\"", with: "\"") }
        while text.contains("''") { text = text.replacingOccurrences(of: "''", with: "'") }
        text = text.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        if let last = text.last, !endings.contains(last) {
            text += "."
        }
        return "<\(language)>\(text)</\(language)>"
    }

    /// The ids of `text`'s characters in `indexer`, a table from each
    /// UTF-16 code unit to its id, -1 for one the model doesn't know;
    /// those it doesn't know are left out.
    static func ids(of text: String, indexer: [Int32]) -> [Int32] {
        text.utf16.compactMap { unit in
            let index = Int(unit)
            guard index < indexer.count, indexer[index] >= 0 else { return nil }
            return indexer[index]
        }
    }

    private static let endings = Set(".!?;:,'\")]}…。」』】〉》›»")

    private static let replacements: [(String, String)] = [
        ("–", "-"), ("‑", "-"), ("—", "-"), ("_", " "), ("\u{201C}", "\""), ("\u{201D}", "\""),
        ("\u{2018}", "'"), ("\u{2019}", "'"), ("´", "'"), ("`", "'"), ("[", " "), ("]", " "), ("|", " "),
        ("/", " "), ("#", " "), ("→", " "), ("←", " "),
    ]

    private static func isEmoji(_ scalar: Unicode.Scalar) -> Bool {
        switch scalar.value {
        case 0x1F300...0x1FAFF, 0x2600...0x27BF, 0x1F1E6...0x1F1FF: true
        default: false
        }
    }

    /// `text`'s sentences, each with its closing mark.
    private static func sentences(of text: String) -> [String] {
        var sentences: [String] = []
        text.enumerateSubstrings(in: text.startIndex..., options: .bySentences) { sentence, _, _, _ in
            if let sentence = sentence?.trimmingCharacters(in: .whitespacesAndNewlines), !sentence.isEmpty {
                sentences.append(sentence)
            }
        }
        return sentences
    }

    /// `sentence` in parts of at most `length`, by word.
    private static func fitting(_ sentence: String, length: Int) -> [String] {
        guard sentence.count > length else { return [sentence] }
        var parts: [String] = []
        var current = ""
        for word in sentence.split(separator: " ").map(String.init) {
            var word = word
            while word.count > length {
                if !current.isEmpty {
                    parts.append(current)
                    current = ""
                }
                parts.append(String(word.prefix(length)))
                word = String(word.dropFirst(length))
            }
            if current.isEmpty {
                current = word
            } else if current.count + 1 + word.count <= length {
                current += " " + word
            } else {
                parts.append(current)
                current = word
            }
        }
        if !current.isEmpty { parts.append(current) }
        return parts
    }
}
