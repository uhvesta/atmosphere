import Foundation
import LangReadrCore
import LangReadrServices

extension ReaderServices {
    /// Services for tests and previews: words split where letters stop,
    /// every text told as `language`, no transform, photos refused, and
    /// pages from `page`.
    public static func fake(
        language: Language = Language("es"),
        page: @escaping @Sendable (URL) async throws(PageError) -> String = { url throws(PageError) in
            throw .unreachable(host: url.host() ?? "", reason: "offline")
        }
    ) -> ReaderServices {
        ReaderServices(
            detectLanguage: { _ in language },
            words: { text, _ in
                let characters = Array(text)
                var words: [WordSpan] = []
                var start: Int?
                for (offset, character) in (characters + [" "]).enumerated() {
                    if character.isLetter {
                        if start == nil { start = offset }
                    } else if let first = start {
                        words.append(WordSpan(text: String(characters[first..<offset]), start: first, end: offset))
                        start = nil
                    }
                }
                return words
            },
            transliterate: { _ in nil },
            recognizeText: { _ throws(RecognitionError) in throw .notAnImage },
            fetchPage: page)
    }
}
