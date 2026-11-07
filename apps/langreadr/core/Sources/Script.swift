/// What the reader needs to know of how a language is written.
public enum Script {
    /// Whether `character` is from a script written without spaces between
    /// words: Han, kana, Thai, Lao, Khmer and Burmese. A tokenizer has to
    /// find their words.
    public static func isWrittenWithoutSpaces(_ character: Character) -> Bool {
        character.unicodeScalars.contains { scalar in
            switch scalar.value {
            case 0x3040...0x30FF,  // Hiragana, Katakana
                0x3400...0x4DBF, 0x4E00...0x9FFF, 0xF900...0xFAFF, 0x20000...0x2FA1F,  // Han
                0x0E00...0x0EFF,  // Thai, Lao
                0x1000...0x109F,  // Myanmar
                0x1780...0x17FF:  // Khmer
                true
            default: false
            }
        }
    }

    /// Whether `text`'s letters are all Latin, so it needs no
    /// transliteration.
    public static func isLatin(_ text: some StringProtocol) -> Bool {
        text.unicodeScalars.allSatisfy { scalar in
            guard scalar.properties.isAlphabetic else { return true }
            switch scalar.value {
            case 0x0041...0x024F, 0x1E00...0x1EFF, 0x2C60...0x2C7F, 0xA720...0xA7FF, 0xFF21...0xFF5A: return true
            default: return false
            }
        }
    }

    /// How to write `language` in Latin letters.
    public static func transliteration(for language: Language) -> Transliteration {
        switch language.base {
        // Its kanji have readings a transform can't know: it would read
        // them as Mandarin.
        case "ja": .model
        default: .transform
        }
    }
}

/// How a language is written in Latin letters.
public enum Transliteration: Hashable, Sendable {
    /// ICU's to-Latin transform: exact and instant.
    case transform
    /// The model, which knows readings that depend on the word.
    case model
}
