import Foundation

/// Text made ready for a voice model to read: written the way models of
/// its language saw text in training, then cut into pieces short enough
/// to say at once.
public enum SpokenText {
    /// The most characters in a piece. Models that read a whole piece
    /// before they speak lose their way in longer ones.
    public static let pieceLength = 150

    /// `text` as a model trained on `language`, a BCP 47 code such as
    /// "fa", reads it. A language with no rules of its own only has its
    /// spacing tidied.
    public static func normalized(_ text: String, language: String) -> String {
        switch Locale(identifier: language).language.languageCode?.identifier ?? language {
        case "fa": PersianText.normalized(text)
        default: collapsedSpaces(text.precomposedStringWithCanonicalMapping)
        }
    }

    /// `text`, normalized for `language`, as pieces of about `length`
    /// characters: whole sentences where they fit, joined while they fit,
    /// and a longer sentence cut at a comma, else a space.
    public static func pieces(of text: String, language: String, length: Int = pieceLength) -> [String] {
        let text = Array(normalized(text, language: language).unicodeScalars)
        guard text.count > length else { return text.isEmpty ? [] : [String(scalars: text)] }
        var pieces: [[Unicode.Scalar]] = []
        for var sentence in sentences(of: text) {
            while sentence.count > length {
                var cut = lastIndex(in: sentence, before: length) { $0 == "\u{060C}" || $0 == "," } ?? -1
                if cut < length / 3 { cut = lastIndex(in: sentence, before: length) { $0 == " " } ?? -1 }
                if cut <= 0 { cut = length }
                pieces.append(trimmed(sentence[..<min(cut + 1, sentence.count)]))
                sentence = trimmed(sentence[min(cut + 1, sentence.count)...])
            }
            guard !sentence.isEmpty else { continue }
            if let last = pieces.last, last.count + sentence.count + 1 <= length {
                pieces[pieces.count - 1] = last + [" "] + sentence
            } else {
                pieces.append(sentence)
            }
        }
        return pieces.filter { !$0.isEmpty }.map { String(scalars: $0) }
    }

    /// `text` split after each sentence's closing mark and the spaces
    /// that follow it.
    private static func sentences(of text: [Unicode.Scalar]) -> [[Unicode.Scalar]] {
        let endings: Set<Unicode.Scalar> = [".", "!", "?", "\u{061F}", "\u{061B}"]
        var sentences: [[Unicode.Scalar]] = []
        var current: [Unicode.Scalar] = []
        var index = 0
        while index < text.count {
            let scalar = text[index]
            if isSpace(scalar), let last = current.last, endings.contains(last) {
                while index < text.count, isSpace(text[index]) { index += 1 }
                sentences.append(current)
                current = []
                continue
            }
            current.append(scalar)
            index += 1
        }
        sentences.append(current)
        return sentences
    }

    private static func lastIndex(
        in text: [Unicode.Scalar], before end: Int, where matches: (Unicode.Scalar) -> Bool
    ) -> Int? {
        text[..<min(end, text.count)].lastIndex(where: matches)
    }

    private static func trimmed(_ text: ArraySlice<Unicode.Scalar>) -> [Unicode.Scalar] {
        Array(text.drop(while: isSpace).reversed().drop(while: isSpace).reversed())
    }

    static func isSpace(_ scalar: Unicode.Scalar) -> Bool {
        scalar.properties.isWhitespace
    }

    /// `text` with each run of spaces made one, and none at either end.
    static func collapsedSpaces(_ text: String) -> String {
        text.unicodeScalars.split(whereSeparator: isSpace).map { String(String.UnicodeScalarView($0)) }
            .joined(separator: " ")
    }
}

extension String {
    init(scalars: [Unicode.Scalar]) {
        self.init(String.UnicodeScalarView(scalars))
    }
}
